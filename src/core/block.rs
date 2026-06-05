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
