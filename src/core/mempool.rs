use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use parking_lot::RwLock;
use anyhow::{Result, anyhow};
use crate::core::tx::{Transaction, OutPoint};
use crate::core::hash::Hash;
use crate::core::chain::ChainState;
use crate::core::consensus::{MAX_BLOCK_SIZE, MAX_BLOCK_SIGOPS, MIN_RELAY_FEE_PER_KB};

const MAX_MEMPOOL: usize = 50_000;
const COINBASE_RESERVE: usize = 4_096;

#[derive(Clone)]
pub struct Entry {
    pub tx: Transaction,
    pub fee: u64,
    pub size: usize,
}

impl Entry {
    pub fn feerate(&self) -> u64 {
        let kb = (self.size as u64).max(1);
        self.fee.saturating_mul(1000) / kb
    }
}

#[derive(Default, Clone)]
pub struct Mempool {
    pub entries: Arc<RwLock<HashMap<Hash, Entry>>>,
    spent: Arc<RwLock<HashSet<OutPoint>>>,
}

impl Mempool {
    pub fn new() -> Self { Self::default() }

    pub fn accept(&self, chain: &ChainState, tx: Transaction) -> Result<()> {
        if tx.is_coinbase() {
            return Err(anyhow!("khong nhan coinbase vao mempool"));
        }
        if tx.inputs.is_empty() || tx.outputs.is_empty() {
            return Err(anyhow!("tx thieu input/output"));
        }
        if tx.has_duplicate_inputs() {
            return Err(anyhow!("tx co input trung"));
        }
        let size = tx.size();
        if size > MAX_BLOCK_SIZE {
            return Err(anyhow!("tx qua lon"));
        }

        {
            let spent = self.spent.read();
            for input in &tx.inputs {
                if spent.contains(&input.prev) {
                    return Err(anyhow!("input da bi tx khac trong mempool tieu"));
                }
            }
        }

        let utxo = chain.utxo.read();
        let mut tx_in: u64 = 0;
        let mut tx_out: u64 = 0;
        for o in &tx.outputs {
            tx_out = tx_out.checked_add(o.value).ok_or_else(|| anyhow!("output overflow"))?;
        }
        for (vin, input) in tx.inputs.iter().enumerate() {
            let prev = utxo.get(&input.prev).cloned()
                .ok_or_else(|| anyhow!("UTXO khong ton tai"))?;
            let expected = crate::wallet::address::script_p2pkh(
                &crate::core::hash::hash160(&input.pubkey));
            if expected != prev.script_pubkey {
                return Err(anyhow!("pubkey khong khop UTXO"));
            }
            let sighash = tx.sighash(vin);
            if !crate::wallet::address::verify(&input.pubkey, &sighash, &input.signature) {
                return Err(anyhow!("chu ky khong hop le"));
            }
            tx_in = tx_in.checked_add(prev.value).ok_or_else(|| anyhow!("input overflow"))?;
        }
        if tx_out > tx_in {
            return Err(anyhow!("tx chi vuot input"));
        }
        drop(utxo);

        let fee = tx_in - tx_out;
        let entry = Entry { tx, fee, size };
        let min_fee = (size as u64).saturating_mul(MIN_RELAY_FEE_PER_KB) / 1000;
        if fee < min_fee {
            return Err(anyhow!("phi duoi muc toi thieu ({} < {})", fee, min_fee));
        }

        let mut map = self.entries.write();
        if map.len() >= MAX_MEMPOOL {
            let lowest = map.iter()
                .min_by_key(|(_, e)| e.feerate())
                .map(|(id, e)| (*id, e.feerate()));
            match lowest {
                Some((id, fr)) if entry.feerate() > fr => {
                    if let Some(old) = map.remove(&id) {
                        let mut spent = self.spent.write();
                        for i in &old.tx.inputs { spent.remove(&i.prev); }
                    }
                }
                _ => return Err(anyhow!("mempool day, phi qua thap")),
            }
        }
        let txid = entry.tx.txid();
        let mut spent = self.spent.write();
        for input in &entry.tx.inputs { spent.insert(input.prev.clone()); }
        drop(spent);
        map.insert(txid, entry);
        Ok(())
    }

    pub fn add(&self, tx: Transaction) {
        let size = tx.size();
        let mut spent = self.spent.write();
        for input in &tx.inputs { spent.insert(input.prev.clone()); }
        drop(spent);
        let txid = tx.txid();
        self.entries.write().insert(txid, Entry { tx, fee: 0, size });
    }

    fn ranked(&self) -> Vec<Entry> {
        let mut v: Vec<Entry> = self.entries.read().values().cloned().collect();
        v.sort_by(|a, b| b.feerate().cmp(&a.feerate()).then(b.fee.cmp(&a.fee)));
        v
    }

    pub fn snapshot(&self, max: usize) -> Vec<Transaction> {
        let mut out = Vec::new();
        let mut bytes = COINBASE_RESERVE;
        let mut sigops = 0usize;
        for e in self.ranked() {
            if out.len() >= max { break; }
            if bytes + e.size > MAX_BLOCK_SIZE { continue; }
            if sigops + e.tx.sigops() > MAX_BLOCK_SIGOPS { continue; }
            bytes += e.size;
            sigops += e.tx.sigops();
            out.push(e.tx);
        }
        out
    }

    pub fn remove_txs(&self, txids: &[Hash]) {
        let mut w = self.entries.write();
        let mut spent = self.spent.write();
        for id in txids {
            if let Some(e) = w.remove(id) {
                for input in &e.tx.inputs { spent.remove(&input.prev); }
            }
        }
    }

    pub fn drain(&self, max: usize) -> Vec<Transaction> {
        let take = self.snapshot(max);
        let ids: Vec<Hash> = take.iter().map(|t| t.txid()).collect();
        self.remove_txs(&ids);
        take
    }

    pub fn len(&self) -> usize { self.entries.read().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}
