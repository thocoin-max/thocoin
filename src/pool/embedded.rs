// In-process pool SERVER used by the wallet GUI "Create Pool" screen.
// Wraps the existing Pool (which already does PPLNS + payouts) with a TCP
// listener, live stats, and start/stop control. Because it shares the wallet's
// ChainState and Mempool, blocks the pool wins are applied directly to the
// same chain the wallet is on — no separate process or node RPC needed.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use parking_lot::{Mutex, RwLock};

use crate::core::chain::ChainState;
use crate::core::mempool::Mempool;
use crate::wallet::Wallet;
use crate::pool::{Pool, ClientMsg, ServerMsg};

// Per-connection reported hashrate, keyed by the unique extranonce we assign.
type Registry = Arc<RwLock<HashMap<u32, u64>>>;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

#[derive(Clone)]
pub struct PoolConfig {
    pub name: String,
    pub fee_percent: f64,
    pub min_payout: u64,     // in base units (COIN)
    pub port: u16,
}

pub struct PoolServerStats {
    pub running: AtomicBool,
    pub online_miners: AtomicU64,
    pub blocks_found: AtomicU64,
    pub pool_hashrate: AtomicU64,   // sum of miner-reported hashrates
    pub total_shares: AtomicU64,
    pub last_status: RwLock<String>,
    pub pool_url: RwLock<String>,
    pub name: RwLock<String>,
}

impl Default for PoolServerStats {
    fn default() -> Self {
        Self {
            running: AtomicBool::new(false),
            online_miners: AtomicU64::new(0),
            blocks_found: AtomicU64::new(0),
            pool_hashrate: AtomicU64::new(0),
            total_shares: AtomicU64::new(0),
            last_status: RwLock::new("Stopped".into()),
            pool_url: RwLock::new(String::new()),
            name: RwLock::new(String::new()),
        }
    }
}

pub struct EmbeddedPool {
    pub stats: Arc<PoolServerStats>,
    running: Arc<AtomicBool>,
    chain: Arc<ChainState>,
    mempool: Mempool,
    wallet: Arc<Wallet>,
}

impl EmbeddedPool {
    pub fn new(chain: Arc<ChainState>, mempool: Mempool, wallet: Arc<Wallet>) -> Self {
        Self {
            stats: Arc::new(PoolServerStats::default()),
            running: Arc::new(AtomicBool::new(false)),
            chain, mempool, wallet,
        }
    }

    pub fn is_running(&self) -> bool { self.running.load(Ordering::SeqCst) }

    pub fn start(&self, cfg: PoolConfig) -> Result<(), String> {
        if self.running.load(Ordering::SeqCst) {
            return Err("Pool already running".into());
        }
        let bind = format!("0.0.0.0:{}", cfg.port);
        let listener = TcpListener::bind(&bind)
            .map_err(|e| format!("Cannot bind port {}: {}", cfg.port, e))?;
        listener.set_nonblocking(true).ok();

        self.running.store(true, Ordering::SeqCst);
        self.stats.running.store(true, Ordering::SeqCst);
        self.stats.blocks_found.store(0, Ordering::Relaxed);
        self.stats.total_shares.store(0, Ordering::Relaxed);
        *self.stats.name.write() = cfg.name.clone();
        *self.stats.pool_url.write() = format!("0.0.0.0:{}", cfg.port);
        *self.stats.last_status.write() = "Running".into();

        let pool = Arc::new(Pool::new(self.chain.clone(), self.mempool.clone(), self.wallet.clone()));
        let running = self.running.clone();
        let stats = self.stats.clone();
        let extranonce_seq = Arc::new(AtomicU32::new(1));
        let registry: Registry = Arc::new(RwLock::new(HashMap::new()));

        // Payout background loop.
        {
            let pool_bg = pool.clone();
            let running_bg = running.clone();
            std::thread::spawn(move || {
                while running_bg.load(Ordering::SeqCst) {
                    pool_bg.process_mature_payouts();
                    for _ in 0..100 {
                        if !running_bg.load(Ordering::SeqCst) { break; }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            });
        }

        // Pool-wide stats from miner-reported hashrates.
        {
            let stats_hr = stats.clone();
            let running_hr = running.clone();
            let registry_hr = registry.clone();
            std::thread::spawn(move || {
                while running_hr.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_secs(2));
                    let (miners, total) = {
                        let r = registry_hr.read();
                        (r.len() as u64, r.values().sum::<u64>())
                    };
                    stats_hr.online_miners.store(miners, Ordering::Relaxed);
                    stats_hr.pool_hashrate.store(total, Ordering::Relaxed);
                }
                stats_hr.pool_hashrate.store(0, Ordering::Relaxed);
                stats_hr.online_miners.store(0, Ordering::Relaxed);
            });
        }

        // Accept loop.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if !running.load(Ordering::SeqCst) { break; }
                match stream {
                    Ok(stream) => {
                        let pool = pool.clone();
                        let ext = extranonce_seq.fetch_add(1, Ordering::SeqCst);
                        let running_c = running.clone();
                        let stats_c = stats.clone();
                        let registry_c = registry.clone();
                        std::thread::spawn(move || {
                            let _ = handle_client(stream, pool, ext, &running_c, &stats_c, registry_c.clone());
                            registry_c.write().remove(&ext);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    Err(_) => break,
                }
            }
            stats.running.store(false, Ordering::SeqCst);
            *stats.last_status.write() = "Stopped".into();
        });

        Ok(())
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.stats.running.store(false, Ordering::SeqCst);
        *self.stats.last_status.write() = "Stopped".into();
    }
}

