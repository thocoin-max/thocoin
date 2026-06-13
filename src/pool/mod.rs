use std::collections::VecDeque;
use std::sync::Arc;
use parking_lot::RwLock;
use serde::{Serialize, Deserialize};

use crate::core::chain::ChainState;
use crate::core::block::{Block, BlockHeader};
use crate::core::tx::Transaction;
use crate::core::mempool::Mempool;
use crate::core::consensus::*;
use crate::core::hash::{Hash, hash_meets_target, bits_to_target};
use crate::wallet::Wallet;
use crate::wallet::address::{decode_address, script_p2pkh};

pub const PPLNS_WINDOW: usize = 1000;

pub const SHARE_SHIFT: u32 = 12;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ClientMsg {
    #[serde(rename = "subscribe")]
    Subscribe { address: String },
    #[serde(rename = "share")]
    Share { nonce: u32, timestamp: u64, extranonce: u32 },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum ServerMsg {
    #[serde(rename = "job")]
    Job {
        job_id: u64,
        prev: String,
        merkle: String,
        bits: u32,
        height: u64,
        timestamp: u64,
        share_bits: u32,
        extranonce: u32,
    },
    #[serde(rename = "accepted")]
    Accepted { shares: u64 },
    #[serde(rename = "rejected")]
    Rejected { reason: String },
    #[serde(rename = "block")]
    BlockFound { height: u64, reward: u64 },
    #[serde(rename = "error")]
    Error { message: String },
}

#[derive(Clone)]
pub struct ShareRecord { pub address: String }

pub struct Pool {
    pub chain: Arc<ChainState>,
    pub mempool: Mempool,
    pub wallet: Arc<Wallet>,
    pub shares: Arc<RwLock<VecDeque<ShareRecord>>>,
    pub job_id: Arc<RwLock<u64>>,
    pub share_count: Arc<RwLock<std::collections::HashMap<String, u64>>>,
}

impl Pool {
    pub fn new(chain: Arc<ChainState>, mempool: Mempool, wallet: Arc<Wallet>) -> Self {
        Pool {
            chain, mempool, wallet,
            shares: Arc::new(RwLock::new(VecDeque::with_capacity(PPLNS_WINDOW))),
            job_id: Arc::new(RwLock::new(0)),
            share_count: Arc::new(RwLock::new(Default::default())),
        }
    }

    pub fn share_bits(net_bits: u32) -> u32 {
        // Share target = network target * 2^SHARE_SHIFT (exactly 2^12 easier).
        // The old code only bumped the exponent by one byte (*2^8), off from SHARE_SHIFT=12.
        use crate::core::hash::{bits_to_target, target_to_bits, target_mul_div, target_cmp};
        let t = bits_to_target(net_bits);
        let mut shifted = target_mul_div(&t, 1u64 << SHARE_SHIFT, 1);
        let limit = bits_to_target(POW_LIMIT_BITS);
        if target_cmp(&shifted, &limit) == std::cmp::Ordering::Greater {
            shifted = limit;
        }
        let b = target_to_bits(&shifted);
        if b == 0 { POW_LIMIT_BITS } else { b }
    }

    pub fn build_template(&self) -> Block {
        let prev = *self.chain.tip.read();
        let height = *self.chain.height.read() + 1;
        let supply = *self.chain.supply.read();
        let reward = block_reward(height, supply);
        let (snap, fees) = self.mempool.snapshot_with_fees(500);
        let mut txs = vec![Transaction::coinbase(height, reward.saturating_add(fees),
            self.wallet.key.read().script_pubkey())];
        txs.extend(snap);
        crate::core::block::add_witness_commitment(&mut txs);
        let bits = self.chain.current_bits();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let mut b = Block::new(prev, txs, bits, ts);
        b.header.nonce = 0;
        b
    }

    pub fn make_job(&self, tmpl: &Block, extranonce: u32) -> ServerMsg {
        let mut jid = self.job_id.write();
        *jid += 1;
        ServerMsg::Job {
            job_id: *jid,
            prev: hex::encode(tmpl.header.prev_hash),
            merkle: hex::encode(tmpl.header.merkle_root),
            bits: tmpl.header.bits,
            height: *self.chain.height.read() + 1,
            timestamp: tmpl.header.timestamp,
            share_bits: Self::share_bits(tmpl.header.bits),
            extranonce,
        }
    }

    pub fn check_share(&self, tmpl: &Block, nonce: u32, ts: u64, address: &str)
        -> (bool, bool) {
        let mut hdr = tmpl.header.clone();
        hdr.nonce = nonce;
        hdr.timestamp = ts;
        let h = hdr.hash();
        let net_bits = tmpl.header.bits;
        let share_bits = Self::share_bits(net_bits);

        if !hash_meets_target(&h, share_bits) {
            return (false, false);
        }

        self.record_share(address);
        let is_block = hash_meets_target(&h, net_bits);
        (true, is_block)
    }

    fn record_share(&self, address: &str) {
        let mut dq = self.shares.write();
        if dq.len() >= PPLNS_WINDOW {
            if let Some(old) = dq.pop_front() {
                let mut sc = self.share_count.write();
                if let Some(c) = sc.get_mut(&old.address) {
                    *c = c.saturating_sub(1);
                    if *c == 0 { sc.remove(&old.address); }
                }
            }
        }
        dq.push_back(ShareRecord { address: address.to_string() });
        *self.share_count.write().entry(address.to_string()).or_insert(0) += 1;
    }

    pub fn on_block_won(&self, tmpl: &Block, nonce: u32, ts: u64) -> anyhow::Result<u64> {
        let mut block = tmpl.clone();
        block.header.nonce = nonce;
        block.header.timestamp = ts;
        let height = *self.chain.height.read() + 1;
        self.chain.apply_block(&block, height)?;
        let reward = block.transactions[0].outputs[0].value;

        let sc = self.share_count.read().clone();
        let total: u64 = sc.values().sum();
        if total == 0 { return Ok(reward); }

        let mut outs: Vec<(String, u64)> = Vec::new();
        for (addr, cnt) in sc.iter() {
            let amount = reward as u128 * (*cnt as u128) / total as u128;
            if amount > 0 { outs.push((addr.clone(), amount as u64)); }
        }

        self.queue_payout(height, outs);
        Ok(reward)
    }

    fn queue_payout(&self, height: u64, outs: Vec<(String, u64)>) {

        if let Ok(tree) = self.chain.db.open_tree("pool_payouts") {
            let data: Vec<(String, u64)> = outs;
            let _ = tree.insert(height.to_be_bytes(), bincode::serialize(&data).unwrap());
        }
    }

    pub fn process_mature_payouts(&self) {
        let cur = *self.chain.height.read();
        let tree = match self.chain.db.open_tree("pool_payouts") {
            Ok(t) => t, Err(_) => return,
        };
        let mut done: Vec<u64> = Vec::new();
        for item in tree.iter() {
            let Ok((k, v)) = item else { continue; };
            let h = u64::from_be_bytes(k.as_ref().try_into().unwrap_or([0u8;8]));
            if cur < h + COINBASE_MATURITY { continue; }
            let outs: Vec<(String, u64)> = match bincode::deserialize(&v) { Ok(x)=>x, Err(_)=>continue };
            if let Some(tx) = self.build_payout_tx(h, &outs) {
                // accept (not add): validated and fee-paying, so other nodes relay it.
                if let Err(e) = self.mempool.accept(&self.chain, tx) {
                    eprintln!("payout at height {h} rejected by mempool: {e}");
                }
            }
            done.push(h);
        }
        for h in done { let _ = tree.remove(h.to_be_bytes()); }
    }

    fn build_payout_tx(&self, h: u64, outs: &[(String, u64)]) -> Option<Transaction> {
        use crate::core::tx::{TxIn, TxOut, OutPoint};
        use crate::core::consensus::MIN_RELAY_FEE_PER_KB;

        let (hash, _) = self.chain.height_index.read().get(&h).cloned()?;
        let (blk, _) = self.chain.headers.read().get(&hash).cloned()?;
        let cb = &blk.transactions[0];
        let cb_txid = cb.txid();
        let prev = OutPoint { txid: cb_txid, vout: 0 };
        let key = self.wallet.key.read();

        let build = |outputs: Vec<TxOut>| -> Transaction {
            let mut tx = Transaction {
                version: 1,
                inputs: vec![TxIn {
                    prev: prev.clone(),
                    signature: vec![],
                    pubkey: key.pubkey_bytes(),
                    sequence: 0xffffffff,
                }],
                outputs,
                lock_time: 0,
            };
            let sighash = tx.sighash(0);
            tx.inputs[0].signature = key.sign(&sighash);
            tx
        };

        let mut outputs = Vec::new();
        for (addr, amt) in outs {
            if let Ok(h20) = decode_address(addr) {
                outputs.push(TxOut { value: *amt, script_pubkey: script_p2pkh(&h20) });
            }
        }
        if outputs.is_empty() { return None; }

        // Pass 1: measure size to compute the fee. Pass 2: deduct it pro-rata and re-sign.
        // Previously the fee was 0, so the payout tx was never relayed.
        let probe = build(outputs.clone());
        let fee = (probe.size() as u64)
            .saturating_mul(MIN_RELAY_FEE_PER_KB).div_ceil(1000)
            .saturating_add(MIN_RELAY_FEE_PER_KB); // headroom for minor size changes
        let total: u64 = outputs.iter().map(|o| o.value).sum();
        if total <= fee { return None; }

        let mut deducted = 0u64;
        for o in outputs.iter_mut() {
            let cut = ((o.value as u128 * fee as u128) / total as u128) as u64;
            let cut = cut.min(o.value);
            o.value -= cut;
            deducted += cut;
        }
        if deducted < fee {
            let mut rest = fee - deducted;
            for o in outputs.iter_mut() {
                let cut = rest.min(o.value);
                o.value -= cut;
                rest -= cut;
                if rest == 0 { break; }
            }
        }
        outputs.retain(|o| o.value > 0);
        if outputs.is_empty() { return None; }

        Some(build(outputs))
    }

    pub fn stats(&self) -> (usize, usize) {
        (self.shares.read().len(), self.share_count.read().len())
    }
}

pub fn hex32(s: &str) -> Option<Hash> {
    let v = hex::decode(s).ok()?;
    if v.len() != 32 { return None; }
    let mut h = [0u8; 32];
    h.copy_from_slice(&v);
    Some(h)
}

pub fn share_target_hex(net_bits: u32) -> String {
    hex::encode(bits_to_target(Pool::share_bits(net_bits)))
}

pub fn header_from_job(prev: Hash, merkle: Hash, bits: u32, ts: u64, nonce: u32) -> BlockHeader {
    BlockHeader { version: 1, prev_hash: prev, merkle_root: merkle, timestamp: ts, bits, nonce }
}