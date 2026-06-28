use serde::{Serialize, Deserialize};
use crate::core::hash::{Hash, sha256d, hash_meets_target};
use crate::core::tx::Transaction;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct BlockHeader {
    pub version: u32,
    pub prev_hash: Hash,
    pub merkle_root: Hash,
    pub timestamp: u64,
    pub bits: u32,
    pub nonce: u32,
}

impl BlockHeader {

    pub fn serialize_80(&self) -> [u8; 80] {
        let mut buf = [0u8; 80];
        buf[0..4].copy_from_slice(&self.version.to_be_bytes());
        buf[4..36].copy_from_slice(&self.prev_hash);
        buf[36..68].copy_from_slice(&self.merkle_root);
        buf[68..72].copy_from_slice(&(self.timestamp as u32).to_be_bytes());
        buf[72..76].copy_from_slice(&self.bits.to_be_bytes());
        buf[76..80].copy_from_slice(&self.nonce.to_be_bytes());
        buf
    }

    pub fn hash(&self) -> Hash {
        sha256d(&self.serialize_80())
    }

    pub fn meets_pow(&self) -> bool {
        hash_meets_target(&self.hash(), self.bits)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
}

impl Block {
    pub fn merkle_root(txs: &[Transaction]) -> Hash {
        if txs.is_empty() { return [0u8; 32]; }
        let mut layer: Vec<Hash> = txs.iter().map(|t| t.txid()).collect();
        while layer.len() > 1 {
            if layer.len() % 2 != 0 {
                let last = *layer.last().unwrap();
                layer.push(last);
            }
            layer = layer.chunks(2).map(|c| {
                let mut buf = Vec::with_capacity(64);
                buf.extend_from_slice(&c[0]);
                buf.extend_from_slice(&c[1]);
                sha256d(&buf)
            }).collect();
        }
        layer[0]
    }

    pub fn new(prev_hash: Hash, txs: Vec<Transaction>, bits: u32, ts: u64) -> Self {
        let merkle = Block::merkle_root(&txs);
        Block {
            header: BlockHeader {
                version: 1,
                prev_hash,
                merkle_root: merkle,
                timestamp: ts,
                bits,
                nonce: 0,
            },
            transactions: txs,
        }
    }

    pub fn hash(&self) -> Hash { self.header.hash() }
}

/// Merkle root of wtxids; the coinbase counts as [0;32] since it carries the commitment.
pub fn witness_merkle_root(txs: &[Transaction]) -> Hash {
    if txs.is_empty() { return [0u8; 32]; }
    let mut layer: Vec<Hash> = txs.iter().enumerate()
        .map(|(i, t)| if i == 0 { [0u8; 32] } else { t.wtxid() })
        .collect();
    while layer.len() > 1 {
        if layer.len() % 2 != 0 {
            let last = *layer.last().unwrap();
            layer.push(last);
        }
        layer = layer.chunks(2).map(|c| {
            let mut buf = Vec::with_capacity(64);
            buf.extend_from_slice(&c[0]);
            buf.extend_from_slice(&c[1]);
            sha256d(&buf)
        }).collect();
    }
    layer[0]
}

pub fn commitment_script(root: &Hash) -> Vec<u8> {
    let mut s = Vec::with_capacity(34);
    s.push(0x6a); s.push(0x20);
    s.extend_from_slice(root);
    s
}

/// Append an OP_RETURN(witness merkle) output to the coinbase. Call BEFORE Block::new.
pub fn add_witness_commitment(txs: &mut Vec<Transaction>) {
    let root = witness_merkle_root(txs);
    if let Some(cb) = txs.first_mut() {
        cb.outputs.push(crate::core::tx::TxOut { value: 0, script_pubkey: commitment_script(&root) });
    }
}

pub fn check_witness_commitment(block: &Block) -> bool {
    let Some(cb) = block.transactions.first() else { return false };
    let Some(last) = cb.outputs.last() else { return false };
    if last.value != 0 || last.script_pubkey.len() != 34
        || last.script_pubkey[0] != 0x6a || last.script_pubkey[1] != 0x20 {
        return false;
    }
    last.script_pubkey[2..] == witness_merkle_root(&block.transactions)
}
