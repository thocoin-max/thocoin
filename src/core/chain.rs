use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use parking_lot::{RwLock, Mutex};
use anyhow::{Result, anyhow};
use serde::{Serialize, Deserialize};
use crate::core::block::Block;
use crate::core::tx::{Transaction, OutPoint, TxOut};
use crate::core::hash::{Hash, work_from_bits, target_add, target_cmp};
use crate::core::consensus::*;

type UtxoMap = HashMap<OutPoint, TxOut>;
type CbMap = HashMap<OutPoint, u64>;

#[derive(Clone, Serialize, Deserialize)]
struct Idx {
    prev: Hash,
    height: u64,
    work: [u8; 32],
    ts: u64,
    bits: u32,
}

pub struct ChainState {
    pub apply_lock: Mutex<()>,
    pub db: sled::Db,
    pub utxo: Arc<RwLock<UtxoMap>>,
    pub tip: Arc<RwLock<Hash>>,
    pub height: Arc<RwLock<u64>>,
    pub supply: Arc<RwLock<u64>>,
    pub headers: Arc<RwLock<HashMap<Hash, (Block, u64)>>>,
    pub height_index: Arc<RwLock<HashMap<u64, (Hash, u64)>>>,
    pub coinbase_at: Arc<RwLock<CbMap>>,
    index: Arc<RwLock<HashMap<Hash, Idx>>>,
    block_store: Arc<RwLock<HashMap<Hash, Block>>>,
    tip_work: Arc<RwLock<[u8; 32]>>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0)
}

impl ChainState {
    pub fn open(path: &str) -> Result<Self> {
        let db = sled::open(path)?;
        let state = ChainState {
            apply_lock: Mutex::new(()),
            db,
            utxo: Arc::new(RwLock::new(HashMap::new())),
            tip: Arc::new(RwLock::new([0u8; 32])),
            height: Arc::new(RwLock::new(0)),
            supply: Arc::new(RwLock::new(0)),
            headers: Arc::new(RwLock::new(HashMap::new())),
            height_index: Arc::new(RwLock::new(HashMap::new())),
            coinbase_at: Arc::new(RwLock::new(HashMap::new())),
            index: Arc::new(RwLock::new(HashMap::new())),
            block_store: Arc::new(RwLock::new(HashMap::new())),
            tip_work: Arc::new(RwLock::new([0u8; 32])),
        };
        state.load_or_init()?;
        Ok(state)
    }

    fn load_or_init(&self) -> Result<()> {
        if let Some(v) = self.db.get(b"tip")? {
            let tip: Hash = bincode::deserialize(&v)?;
            *self.tip.write() = tip;
            if let Some(h) = self.db.get(b"height")? { *self.height.write() = bincode::deserialize(&h)?; }
            if let Some(s) = self.db.get(b"supply")? { *self.supply.write() = bincode::deserialize(&s)?; }
            if let Some(w) = self.db.get(b"tipwork")? { *self.tip_work.write() = bincode::deserialize(&w)?; }
            self.load_blocks()?;
            self.load_index()?;
            self.load_utxo()?;
            self.rebuild_active()?;
        } else {
            self.create_genesis()?;
        }
        Ok(())
    }

    fn load_utxo(&self) -> Result<()> {
        let tree = self.db.open_tree("utxo")?;
        let mut map = self.utxo.write();
        for item in tree.iter() {
            let (k, v) = item?;
            let op: OutPoint = bincode::deserialize(&k)?;
            let out: TxOut = bincode::deserialize(&v)?;
            map.insert(op, out);
        }
        Ok(())
    }

    fn load_blocks(&self) -> Result<()> {
        let tree = self.db.open_tree("blocks")?;
        let mut store = self.block_store.write();
        for item in tree.iter() {
            let (_k, v) = item?;
            let blk: Block = bincode::deserialize(&v)?;
            store.insert(blk.hash(), blk);
        }
        Ok(())
    }

    fn load_index(&self) -> Result<()> {
        let tree = self.db.open_tree("idx")?;
        let mut map = self.index.write();
        for item in tree.iter() {
            let (k, v) = item?;
            let h: Hash = bincode::deserialize(&k)?;
            let idx: Idx = bincode::deserialize(&v)?;
            map.insert(h, idx);
        }
        Ok(())
    }

