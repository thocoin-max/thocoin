use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use parking_lot::RwLock;

use thocoin::core::chain::ChainState;
use thocoin::core::mempool::Mempool;
use thocoin::wallet::Wallet;
use thocoin::pool::{Pool, ClientMsg, ServerMsg};

fn data_dir() -> String {

    if let Ok(base) = std::env::var("APPDATA") {
        format!("{}\\ThoCoinPool", base)
    } else {
        "./pooldata".to_string()
    }
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

    {
        let pool_bg = pool.clone();
        std::thread::spawn(move || {
            loop {
                pool_bg.process_mature_payouts();
                std::thread::sleep(std::time::Duration::from_secs(10));
            }
        });
    }

    let listener = TcpListener::bind(&bind)?;
    let extranonce_seq = Arc::new(AtomicU32::new(1));

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let pool = pool.clone();
        let ext = extranonce_seq.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            if let Err(e) = handle_client(stream, pool, ext) {
                eprintln!("client error: {e}");
            }
        });
    }
    Ok(())
}

fn send(stream: &mut TcpStream, msg: &ServerMsg) -> std::io::Result<()> {
    let mut s = serde_json::to_string(msg).unwrap();
    s.push('\n');
    stream.write_all(s.as_bytes())
}

fn handle_client(stream: TcpStream, pool: Arc<Pool>, extranonce: u32) -> anyhow::Result<()> {
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    let mut writer = stream.try_clone()?;
    let reader = BufReader::new(stream);
    let address = Arc::new(RwLock::new(String::new()));

    let template = Arc::new(RwLock::new(pool.build_template()));

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() { continue; }
        let msg: ClientMsg = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => { let _ = send(&mut writer, &ServerMsg::Error { message: format!("bad json: {e}") }); continue; }
        };
        match msg {
            ClientMsg::Subscribe { address: addr } => {
                *address.write() = addr.clone();
                println!("[+] miner {peer} addr={addr}");
                let tmpl = pool.build_template();
                *template.write() = tmpl.clone();
                let job = pool.make_job(&tmpl, extranonce);
                send(&mut writer, &job)?;
            }
            ClientMsg::Share { nonce, timestamp, extranonce: _ } => {
                let addr = address.read().clone();
                if addr.is_empty() {
                    send(&mut writer, &ServerMsg::Rejected { reason: "not subscribed".into() })?;
                    continue;
                }
                let mut tmpl = template.read().clone();

                if tmpl.header.prev_hash != *pool.chain.tip.read() {
                    let fresh = pool.build_template();
                    *template.write() = fresh.clone();
                    let job = pool.make_job(&fresh, extranonce);
                    send(&mut writer, &job)?;
                    tmpl = fresh;
                }

                let (valid, is_block) = pool.check_share(&tmpl, nonce, timestamp, &addr);
                if !valid {
                    send(&mut writer, &ServerMsg::Rejected { reason: "share below target".into() })?;
                } else {
                    let (n, _) = pool.stats();
                    send(&mut writer, &ServerMsg::Accepted { shares: n as u64 })?;
                    if is_block {
                        match pool.on_block_won(&tmpl, nonce, timestamp) {
                            Ok(reward) => {
                                let h = *pool.chain.height.read();
                                println!("[*] BLOCK! height={h} reward={reward} by {addr}");
                                send(&mut writer, &ServerMsg::BlockFound { height: h, reward })?;
                            }
                            Err(e) => eprintln!("apply block failed: {e}"),
                        }

                        let tmpl2 = pool.build_template();
                        *template.write() = tmpl2.clone();
                        let job = pool.make_job(&tmpl2, extranonce);
                        send(&mut writer, &job)?;
                    }
                }
            }
        }
    }
    println!("[-] miner {peer} disconnected");
    Ok(())
}
