use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use thocoin::pool::{ServerMsg, hex32, header_from_job};
use thocoin::core::hash::hash_meets_target;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Dùng: thocoin-pool-miner <host:port> <địa_chỉ_THO> [threads]");
        std::process::exit(1);
    }
    let host = args[1].clone();
    let address = args[2].clone();
    let threads: usize = args.get(3).and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2));

    println!("Kết nối pool {host} ...");
    let stream = TcpStream::connect(&host)?;
    stream.set_nodelay(true).ok();
    let mut writer = stream.try_clone()?;
    let reader = BufReader::new(stream);

    let sub = format!("{{\"type\":\"subscribe\",\"address\":\"{}\"}}\n", address);
    writer.write_all(sub.as_bytes())?;
    println!("Đã subscribe với địa chỉ {address}, {threads} luồng");

    let job: Arc<parking_lot::RwLock<Option<JobData>>> = Arc::new(parking_lot::RwLock::new(None));
    let running = Arc::new(AtomicBool::new(true));
    let hashes = Arc::new(AtomicU64::new(0));
    let found = Arc::new(AtomicU32::new(0));

    for tid in 0..threads {
        let job = job.clone();
        let running = running.clone();
        let hashes = hashes.clone();
        let mut w = writer.try_clone()?;
        std::thread::spawn(move || {
            let mut nonce: u32 = (tid as u32).wrapping_mul(0x10000000);
            loop {
                if !running.load(Ordering::Relaxed) { return; }
                let jd = { job.read().clone() };
                let Some(jd) = jd else {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    continue;
                };
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
        });
    }

    {
        let hashes = hashes.clone();
        std::thread::spawn(move || {
            let mut last = 0u64;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                let cur = hashes.load(Ordering::Relaxed);
                let hr = (cur - last) / 5;
                last = cur;
                println!("Hashrate: {} H/s", hr);
            }
        });
    }

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() { continue; }
        let msg: ServerMsg = match serde_json::from_str(&line) { Ok(m)=>m, Err(_)=>continue };
        match msg {
            ServerMsg::Job { prev, merkle, bits, timestamp, share_bits, extranonce, height, .. } => {
                let (Some(p), Some(m)) = (hex32(&prev), hex32(&merkle)) else { continue };
                *job.write() = Some(JobData { prev: p, merkle: m, bits, timestamp, share_bits, extranonce });
                println!("Job mới: height {height}");
            }
            ServerMsg::Accepted { shares } => {
                println!("✓ Share được chấp nhận (tổng share trong cửa sổ: {shares})");
            }
            ServerMsg::Rejected { reason } => {
                println!("✗ Share bị từ chối: {reason}");
            }
            ServerMsg::BlockFound { height, reward } => {
                let n = found.fetch_add(1, Ordering::Relaxed) + 1;
                println!("★★ POOL THẮNG BLOCK #{height}! reward {} (lần {n}) — sẽ chia theo share", reward);
            }
            ServerMsg::Error { message } => println!("Lỗi từ pool: {message}"),
        }
    }
    running.store(false, Ordering::Relaxed);
    Ok(())
}

#[derive(Clone)]
struct JobData {
    prev: [u8; 32],
    merkle: [u8; 32],
    bits: u32,
    timestamp: u64,
    share_bits: u32,
    extranonce: u32,
}
