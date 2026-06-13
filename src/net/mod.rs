use std::collections::HashSet;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use tokio::net::{TcpListener, TcpStream};
use tokio::net::tcp::OwnedReadHalf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use serde::{Serialize, Deserialize};
use parking_lot::RwLock;
use crate::core::chain::ChainState;
use crate::core::block::Block;
use crate::core::tx::Transaction;
use crate::core::hash::Hash;
use crate::core::mempool::Mempool;
use crate::core::consensus::{NETWORK_MAGIC, P2P_PORT, SEED_NODES};

const MAX_PEERS: usize = 64;
const MAX_MSG_LEN: usize = 2_000_000;
const MAX_MSGS_PER_CONN: u64 = 1_000_000;
const MAX_SYNC_BATCH: u64 = 500;
const MAX_INV: usize = 1_000;
const BAN_THRESHOLD: i32 = 100;
const MAX_ADDR_SHARE: usize = 100;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Msg {
    Version { magic: u32, height: u64 },
    Verack,
    GetBlocks { from_height: u64 },
    Inv { blocks: Vec<Hash>, txs: Vec<Hash> },
    GetData { blocks: Vec<Hash>, txs: Vec<Hash> },
    BlockMsg(Block),
    TxMsg(Transaction),
    GetAddr,
    Addr(Vec<String>),
    Ping,
    Pong,
}

pub struct P2P {
    pub chain: Arc<ChainState>,
    pub mempool: Mempool,
    peers: Arc<RwLock<HashMap<u64, mpsc::UnboundedSender<Msg>>>>,
    next_id: Arc<AtomicU64>,
    pub conn_count: Arc<AtomicUsize>,
    known_addrs: Arc<RwLock<HashSet<String>>>,
    outbound: Arc<RwLock<HashSet<String>>>,
    banned: Arc<RwLock<HashSet<String>>>,
}

