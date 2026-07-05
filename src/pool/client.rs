// In-process pool mining client used by the wallet GUI "Join Pool" screen.
// Connects to a ThoCoin pool server over the existing JSON line protocol,
// mines shares on the GPU (gpu build) or CPU worker threads (cpu build),
// auto-reconnects, and exposes live stats.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use parking_lot::RwLock;

use crate::pool::{ServerMsg, hex32, header_from_job};
use crate::core::hash::hash_meets_target;

pub const DEFAULT_POOL: &str = "thocoin.org";
const DEFAULT_PORT: u16 = 23333;

/// Accepts "host", "host:port", "ip", "ip:port" and always returns "host:port".
/// Empty input falls back to the public default pool so the app works out of the box.
fn normalize_host(host: &str) -> String {
    let h = host.trim();
    if h.is_empty() {
        return DEFAULT_POOL.to_string();
    }
    if let Some((_, port)) = h.rsplit_once(':') {
        if port.parse::<u16>().is_ok() {
            return h.to_string();
        }
    }
    format!("{}:{}", h, DEFAULT_PORT)
}

#[derive(Clone)]
struct JobData {
    prev: [u8; 32],
    merkle: [u8; 32],
    bits: u32,
    timestamp: u64,
    share_bits: u32,
    extranonce: u32,
    height: u64,
}

pub struct PoolClientStats {
    pub connected: AtomicBool,
    pub hashrate: AtomicU64,        // worker hashes/sec (this miner)
    pub accepted: AtomicU64,
    pub rejected: AtomicU64,
    pub height: AtomicU64,          // current job height
    pub pool_hashrate: AtomicU64,   // total pool hashrate (all miners)
    pub pool_miners: AtomicU64,     // number of miners connected to the pool
    pub pool_blocks: AtomicU64,     // total blocks the pool has found
    pub last_status: RwLock<String>,
    pub worker_id: RwLock<String>,
    pub pool_url: RwLock<String>,
}

impl Default for PoolClientStats {
    fn default() -> Self {
        Self {
            connected: AtomicBool::new(false),
            hashrate: AtomicU64::new(0),
            accepted: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            height: AtomicU64::new(0),
            pool_hashrate: AtomicU64::new(0),
            pool_miners: AtomicU64::new(0),
            pool_blocks: AtomicU64::new(0),
            last_status: RwLock::new("Idle".into()),
            worker_id: RwLock::new(String::new()),
            pool_url: RwLock::new(String::new()),
        }
    }
}

pub struct PoolClient {
    pub stats: Arc<PoolClientStats>,
    running: Arc<AtomicBool>,
    threads: usize,
}

