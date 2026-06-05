use std::path::PathBuf;
use std::sync::Arc;
use thocoin::core::chain::ChainState;
use thocoin::core::mempool::Mempool;
use thocoin::wallet::Wallet;
use thocoin::miner::Miner;
use thocoin::net::P2P;
use thocoin::rpc::start_rpc;

fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("THOCOIN_DATA") {
        let dir = PathBuf::from(d);
        let _ = std::fs::create_dir_all(&dir);
        return dir;
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("ThoCoin");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let data = data_dir();
    let chain = Arc::new(ChainState::open(data.join("chain").to_str().unwrap())?);
    let wallet = Arc::new(Wallet::load_or_create(data.join("wallet.key").to_str().unwrap())?);
    let mempool = Mempool::new();

    println!("Data dir: {}", data.display());
    println!("Address: {}", wallet.address());
    println!("Height: {}, Supply: {}", *chain.height.read(), *chain.supply.read());

    let p2p = Arc::new(P2P::new(chain.clone(), mempool.clone()));
    let p2p_run = p2p.clone();
    tokio::spawn(async move { let _ = p2p_run.run().await; });

    let _rpc = start_rpc(chain.clone(), wallet.clone());

    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let miner = Arc::new(Miner::new(chain.clone(), mempool.clone(), wallet.clone(), threads));

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--mine") { miner.start(); }

    println!("ThoCoin daemon  |  RPC 127.0.0.1:22222  |  P2P 0.0.0.0:22221");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
