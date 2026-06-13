pub mod gpu;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::core::chain::ChainState;
use crate::core::block::Block;
use crate::core::tx::Transaction;
use crate::core::mempool::Mempool;
use crate::core::consensus::*;
use crate::wallet::Wallet;

pub struct Miner {
    pub chain: Arc<ChainState>,
    pub mempool: Mempool,
    pub wallet: Arc<Wallet>,
    pub running: Arc<AtomicBool>,
    pub hashrate: Arc<AtomicU64>,
    pub blocks_found: Arc<AtomicU64>,
    pub threads: usize,
}

impl Miner {
    pub fn new(chain: Arc<ChainState>, mempool: Mempool, wallet: Arc<Wallet>, threads: usize) -> Self {
        Miner {
            chain, mempool, wallet,
            running: Arc::new(AtomicBool::new(false)),
            hashrate: Arc::new(AtomicU64::new(0)),
            blocks_found: Arc::new(AtomicU64::new(0)),
            threads,
        }
    }

    pub fn start(self: &Arc<Self>) {
        if self.running.load(Ordering::SeqCst) { return; }
        self.running.store(true, Ordering::SeqCst);
        for tid in 0..self.threads {
            let me = self.clone();
            thread::spawn(move || me.mine_loop(tid as u32));
        }
    }

    pub fn stop(&self) { self.running.store(false, Ordering::SeqCst); }

    fn build_candidate(&self, nonce_offset: u32) -> (Block, Vec<[u8; 32]>) {
        let prev = *self.chain.tip.read();
        let height = *self.chain.height.read() + 1;
        let supply = *self.chain.supply.read();
        let reward = block_reward(height, supply);
        let (snap, fees) = self.mempool.snapshot_with_fees(500);
        let txids: Vec<[u8; 32]> = snap.iter().map(|t| t.txid()).collect();
        // Coinbase claims reward plus total fees.
        let mut txs = vec![Transaction::coinbase(height, reward.saturating_add(fees),
            self.wallet.key.read().script_pubkey())];
        txs.extend(snap);
        crate::core::block::add_witness_commitment(&mut txs);
        let bits = self.chain.current_bits();
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let mut b = Block::new(prev, txs, bits, ts);
        b.header.nonce = nonce_offset;
        (b, txids)
    }

    fn mine_loop(self: Arc<Self>, tid: u32) {
        while self.running.load(Ordering::SeqCst) {
            let (mut block, txids) = self.build_candidate(tid * 1_000_000_000);
            let start_nonce = block.header.nonce;
            loop {
                if !self.running.load(Ordering::SeqCst) { return; }
                for _ in 0..50_000 {
                    if block.header.meets_pow() {
                        let height = *self.chain.height.read() + 1;
                        if self.chain.apply_block(&block, height).is_ok() {
                            self.blocks_found.fetch_add(1, Ordering::Relaxed);
                            self.mempool.remove_txs(&txids);
                        }
                        break;
                    }
                    block.header.nonce = block.header.nonce.wrapping_add(1);
                    self.hashrate.fetch_add(1, Ordering::Relaxed);
                }
                let tip = *self.chain.tip.read();
                if block.header.prev_hash != tip { break; }
                if block.header.nonce.wrapping_sub(start_nonce) > 5_000_000 { break; }
            }
        }
    }
}