impl PoolClient {
    pub fn new() -> Self {
        Self {
            stats: Arc::new(PoolClientStats::default()),
            running: Arc::new(AtomicBool::new(false)),
            threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2),
        }
    }

    pub fn is_running(&self) -> bool { self.running.load(Ordering::SeqCst) }

    fn make_worker_id(address: &str) -> String {
        use rand::RngCore;
        let mut b = [0u8; 3];
        rand::rngs::OsRng.fill_bytes(&mut b);
        let short = if address.len() >= 6 { &address[address.len()-6..] } else { address };
        format!("w-{}-{}", short, hex::encode(b))
    }

    pub fn start(&self, host: String, address: String) {
        if self.running.load(Ordering::SeqCst) { return; }
        let host = normalize_host(&host);
        self.running.store(true, Ordering::SeqCst);

        let worker_id = Self::make_worker_id(&address);
        *self.stats.worker_id.write() = worker_id.clone();
        *self.stats.pool_url.write() = host.clone();
        self.stats.accepted.store(0, Ordering::Relaxed);
        self.stats.rejected.store(0, Ordering::Relaxed);

        let running = self.running.clone();
        let stats = self.stats.clone();
        let threads = self.threads;

        std::thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                *stats.last_status.write() = format!("Connecting to {host}...");
                match TcpStream::connect(&host) {
                    Ok(stream) => {
                        stream.set_nodelay(true).ok();
                        Self::run_connection(stream, &host, &address, threads, &running, &stats);
                    }
                    Err(e) => {
                        *stats.last_status.write() = format!("Connect failed: {e}. Retrying in 5s...");
                    }
                }
                stats.connected.store(false, Ordering::SeqCst);
                stats.hashrate.store(0, Ordering::Relaxed);
                if !running.load(Ordering::SeqCst) { break; }
                for _ in 0..50 {
                    if !running.load(Ordering::SeqCst) { break; }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            *stats.last_status.write() = "Stopped".into();
        });
    }

    fn run_connection(
        stream: TcpStream,
        _host: &str,
        address: &str,
        threads: usize,
        running: &Arc<AtomicBool>,
        stats: &Arc<PoolClientStats>,
    ) {
        let writer = match stream.try_clone() { Ok(w) => w, Err(_) => return };
        let reader = BufReader::new(stream);

        let sub = format!("{{\"type\":\"subscribe\",\"address\":\"{}\"}}\n", address);
        {
            let mut w = match writer.try_clone() { Ok(w) => w, Err(_) => return };
            if w.write_all(sub.as_bytes()).is_err() { return; }
        }

        stats.connected.store(true, Ordering::SeqCst);
        *stats.last_status.write() = "Connected".into();

        let job: Arc<RwLock<Option<JobData>>> = Arc::new(RwLock::new(None));
        let hashes = Arc::new(AtomicU64::new(0));
        let conn_alive = Arc::new(AtomicBool::new(true));

        let mut handles = Vec::new();
        Self::spawn_workers(&mut handles, &job, running, &conn_alive, &hashes, &writer, threads, stats);

        // Hashrate sampler — also reports our hashrate up to the pool so the
        // server can aggregate total pool hashrate and miner count.
        {
            let hashes = hashes.clone();
            let stats = stats.clone();
            let running = running.clone();
            let conn_alive = conn_alive.clone();
            let mut rw = match writer.try_clone() { Ok(w) => Some(w), Err(_) => None };
            std::thread::spawn(move || {
                let mut last = 0u64;
                while running.load(Ordering::SeqCst) && conn_alive.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_secs(2));
                    let cur = hashes.load(Ordering::Relaxed);
                    let hr = cur.wrapping_sub(last) / 2;
                    stats.hashrate.store(hr, Ordering::Relaxed);
                    last = cur;
                    if let Some(w) = rw.as_mut() {
                        let msg = format!("{{\"type\":\"hashrate\",\"hashrate\":{}}}\n", hr);
                        if w.write_all(msg.as_bytes()).is_err() { rw = None; }
                    }
                }
                stats.hashrate.store(0, Ordering::Relaxed);
            });
        }

        // Read server messages on this thread.
        for line in reader.lines() {
            if !running.load(Ordering::SeqCst) { break; }
            let line = match line { Ok(l) => l, Err(_) => break };
            if line.trim().is_empty() { continue; }
            let msg: ServerMsg = match serde_json::from_str(&line) { Ok(m) => m, Err(_) => continue };
            match msg {
                ServerMsg::Job { prev, merkle, bits, timestamp, share_bits, extranonce, height, .. } => {
                    let (Some(p), Some(m)) = (hex32(&prev), hex32(&merkle)) else { continue };
                    *job.write() = Some(JobData { prev: p, merkle: m, bits, timestamp, share_bits, extranonce, height });
                    stats.height.store(height, Ordering::Relaxed);
                }
                ServerMsg::Accepted { .. } => { stats.accepted.fetch_add(1, Ordering::Relaxed); }
                ServerMsg::Rejected { .. } => { stats.rejected.fetch_add(1, Ordering::Relaxed); }
                ServerMsg::BlockFound { height, .. } => {
                    *stats.last_status.write() = format!("Pool found block #{height}!");
                }
                ServerMsg::Error { message } => {
                    *stats.last_status.write() = format!("Pool: {message}");
                }
                ServerMsg::PoolStats { miners, pool_hashrate, blocks_found } => {
                    stats.pool_miners.store(miners, Ordering::Relaxed);
                    stats.pool_hashrate.store(pool_hashrate, Ordering::Relaxed);
                    stats.pool_blocks.store(blocks_found, Ordering::Relaxed);
                }
            }
        }

        conn_alive.store(false, Ordering::SeqCst);
        for h in handles { let _ = h.join(); }
    }

    // ---- worker selection: GPU build mines on the GPU, CPU build on threads ----

    #[cfg(feature = "gpu")]
    fn spawn_workers(
        handles: &mut Vec<std::thread::JoinHandle<()>>,
        job: &Arc<RwLock<Option<JobData>>>,
        running: &Arc<AtomicBool>,
        conn_alive: &Arc<AtomicBool>,
        hashes: &Arc<AtomicU64>,
        writer: &TcpStream,
        threads: usize,
        stats: &Arc<PoolClientStats>,
    ) {
        if crate::miner::gpu::GpuSearcher::new().is_some() {
            let job = job.clone();
            let running = running.clone();
            let conn_alive = conn_alive.clone();
            let hashes = hashes.clone();
            let stats = stats.clone();
            let w = match writer.try_clone() { Ok(w) => w, Err(_) => return };
            handles.push(std::thread::spawn(move || {
                Self::gpu_worker(job, running, conn_alive, hashes, w, stats);
            }));
        } else {
            Self::spawn_cpu_workers(handles, job, running, conn_alive, hashes, writer, threads);
        }
    }

    #[cfg(not(feature = "gpu"))]
    fn spawn_workers(
        handles: &mut Vec<std::thread::JoinHandle<()>>,
        job: &Arc<RwLock<Option<JobData>>>,
        running: &Arc<AtomicBool>,
        conn_alive: &Arc<AtomicBool>,
        hashes: &Arc<AtomicU64>,
        writer: &TcpStream,
        threads: usize,
        _stats: &Arc<PoolClientStats>,
    ) {
        Self::spawn_cpu_workers(handles, job, running, conn_alive, hashes, writer, threads);
    }

    #[cfg(feature = "gpu")]
    fn gpu_worker(
        job: Arc<RwLock<Option<JobData>>>,
        running: Arc<AtomicBool>,
        conn_alive: Arc<AtomicBool>,
        hashes: Arc<AtomicU64>,
        mut w: TcpStream,
        stats: Arc<PoolClientStats>,
    ) {
        use crate::miner::gpu::{GpuSearcher, header_words_19, target_words_8};
        let mut searcher = match GpuSearcher::new() { Some(s) => s, None => return };
        *stats.last_status.write() = format!("Connected (GPU: {})", searcher.device_name());

        let batch: u32 = 1 << 20;
        let mut loaded: Option<([u8; 32], [u8; 32], u64)> = None;
        let mut nonce_base: u32 = 0;

        loop {
            if !running.load(Ordering::SeqCst) || !conn_alive.load(Ordering::SeqCst) { return; }
            let jd = { job.read().clone() };
            let Some(jd) = jd else { std::thread::sleep(Duration::from_millis(50)); continue; };

            let id = (jd.prev, jd.merkle, jd.timestamp);
            if loaded.as_ref() != Some(&id) {
                let hdr = header_from_job(jd.prev, jd.merkle, jd.bits, jd.timestamp, 0);
                let h80 = hdr.serialize_80();
                let header_words = header_words_19(&h80);
                let target = crate::core::hash::bits_to_target(jd.share_bits);
                let target_be = target_words_8(&target);
                if !searcher.load_job(&header_words, &target_be) {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                loaded = Some(id);
                nonce_base = 0;
            }

            match searcher.search_batch(nonce_base, batch) {
                Ok(found) => {
                    hashes.fetch_add(batch as u64, Ordering::Relaxed);
                    if let Some(nonce) = found {
                        let msg = format!(
                            "{{\"type\":\"share\",\"nonce\":{},\"timestamp\":{},\"extranonce\":{}}}\n",
                            nonce, jd.timestamp, jd.extranonce);
                        if w.write_all(msg.as_bytes()).is_err() { return; }
                    }
                }
                Err(_) => { std::thread::sleep(Duration::from_millis(100)); }
            }

            nonce_base = nonce_base.wrapping_add(batch);
        }
    }

    fn spawn_cpu_workers(
        handles: &mut Vec<std::thread::JoinHandle<()>>,
        job: &Arc<RwLock<Option<JobData>>>,
        running: &Arc<AtomicBool>,
        conn_alive: &Arc<AtomicBool>,
        hashes: &Arc<AtomicU64>,
        writer: &TcpStream,
        threads: usize,
    ) {
        for tid in 0..threads {
            let job = job.clone();
            let running = running.clone();
            let conn_alive = conn_alive.clone();
            let hashes = hashes.clone();
            let mut w = match writer.try_clone() { Ok(w) => w, Err(_) => continue };
            handles.push(std::thread::spawn(move || {
                let mut nonce: u32 = (tid as u32).wrapping_mul(0x1000_0000);
                loop {
                    if !running.load(Ordering::SeqCst) || !conn_alive.load(Ordering::SeqCst) { return; }
                    let jd = { job.read().clone() };
                    let Some(jd) = jd else { std::thread::sleep(Duration::from_millis(50)); continue; };
                    for _ in 0..200_000 {
                        let hdr = header_from_job(jd.prev, jd.merkle, jd.bits, jd.timestamp, nonce);
                        let h = hdr.hash();
                        hashes.fetch_add(1, Ordering::Relaxed);
                        if hash_meets_target(&h, jd.share_bits) {
                            let msg = format!(
                                "{{\"type\":\"share\",\"nonce\":{},\"timestamp\":{},\"extranonce\":{}}}\n",
                                nonce, jd.timestamp, jd.extranonce);
                            let _ = w.write_all(msg.as_bytes());
                        }
                        nonce = nonce.wrapping_add(1);
                    }
                }
            }));
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.stats.connected.store(false, Ordering::SeqCst);
        *self.stats.last_status.write() = "Stopped".into();
    }
}