    fn rebuild_active(&self) -> Result<()> {
        let tip = *self.tip.read();
        let index = self.index.read();
        let store = self.block_store.read();
        let mut headers = self.headers.write();
        let mut hidx = self.height_index.write();
        let mut cb = self.coinbase_at.write();
        headers.clear(); hidx.clear(); cb.clear();
        let mut cur = tip;
        while let Some(idx) = index.get(&cur) {
            if let Some(blk) = store.get(&cur) {
                if let Some(coinbase) = blk.transactions.first() {
                    if coinbase.is_coinbase() {
                        let txid = coinbase.txid();
                        for vout in 0..coinbase.outputs.len() {
                            cb.insert(OutPoint { txid, vout: vout as u32 }, idx.height);
                        }
                    }
                }
                hidx.insert(idx.height, (cur, blk.header.timestamp));
                headers.insert(cur, (blk.clone(), idx.height));
            }
            if idx.height == 0 { break; }
            cur = idx.prev;
        }
        Ok(())
    }

    fn create_genesis(&self) -> Result<()> {
        let cb = Transaction::coinbase(0, INITIAL_REWARD, crate::wallet::address::genesis_script());
        let mut genesis = Block::new([0u8; 32], vec![cb], GENESIS_BITS, 1735689600);
        loop {
            if genesis.header.meets_pow() { break; }
            genesis.header.nonce = genesis.header.nonce.wrapping_add(1);
            if genesis.header.nonce == 0 { genesis.header.timestamp += 1; }
        }
        let _guard = self.apply_lock.lock();
        self.connect_genesis(&genesis)
    }

    fn store_block_meta(&self, block: &Block, idx: &Idx) -> Result<()> {
        let h = block.hash();
        self.block_store.write().insert(h, block.clone());
        self.index.write().insert(h, idx.clone());
        self.db.open_tree("blocks")?.insert(h, bincode::serialize(block)?)?;
        self.db.open_tree("idx")?.insert(h, bincode::serialize(idx)?)?;
        Ok(())
    }

    fn ancestors(&self, from: &Hash, max: usize) -> Vec<(u64, u64, u32)> {
        let index = self.index.read();
        let mut out = Vec::new();
        let mut cur = *from;
        for _ in 0..max {
            match index.get(&cur) {
                Some(idx) => {
                    out.push((idx.height, idx.ts, idx.bits));
                    if idx.height == 0 { break; }
                    cur = idx.prev;
                }
                None => break,
            }
        }
        out.reverse();
        out
    }

    fn bits_for_prev(&self, prev: &Hash, height: u64) -> u32 {
        if height == 0 { return GENESIS_BITS; }
        let window = LWMA_WINDOW as usize + 1;
        let anc = self.ancestors(prev, window);
        if anc.len() < 2 { return GENESIS_BITS; }
        let times: Vec<u64> = anc.iter().map(|(_, t, _)| *t).collect();
        let bits: Vec<u32> = anc.iter().map(|(_, _, b)| *b).collect();
        lwma_next_bits(&times, &bits)
    }

    fn mtp_branch(&self, prev: &Hash) -> u64 {
        let anc = self.ancestors(prev, 11);
        if anc.is_empty() { return 0; }
        let mut ts: Vec<u64> = anc.iter().map(|(_, t, _)| *t).collect();
        ts.sort_unstable();
        ts[ts.len() / 2]
    }

    pub fn current_bits(&self) -> u32 {
        let tip = *self.tip.read();
        let h = *self.height.read() + 1;
        self.bits_for_prev(&tip, h)
    }

    pub fn bits_for_height(&self, height: u64) -> u32 {
        if height == 0 { return GENESIS_BITS; }
        let prev = match self.height_index.read().get(&(height - 1)) {
            Some((h, _)) => *h,
            None => return GENESIS_BITS,
        };
        self.bits_for_prev(&prev, height)
    }

    pub fn next_timestamp(&self) -> u64 {
        let tip = *self.tip.read();
        let mtp = self.mtp_branch(&tip);
        now_secs().max(mtp + 1)
    }

