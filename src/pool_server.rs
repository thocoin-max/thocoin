use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use parking_lot::{Mutex, RwLock};

use thocoin::core::chain::ChainState;
use thocoin::core::mempool::Mempool;
use thocoin::wallet::Wallet;
use thocoin::net::P2P;
use thocoin::pool::{Pool, ClientMsg, ServerMsg};

// Per-connection reported hashrate; keyed by the unique extranonce we assign.
type Registry = Arc<RwLock<HashMap<u32, u64>>>;

fn data_dir() -> String {
    if let Ok(base) = std::env::var("APPDATA") {
        format!("{}\\ThoCoinPool", base)
    } else {
        "./pooldata".to_string()
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn main() -> anyhow::Result<()> {
    let bind = std::env::args().nth(1).unwrap_or_else(|| "0.0.0.0:23333".to_string());
    let dir = data_dir();
    std::fs::create_dir_all(&dir).ok();

    let chain = Arc::new(ChainState::open(&format!("{}\\chain", dir))?);
    let wallet = Arc::new(Wallet::load_or_create(&format!("{}\\pool_wallet.key", dir))?);
    let mempool = Mempool::new();
    let pool = Arc::new(Pool::new(chain.clone(), mempool.clone(), wallet.clone()));

    println!("==============================================");
    println!(" ThoCoin Mining Pool");
    println!(" Pool wallet : {}", wallet.address());
    println!(" Listening   : {}", bind);
    println!(" Height      : {}", *chain.height.read());
    println!("==============================================");

    let rt = tokio::runtime::Runtime::new()?;
    let p2p = Arc::new(P2P::new(chain.clone(), mempool.clone()));
    {
        let p2p = p2p.clone();
        rt.spawn(async move { let _ = p2p.run().await; });
    }
    std::mem::forget(rt);
    println!(" P2P         : syncing with mainnet (seeds + THOCOIN_PEERS)...");

    // Wait until we've caught up to the best height any peer advertised before we
    // start serving miners. Otherwise the pool would mine its own fork from
    // genesis (easy difficulty) and never adopt the real chain, so rewards never
    // reach wallets. If no peer is found in time, fall through (standalone mode).
    {
        let mut waited = 0;
        loop {
            let best = p2p.best_height.load(std::sync::atomic::Ordering::SeqCst);
            let cur = *chain.height.read();
            if best == 0 {
                // no peer yet
                if waited >= 60 {
                    println!(" P2P         : no peer found, running standalone.");
                    break;
                }
            } else if cur + 1 >= best {
                println!(" P2P         : synced to height {} (peer best {}).", cur, best);
                break;
            } else {
                println!(" P2P         : syncing {}/{} ...", cur, best);
            }
            waited += 1;
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    // Live height reporter. The boot banner above prints height ONCE; without this
    // the console line stays frozen while the chain advances, making it look out
    // of sync with the job heights miners receive. This prints the real tip
    // height on an interval so the two always agree.
    {
        let chain = chain.clone();
        std::thread::spawn(move || {
            let mut last = u64::MAX;
            loop {
                let h = *chain.height.read();
                if h != last {
                    println!(" Height      : {} (job height = {})", h, h + 1);
                    last = h;
                }
                std::thread::sleep(Duration::from_secs(5));
            }
        });
    }

    {
        let pool_bg = pool.clone();
        std::thread::spawn(move || {
            loop {
                pool_bg.process_mature_payouts();
                std::thread::sleep(Duration::from_secs(10));
            }
        });
    }

    let registry: Registry = Arc::new(RwLock::new(HashMap::new()));
    let listener = TcpListener::bind(&bind)?;
    let extranonce_seq = Arc::new(AtomicU32::new(1));

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let pool = pool.clone();
        let registry = registry.clone();
        let ext = extranonce_seq.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            if let Err(e) = handle_client(stream, pool, ext, registry.clone()) {
                eprintln!("client error: {e}");
            }
            registry.write().remove(&ext);
        });
    }
    Ok(())
}

fn send(stream: &mut TcpStream, msg: &ServerMsg) -> std::io::Result<()> {
    let mut s = serde_json::to_string(msg).unwrap();
    s.push('\n');
    stream.write_all(s.as_bytes())
}

fn handle_client(stream: TcpStream, pool: Arc<Pool>, extranonce: u32, registry: Registry)
    -> anyhow::Result<()>
{
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    let writer = Arc::new(Mutex::new(stream.try_clone()?));
    let reader = BufReader::new(stream);
    let address = Arc::new(RwLock::new(String::new()));
    let template = Arc::new(RwLock::new(pool.build_template()));
    let alive = Arc::new(AtomicBool::new(true));

    // Job-refresh + pool-stats broadcast thread.
    {
        let pool = pool.clone();
        let writer = writer.clone();
        let template = template.clone();
        let address = address.clone();
        let alive = alive.clone();
        let registry = registry.clone();
        std::thread::spawn(move || {
            while alive.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(4));
                if !alive.load(Ordering::SeqCst) { break; }
                if address.read().is_empty() { continue; }

                if template.read().header.prev_hash != *pool.chain.tip.read() {
                    *template.write() = pool.build_template();
                }
                let tmpl = template.read().clone();
                let job = pool.make_job_at(&tmpl, extranonce, now_secs());

                let (miners, total_hr) = {
                    let r = registry.read();
                    (r.len() as u64, r.values().sum::<u64>())
                };

                let mut w = writer.lock();
                if send(&mut w, &job).is_err() { break; }
                if send(&mut w, &ServerMsg::PoolStats { miners, pool_hashrate: total_hr }).is_err() { break; }
            }
        });
    }

    for line in reader.lines() {
        let line = line?;
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
                println!("[+] miner {peer} addr={addr}");
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
                    let (n, _) = pool.stats();
                    {
                        let mut w = writer.lock();
                        send(&mut w, &ServerMsg::Accepted { shares: n as u64 })?;
                    }
                    if is_block {
                        match pool.on_block_won(&tmpl, nonce, timestamp) {
                            Ok(reward) => {
                                let h = tmpl.height;
                                println!("[*] BLOCK! height={h} reward={reward} by {addr}");
                                let mut w = writer.lock();
                                send(&mut w, &ServerMsg::BlockFound { height: h, reward })?;
                            }
                            Err(e) => eprintln!("apply block failed: {e}"),
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
    println!("[-] miner {peer} disconnected");
    Ok(())
}