fn send(stream: &mut TcpStream, msg: &ServerMsg) -> std::io::Result<()> {
    let mut s = serde_json::to_string(msg).unwrap();
    s.push('\n');
    stream.write_all(s.as_bytes())
}

fn handle_client(
    stream: TcpStream,
    pool: Arc<Pool>,
    extranonce: u32,
    running: &Arc<AtomicBool>,
    stats: &Arc<PoolServerStats>,
    registry: Registry,
) -> anyhow::Result<()> {
    // The listener is non-blocking, and accepted sockets inherit that on Windows.
    // A non-blocking read makes reader.lines() return WouldBlock immediately and
    // the loop would close the miner. Force blocking mode for this connection.
    stream.set_nonblocking(false).ok();
    stream.set_read_timeout(Some(Duration::from_secs(120))).ok();
    let writer = Arc::new(Mutex::new(stream.try_clone()?));
    let reader = BufReader::new(stream);
    let address = Arc::new(RwLock::new(String::new()));
    let template = Arc::new(RwLock::new(pool.build_template()));
    let alive = Arc::new(AtomicBool::new(true));

    // Job-refresh thread: roll the timestamp so the miner always has a fresh
    // nonce space (a single fixed 2^32 space rarely contains a valid block nonce
    // at mainnet difficulty). Rebuild the template only when the tip advances.
    {
        let pool = pool.clone();
        let writer = writer.clone();
        let template = template.clone();
        let address = address.clone();
        let alive = alive.clone();
        let running = running.clone();
        std::thread::spawn(move || {
            while alive.load(Ordering::SeqCst) && running.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(4));
                if !alive.load(Ordering::SeqCst) || !running.load(Ordering::SeqCst) { break; }
                if address.read().is_empty() { continue; }
                if template.read().header.prev_hash != *pool.chain.tip.read() {
                    *template.write() = pool.build_template();
                }
                let tmpl = template.read().clone();
                let job = pool.make_job_at(&tmpl, extranonce, now_secs());
                let mut w = writer.lock();
                if send(&mut w, &job).is_err() { break; }
            }
        });
    }

    for line in reader.lines() {
        if !running.load(Ordering::SeqCst) { break; }
        let line = match line { Ok(l) => l, Err(_) => break };
        if line.trim().is_empty() { continue; }
        let msg: ClientMsg = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                let mut w = writer.lock();
                let _ = send(&mut w, &ServerMsg::Error { message: format!("bad json: {e}") });
                continue;
            }
        };
        match msg {
            ClientMsg::Hashrate { hashrate } => {
                registry.write().insert(extranonce, hashrate);
            }
            ClientMsg::Subscribe { address: addr } => {
                *address.write() = addr.clone();
                registry.write().entry(extranonce).or_insert(0);
                let tmpl = pool.build_template();
                *template.write() = tmpl.clone();
                let job = pool.make_job_at(&tmpl, extranonce, now_secs());
                let mut w = writer.lock();
                send(&mut w, &job)?;
            }
            ClientMsg::Share { nonce, timestamp, extranonce: _ } => {
                let addr = address.read().clone();
                if addr.is_empty() {
                    let mut w = writer.lock();
                    send(&mut w, &ServerMsg::Rejected { reason: "not subscribed".into() })?;
                    continue;
                }
                let mut tmpl = template.read().clone();
                if tmpl.header.prev_hash != *pool.chain.tip.read() {
                    let fresh = pool.build_template();
                    *template.write() = fresh.clone();
                    let job = pool.make_job_at(&fresh, extranonce, now_secs());
                    let mut w = writer.lock();
                    send(&mut w, &job)?;
                    drop(w);
                    tmpl = fresh;
                }
                let (valid, is_block) = pool.check_share(&tmpl, nonce, timestamp, &addr);
                if !valid {
                    let mut w = writer.lock();
                    send(&mut w, &ServerMsg::Rejected { reason: "share below target".into() })?;
                } else {
                    stats.total_shares.fetch_add(1, Ordering::Relaxed);
                    let (n, _) = pool.stats();
                    {
                        let mut w = writer.lock();
                        send(&mut w, &ServerMsg::Accepted { shares: n as u64 })?;
                    }
                    if is_block {
                        match pool.on_block_won(&tmpl, nonce, timestamp) {
                            Ok(reward) => {
                                stats.blocks_found.fetch_add(1, Ordering::Relaxed);
                                let h = *pool.chain.height.read();
                                let mut w = writer.lock();
                                send(&mut w, &ServerMsg::BlockFound { height: h, reward })?;
                            }
                            Err(_) => {}
                        }
                        let tmpl2 = pool.build_template();
                        *template.write() = tmpl2.clone();
                        let job = pool.make_job_at(&tmpl2, extranonce, now_secs());
                        let mut w = writer.lock();
                        send(&mut w, &job)?;
                    }
                }
            }
        }
    }
    alive.store(false, Ordering::SeqCst);
    registry.write().remove(&extranonce);
    Ok(())
}