    fn validate_block_basic(block: &Block, height: u64, expected_bits: u32, mtp: u64) -> Result<()> {
        if let Some(cp) = checkpoint_hash(height) {
            if block.hash() != cp {
                return Err(anyhow!("block khong khop checkpoint tai height {}", height));
            }
        }
        if block.transactions.is_empty() {
            return Err(anyhow!("block rong"));
        }
        let raw = bincode::serialize(block).map(|b| b.len()).unwrap_or(usize::MAX);
        if raw > MAX_BLOCK_SIZE {
            return Err(anyhow!("block vuot MAX_BLOCK_SIZE"));
        }
        let sigops: usize = block.transactions.iter().map(|t| t.sigops()).sum();
        if sigops > MAX_BLOCK_SIGOPS {
            return Err(anyhow!("block vuot MAX_BLOCK_SIGOPS"));
        }
        if block.header.merkle_root != Block::merkle_root(&block.transactions) {
            return Err(anyhow!("merkle root khong khop"));
        }
        if !block.header.meets_pow() {
            return Err(anyhow!("PoW invalid"));
        }
        if height > 0 {
            if block.header.bits != expected_bits {
                return Err(anyhow!("bad bits: got 0x{:08x}, expected 0x{:08x}",
                    block.header.bits, expected_bits));
            }
            if block.header.timestamp < mtp {
                return Err(anyhow!("timestamp < median-time-past"));
            }
            if block.header.timestamp > now_secs() + 2 * 60 * 60 {
                return Err(anyhow!("timestamp qua xa tuong lai"));
            }
        }
        Ok(())
    }