impl P2P {
    pub fn new(chain: Arc<ChainState>, mempool: Mempool) -> Self {
        P2P {
            chain, mempool,
            peers: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            conn_count: Arc::new(AtomicUsize::new(0)),
            known_addrs: Arc::new(RwLock::new(HashSet::new())),
            outbound: Arc::new(RwLock::new(HashSet::new())),
            banned: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    async fn send_msg<W: AsyncWriteExt + Unpin>(w: &mut W, msg: &Msg) -> anyhow::Result<()> {
        let bytes = bincode::serialize(msg)?;
        if bytes.len() > MAX_MSG_LEN { anyhow::bail!("message too large"); }
        w.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
        w.write_all(&bytes).await?;
        Ok(())
    }

    async fn read_msg(r: &mut OwnedReadHalf) -> Option<Msg> {
        let mut len_buf = [0u8; 4];
        if r.read_exact(&mut len_buf).await.is_err() { return None; }
        let len = u32::from_le_bytes(len_buf) as usize;
        if len == 0 || len > MAX_MSG_LEN { return None; }
        let mut buf = vec![0u8; len];
        if r.read_exact(&mut buf).await.is_err() { return None; }
        bincode::deserialize::<Msg>(&buf).ok()
    }

    fn broadcast_inv(&self, except: u64, blocks: Vec<Hash>, txs: Vec<Hash>) {
        let msg = Msg::Inv { blocks, txs };
        let peers = self.peers.read();
        for (id, tx) in peers.iter() {
            if *id != except {
                let _ = tx.send(msg.clone());
            }
        }
    }

    pub async fn run(self: Arc<Self>) -> anyhow::Result<()> {
        if let Ok(seeds) = std::env::var("THOCOIN_PEERS") {
            for s in seeds.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
                self.known_addrs.write().insert(s.clone());
                self.connect(&s);
            }
        }

        for seed in SEED_NODES {
            if let Ok(addrs) = tokio::net::lookup_host(*seed).await {
                for a in addrs {
                    let s = a.to_string();
                    self.known_addrs.write().insert(s.clone());
                    self.connect(&s);
                }
            }
        }

        let announce = self.clone();
        tokio::spawn(async move {
            let mut last = announce.chain.tip_height();
            let mut announced: HashSet<Hash> = HashSet::new();
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                let h = announce.chain.tip_height();
                if h > last {
                    last = h;
                    let tip = announce.chain.tip_hash();
                    announce.broadcast_inv(0, vec![tip], vec![]);
                }
                // Relay new mempool txs (e.g. from RPC send); local txs were never announced before.
                let cur: HashSet<Hash> = announce.mempool.entries.read().keys().cloned().collect();
                let new_txs: Vec<Hash> = cur.iter().filter(|t| !announced.contains(*t)).cloned().collect();
                if !new_txs.is_empty() {
                    announce.broadcast_inv(0, vec![], new_txs);
                }
                announced = cur; // stop tracking txs that left the mempool
            }
        });

        let listener = TcpListener::bind(("0.0.0.0", P2P_PORT)).await?;
        loop {
            let (sock, addr) = listener.accept().await?;
            if self.conn_count.load(Ordering::SeqCst) >= MAX_PEERS { continue; }
            let peer = addr.to_string();
            if self.banned.read().contains(&peer) { continue; }
            self.conn_count.fetch_add(1, Ordering::SeqCst);
            let me = self.clone();
            tokio::spawn(async move {
                let _ = me.run_peer(sock, false, peer.clone()).await;
                me.conn_count.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }

    pub fn connect(self: &Arc<Self>, addr: &str) {
        let addr = addr.to_string();
        if self.banned.read().contains(&addr) { return; }
        if self.outbound.read().contains(&addr) { return; }
        if self.conn_count.load(Ordering::SeqCst) >= MAX_PEERS { return; }
        self.outbound.write().insert(addr.clone());
        self.conn_count.fetch_add(1, Ordering::SeqCst);
        let me = self.clone();
        tokio::spawn(async move {
            if let Ok(sock) = TcpStream::connect(&addr).await {
                let _ = me.run_peer(sock, true, addr.clone()).await;
            }
            me.conn_count.fetch_sub(1, Ordering::SeqCst);
            me.outbound.write().remove(&addr);
        });
    }

    async fn run_peer(self: &Arc<Self>, sock: TcpStream, initiator: bool, peer: String)
        -> anyhow::Result<()>
    {
        sock.set_nodelay(true).ok();
        let (mut reader, mut writer) = sock.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.peers.write().insert(id, tx.clone());

        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if Self::send_msg(&mut writer, &msg).await.is_err() { break; }
            }
        });

        if initiator {
            let _ = tx.send(Msg::Version { magic: NETWORK_MAGIC, height: self.chain.tip_height() });
        }

        let mut handshaked = false;
        let mut count = 0u64;
        let mut ban: i32 = 0;
        loop {
            count += 1;
            if count > MAX_MSGS_PER_CONN { break; }
            // Timeout: a silent peer would hold a slot forever; ping once, drop on the second timeout.
            let msg = match tokio::time::timeout(
                std::time::Duration::from_secs(150), Self::read_msg(&mut reader)).await
            {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(_) => {
                    let _ = tx.send(Msg::Ping);
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(150), Self::read_msg(&mut reader)).await
                    {
                        Ok(Some(m)) => m,
                        _ => break,
                    }
                }
            };

            if !handshaked {
                match msg {
                    Msg::Version { magic, height } if magic == NETWORK_MAGIC => {
                        handshaked = true;
                        if !initiator {
                            let _ = tx.send(Msg::Version { magic: NETWORK_MAGIC, height: self.chain.tip_height() });
                        }
                        let _ = tx.send(Msg::Verack);
                        let _ = tx.send(Msg::GetAddr);
                        if height > self.chain.tip_height() {
                            let _ = tx.send(Msg::GetBlocks { from_height: self.chain.tip_height() + 1 });
                        }
                        continue;
                    }
                    _ => { ban += BAN_THRESHOLD; break; }
                }
            }

            match msg {
                Msg::Version { .. } | Msg::Verack => {}
                Msg::Ping => { let _ = tx.send(Msg::Pong); }
                Msg::Pong => {}
                Msg::GetAddr => {
                    let addrs: Vec<String> = self.known_addrs.read().iter()
                        .take(MAX_ADDR_SHARE).cloned().collect();
                    if !addrs.is_empty() { let _ = tx.send(Msg::Addr(addrs)); }
                }
                Msg::Addr(addrs) => {
                    if addrs.len() > MAX_ADDR_SHARE { ban += 20; }
                    for a in addrs.into_iter().take(MAX_ADDR_SHARE) {
                        if self.known_addrs.read().len() >= 1024 { break; }
                        let is_new = self.known_addrs.write().insert(a.clone());
                        if is_new && self.conn_count.load(Ordering::SeqCst) < MAX_PEERS / 2 {
                            self.connect(&a);
                        }
                    }
                }
                Msg::GetBlocks { from_height } => {
                    let our_h = self.chain.tip_height();
                    let mut h = from_height.max(1);
                    let mut sent = 0u64;
                    while h <= our_h && sent < MAX_SYNC_BATCH {
                        if let Some(b) = self.chain.block_at(h) {
                            let _ = tx.send(Msg::BlockMsg(b));
                            sent += 1;
                        }
                        h += 1;
                    }
                }
                Msg::Inv { blocks, txs } => {
                    if blocks.len() > MAX_INV || txs.len() > MAX_INV { ban += 20; break; }
                    let want_blocks: Vec<Hash> = blocks.into_iter()
                        .filter(|h| !self.chain.has_block(h)).collect();
                    let want_txs: Vec<Hash> = txs.into_iter()
                        .filter(|h| !self.mempool.entries.read().contains_key(h)).collect();
                    if !want_blocks.is_empty() || !want_txs.is_empty() {
                        let _ = tx.send(Msg::GetData { blocks: want_blocks, txs: want_txs });
                    }
                }
                Msg::GetData { blocks, txs } => {
                    if blocks.len() > MAX_INV || txs.len() > MAX_INV { ban += 20; break; }
                    for h in blocks {
                        if let Some(b) = self.chain.get_block(&h) {
                            let _ = tx.send(Msg::BlockMsg(b));
                        }
                    }
                    for h in txs {
                        if let Some(e) = self.mempool.entries.read().get(&h) {
                            let _ = tx.send(Msg::TxMsg(e.tx.clone()));
                        }
                    }
                }
                Msg::BlockMsg(b) => {
                    let hash = b.hash();
                    let prev = b.header.prev_hash;
                    match self.chain.accept_block(&b) {
                        Ok(true) => {
                            self.broadcast_inv(id, vec![hash], vec![]);
                            let _ = tx.send(Msg::GetBlocks { from_height: self.chain.tip_height() + 1 });
                        }
                        Ok(false) => {}
                        Err(_) => {
                            if !self.chain.has_block(&prev) {
                                let _ = tx.send(Msg::GetBlocks { from_height: self.chain.tip_height() + 1 });
                            } else {
                                ban += 10;
                            }
                        }
                    }
                }
                Msg::TxMsg(t) => {
                    let txid = t.txid();
                    if self.mempool.accept(&self.chain, t).is_ok() {
                        self.broadcast_inv(id, vec![], vec![txid]);
                    }
                }
            }

            if ban >= BAN_THRESHOLD { break; }
        }

        self.peers.write().remove(&id);
        if ban >= BAN_THRESHOLD {
            self.banned.write().insert(peer);
        }
        Ok(())
    }
}