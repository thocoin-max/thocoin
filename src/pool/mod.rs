use std::collections::VecDeque;
use std::sync::Arc;
use parking_lot::RwLock;
use serde::{Serialize, Deserialize};

pub mod client;
pub mod embedded;

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
    #[serde(rename = "hashrate")]
    Hashrate { hashrate: u64 },
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
    #[serde(rename = "poolstats")]
    PoolStats { miners: u64, pool_hashrate: u64 },
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
        // Share target = network target * 2^SHARE_SHIFT (always 2^12 easier).
        // NOT clamped to POW_LIMIT: shares are an off-chain accounting target, so
        // even at genesis difficulty (net_bits == POW_LIMIT_BITS) miners still
        // produce frequent shares. Clamping made share_bits == net_bits there,
        // so a fixed template often had zero valid share nonces -> 0 shares.
        use crate::core::hash::{bits_to_target, target_to_bits, target_mul_div};
        let t = bits_to_target(net_bits);
        let shifted = target_mul_div(&t, 1u64 << SHARE_SHIFT, 1);
        let b = target_to_bits(&shifted);
        if b == 0 { POW_LIMIT_BITS } else { b }
    }

    pub fn build_template(&self) -> Block {
        self.build_template_fee(1.0)
    }

    /// Build a block template whose COINBASE already splits the reward among the
    /// current PPLNS shareholders (P2Pool-style). Paying inside the coinbase makes
    /// payouts atomic with the won block: no separate payout tx, nothing can get
    /// stuck in the mempool, and miners are credited the instant the block applies.
    /// `fee_frac` in [0,1] is the share kept by the pool wallet (e.g. 0.99 = 1% fee).
    pub fn build_template_fee(&self, fee_frac: f64) -> Block {
        let prev = *self.chain.tip.read();
        let height = *self.chain.height.read() + 1;
        let supply = *self.chain.supply.read();
        let reward = block_reward(height, supply);
        let (snap, fees) = self.mempool.snapshot_with_fees(500);
        let total = reward.saturating_add(fees);

        let pool_script = self.wallet.key.read().script_pubkey();
        let outputs = self.payout_outputs(total, fee_frac, &pool_script);

        let mut coinbase = Transaction::coinbase(height, 0, pool_script.clone());
        coinbase.outputs = outputs;

        let mut txs = vec![coinbase];
        txs.extend(snap);
        crate::core::block::add_witness_commitment(&mut txs);
        let bits = self.chain.current_bits();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let mut b = Block::new(prev, txs, bits, ts);
        b.header.nonce = 0;
        b
    }

    /// Split `total` across PPLNS shareholders by share count. A `fee_frac` cut and
    /// any rounding dust go to the pool wallet. Always returns at least one output.
    fn payout_outputs(&self, total: u64, fee_frac: f64, pool_script: &[u8]) -> Vec<crate::core::tx::TxOut> {
        use crate::core::tx::TxOut;
        let sc = self.share_count.read().clone();
        let shares_total: u64 = sc.values().sum();

        // No shares yet (or fee disabled): everything to the pool wallet.
        if shares_total == 0 {
            return vec![TxOut { value: total, script_pubkey: pool_script.to_vec() }];
        }

        let fee_frac = fee_frac.clamp(0.0, 1.0);
        let pool_fee = ((total as f64) * (1.0 - fee_frac)) as u64;
        let distributable = total.saturating_sub(pool_fee);

        let mut outs: Vec<TxOut> = Vec::new();
        let mut paid: u64 = 0;
        for (addr, cnt) in sc.iter() {
            let amount = (distributable as u128 * (*cnt as u128) / shares_total as u128) as u64;
            if amount == 0 { continue; }
            if let Ok(h20) = decode_address(addr) {
                outs.push(TxOut { value: amount, script_pubkey: script_p2pkh(&h20) });
                paid += amount;
            }
        }

        // Pool fee + rounding dust + any amount for undecodable addresses.
        let pool_cut = total.saturating_sub(paid);
        if pool_cut > 0 {
            outs.push(TxOut { value: pool_cut, script_pubkey: pool_script.to_vec() });
        }
        if outs.is_empty() {
            outs.push(TxOut { value: total, script_pubkey: pool_script.to_vec() });
        }
        outs
    }

    pub fn make_job(&self, tmpl: &Block, extranonce: u32) -> ServerMsg {
        self.make_job_at(tmpl, extranonce, tmpl.header.timestamp)
    }

    /// Build a job carrying an explicit timestamp. Rolling the timestamp gives the
    /// miner a fresh nonce search space without changing prev/merkle, so shares
    /// validated against the same template still hash identically. This is what
    /// keeps blocks coming at mainnet difficulty (one fixed 2^32 nonce space is
    /// too small to reliably contain a valid block nonce).
    pub fn make_job_at(&self, tmpl: &Block, extranonce: u32, timestamp: u64) -> ServerMsg {
        let mut jid = self.job_id.write();
        *jid += 1;
        ServerMsg::Job {
            job_id: *jid,
            prev: hex::encode(tmpl.header.prev_hash),
            merkle: hex::encode(tmpl.header.merkle_root),
            bits: tmpl.header.bits,
            height: *self.chain.height.read() + 1,
            timestamp,
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
        // The template's coinbase already splits the reward among PPLNS
        // shareholders, so winning the block IS the payout — applying it credits
        // every miner atomically. No separate payout tx, nothing to mature.
        if tmpl.header.prev_hash != *self.chain.tip.read() {
            anyhow::bail!("stale template (tip advanced); win discarded");
        }
        let mut block = tmpl.clone();
        block.header.nonce = nonce;
        block.header.timestamp = ts;
        let height = *self.chain.height.read() + 1;
        self.chain.apply_block(&block, height)?;
        let reward: u64 = block.transactions[0].outputs.iter().map(|o| o.value).sum();
        Ok(reward)
    }

    /// Kept for API compatibility with the pool servers' background loops.
    /// Payouts now happen inside the coinbase at block-win time, so there is
    /// nothing to process here.
    pub fn process_mature_payouts(&self) {}

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