    fn validate_txs(utxo: &UtxoMap, cb: &CbMap, supply: u64, block: &Block, height: u64)
        -> Result<(u64, u64)>
    {
        let mut coinbase_count = 0usize;
        let mut spent_in_block: HashSet<(Hash, u32)> = HashSet::new();
        let reward = block_reward(height, supply);
        let mut total_fees = 0u64;

        for (i, tx) in block.transactions.iter().enumerate() {
            if tx.is_coinbase() {
                coinbase_count += 1;
                if i != 0 { return Err(anyhow!("coinbase khong o vi tri 0")); }
                continue;
            }
            if tx.inputs.is_empty() { return Err(anyhow!("tx khong co input")); }
            if tx.has_duplicate_inputs() { return Err(anyhow!("tx co input trung")); }

            let mut tx_in: u64 = 0;
            let mut tx_out: u64 = 0;
            for o in &tx.outputs {
                tx_out = tx_out.checked_add(o.value).ok_or_else(|| anyhow!("output overflow"))?;
            }
            for (vin, input) in tx.inputs.iter().enumerate() {
                let key = (input.prev.txid, input.prev.vout);
                if !spent_in_block.insert(key) {
                    return Err(anyhow!("double-spend trong cung block"));
                }
                let prev = utxo.get(&input.prev).cloned()
                    .ok_or_else(|| anyhow!("UTXO missing hoac da chi"))?;
                if let Some(&cb_height) = cb.get(&input.prev) {
                    if height < cb_height + COINBASE_MATURITY {
                        return Err(anyhow!("coinbase chua du tuoi"));
                    }
                }
                let expected = crate::wallet::address::script_p2pkh(
                    &crate::core::hash::hash160(&input.pubkey));
                if expected != prev.script_pubkey {
                    return Err(anyhow!("pubkey khong khop script"));
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
            total_fees = total_fees.checked_add(tx_in - tx_out)
                .ok_or_else(|| anyhow!("fee overflow"))?;
        }
        if coinbase_count != 1 {
            return Err(anyhow!("block phai co dung 1 coinbase"));
        }
        let cb_out: u64 = block.transactions[0].outputs.iter()
            .try_fold(0u64, |a, o| a.checked_add(o.value))
            .ok_or_else(|| anyhow!("coinbase overflow"))?;
        let max_cb = reward.checked_add(total_fees).ok_or_else(|| anyhow!("reward+fees overflow"))?;
        if cb_out > max_cb {
            return Err(anyhow!("coinbase vuot reward+fees"));
        }
        Ok((total_fees, cb_out))
    }

    fn apply_txs(utxo: &mut UtxoMap, cb: &mut CbMap, supply: &mut u64,
                 block: &Block, height: u64, fees: u64, cb_out: u64)
    {
        for tx in &block.transactions {
            if !tx.is_coinbase() {
                for input in &tx.inputs {
                    utxo.remove(&input.prev);
                    cb.remove(&input.prev);
                }
            }
            let txid = tx.txid();
            let is_cb = tx.is_coinbase();
            for (vout, out) in tx.outputs.iter().enumerate() {
                let op = OutPoint { txid, vout: vout as u32 };
                if is_cb { cb.insert(op.clone(), height); }
                utxo.insert(op, out.clone());
            }
        }
        let issued = cb_out as i128 - fees as i128;
        *supply = ((*supply as i128) + issued).max(0) as u64;
    }

    fn connect_genesis(&self, block: &Block) -> Result<()> {
        let (fees, cb_out) = Self::validate_txs(&self.utxo.read(), &self.coinbase_at.read(), 0, block, 0)?;
        Self::validate_block_basic(block, 0, GENESIS_BITS, 0)?;
        let mut utxo = self.utxo.write();
        let mut cb = self.coinbase_at.write();
        let mut supply = 0u64;
        Self::apply_txs(&mut utxo, &mut cb, &mut supply, block, 0, fees, cb_out);
        let utxo_tree = self.db.open_tree("utxo")?;
        let mut batch = sled::Batch::default();
        for (op, out) in utxo.iter() {
            batch.insert(bincode::serialize(op)?, bincode::serialize(out)?);
        }
        utxo_tree.apply_batch(batch)?;
        drop(utxo); drop(cb);
        let work = work_from_bits(block.header.bits);
        let idx = Idx { prev: [0u8; 32], height: 0, work, ts: block.header.timestamp, bits: block.header.bits };
        self.store_block_meta(block, &idx)?;
        *self.supply.write() = supply;
        *self.tip.write() = block.hash();
        *self.height.write() = 0;
        *self.tip_work.write() = work;
        self.headers.write().insert(block.hash(), (block.clone(), 0));
        self.height_index.write().insert(0, (block.hash(), block.header.timestamp));
        self.persist_meta(&block.hash(), 0, supply, &work)?;
        Ok(())
    }

    fn connect_tip(&self, block: &Block) -> Result<()> {
        let tip = *self.tip.read();
        let height = *self.height.read() + 1;
        if block.header.prev_hash != tip {
            return Err(anyhow!("prev_hash khong khop tip"));
        }
        let expected = self.bits_for_prev(&tip, height);
        let mtp = self.mtp_branch(&tip);
        Self::validate_block_basic(block, height, expected, mtp)?;

        let supply_before = *self.supply.read();
        let (fees, cb_out) = {
            let utxo = self.utxo.read();
            let cb = self.coinbase_at.read();
            Self::validate_txs(&utxo, &cb, supply_before, block, height)?
        };

        let utxo_tree = self.db.open_tree("utxo")?;
        let mut batch = sled::Batch::default();
        {
            let mut utxo = self.utxo.write();
            let mut cb = self.coinbase_at.write();
            for tx in &block.transactions {
                if !tx.is_coinbase() {
                    for input in &tx.inputs {
                        utxo.remove(&input.prev);
                        cb.remove(&input.prev);
                        batch.remove(bincode::serialize(&input.prev)?);
                    }
                }
                let txid = tx.txid();
                let is_cb = tx.is_coinbase();
                for (vout, out) in tx.outputs.iter().enumerate() {
                    let op = OutPoint { txid, vout: vout as u32 };
                    if is_cb { cb.insert(op.clone(), height); }
                    batch.insert(bincode::serialize(&op)?, bincode::serialize(out)?);
                    utxo.insert(op, out.clone());
                }
            }
        }
        utxo_tree.apply_batch(batch)?;

        let new_supply = {
            let issued = cb_out as i128 - fees as i128;
            ((supply_before as i128) + issued).max(0) as u64
        };
        let pwork = *self.tip_work.read();
        let work = target_add(&pwork, &work_from_bits(block.header.bits));
        let idx = Idx { prev: tip, height, work, ts: block.header.timestamp, bits: block.header.bits };
        self.store_block_meta(block, &idx)?;

        *self.supply.write() = new_supply;
        *self.tip.write() = block.hash();
        *self.height.write() = height;
        *self.tip_work.write() = work;
        self.headers.write().insert(block.hash(), (block.clone(), height));
        self.height_index.write().insert(height, (block.hash(), block.header.timestamp));
        self.persist_meta(&block.hash(), height, new_supply, &work)?;
        Ok(())
    }

    fn persist_meta(&self, tip: &Hash, height: u64, supply: u64, work: &[u8; 32]) -> Result<()> {
        self.db.insert(b"tip", bincode::serialize(tip)?)?;
        self.db.insert(b"height", bincode::serialize(&height)?)?;
        self.db.insert(b"supply", bincode::serialize(&supply)?)?;
        self.db.insert(b"tipwork", bincode::serialize(work)?)?;
        Ok(())
    }

    fn path_from_genesis(&self, target: &Hash) -> Result<Vec<Hash>> {
        let index = self.index.read();
        let mut path = Vec::new();
        let mut cur = *target;
        loop {
            let idx = index.get(&cur).ok_or_else(|| anyhow!("thieu block trong index"))?;
            path.push(cur);
            if idx.height == 0 { break; }
            cur = idx.prev;
        }
        path.reverse();
        Ok(path)
    }

    fn switch_to(&self, target: &Hash) -> Result<()> {
        let path = self.path_from_genesis(target)?;
        let store = self.block_store.read();
        let mut utxo: UtxoMap = HashMap::new();
        let mut cb: CbMap = HashMap::new();
        let mut supply: u64 = 0;
        for h in &path {
            let blk = store.get(h).ok_or_else(|| anyhow!("thieu block body"))?;
            let height = self.index.read().get(h).map(|i| i.height)
                .ok_or_else(|| anyhow!("thieu idx"))?;
            let (fees, cb_out) = Self::validate_txs(&utxo, &cb, supply, blk, height)?;
            Self::apply_txs(&mut utxo, &mut cb, &mut supply, blk, height, fees, cb_out);
        }
        drop(store);

        let utxo_tree = self.db.open_tree("utxo")?;
        utxo_tree.clear()?;
        let mut batch = sled::Batch::default();
        for (op, out) in utxo.iter() {
            batch.insert(bincode::serialize(op)?, bincode::serialize(out)?);
        }
        utxo_tree.apply_batch(batch)?;

        let target_idx = self.index.read().get(target).cloned()
            .ok_or_else(|| anyhow!("thieu target idx"))?;
        *self.utxo.write() = utxo;
        *self.coinbase_at.write() = cb;
        *self.supply.write() = supply;
        *self.tip.write() = *target;
        *self.height.write() = target_idx.height;
        *self.tip_work.write() = target_idx.work;
        self.persist_meta(target, target_idx.height, supply, &target_idx.work)?;
        self.rebuild_active()?;
        Ok(())
    }

    pub fn apply_block(&self, block: &Block, height: u64) -> Result<()> {
        let _guard = self.apply_lock.lock();
        if height == 0 {
            return Err(anyhow!("genesis da ton tai"));
        }
        if height != *self.height.read() + 1 {
            return Err(anyhow!("height khong lien tuc"));
        }
        self.connect_tip(block)
    }

    pub fn accept_block(&self, block: &Block) -> Result<bool> {
        let _guard = self.apply_lock.lock();
        let hash = block.hash();
        if self.index.read().contains_key(&hash) {
            return Ok(false);
        }
        let prev = block.header.prev_hash;

        if prev == *self.tip.read() {
            self.connect_tip(block)?;
            return Ok(true);
        }

        let pidx = self.index.read().get(&prev).cloned()
            .ok_or_else(|| anyhow!("orphan: prev khong biet"))?;
        let height = pidx.height + 1;
        if height <= last_checkpoint_height() {
            return Err(anyhow!("tu choi nhanh tai/duoi checkpoint"));
        }
        let expected = self.bits_for_prev(&prev, height);
        let mtp = self.mtp_branch(&prev);
        Self::validate_block_basic(block, height, expected, mtp)?;
        let work = target_add(&pidx.work, &work_from_bits(block.header.bits));
        let idx = Idx { prev, height, work, ts: block.header.timestamp, bits: block.header.bits };
        self.store_block_meta(block, &idx)?;

        if target_cmp(&work, &self.tip_work.read()) == std::cmp::Ordering::Greater {
            match self.switch_to(&hash) {
                Ok(()) => Ok(true),
                Err(_) => Ok(false),
            }
        } else {
            Ok(false)
        }
    }

    pub fn balance_for_script(&self, script: &[u8]) -> u64 {
        self.utxo.read().values()
            .filter(|o| o.script_pubkey == script)
            .map(|o| o.value).sum()
    }

    pub fn tip_height(&self) -> u64 { *self.height.read() }

    pub fn has_block(&self, h: &Hash) -> bool {
        self.index.read().contains_key(h)
    }

    pub fn get_block(&self, h: &Hash) -> Option<Block> {
        self.block_store.read().get(h).cloned()
    }

    pub fn tip_hash(&self) -> Hash { *self.tip.read() }

    pub fn block_at(&self, height: u64) -> Option<Block> {
        let hash = self.height_index.read().get(&height).map(|(h, _)| *h)?;
        self.block_store.read().get(&hash).cloned()
    }
}
