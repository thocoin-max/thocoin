use serde::{Serialize, Deserialize};
use crate::core::hash::{Hash, sha256d};

#[derive(Clone, Serialize, Deserialize, Debug, Eq, Hash, PartialEq)]
pub struct OutPoint { pub txid: Hash, pub vout: u32 }

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TxIn {
    pub prev: OutPoint,
    pub signature: Vec<u8>,
    pub pubkey: Vec<u8>,
    pub sequence: u32,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TxOut {
    pub value: u64,
    pub script_pubkey: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Transaction {
    pub version: u32,
    pub inputs: Vec<TxIn>,
    pub outputs: Vec<TxOut>,
    pub lock_time: u32,
}

impl Transaction {
    pub fn txid(&self) -> Hash {
        let mut stripped = self.clone();
        for inp in stripped.inputs.iter_mut() {
            inp.signature = Vec::new();
            inp.pubkey = Vec::new();
        }
        match bincode::serialize(&stripped) {
            Ok(bytes) => sha256d(&bytes),
            Err(_) => [0u8; 32],
        }
    }

    pub fn wtxid(&self) -> Hash {
        match bincode::serialize(self) {
            Ok(bytes) => sha256d(&bytes),
            Err(_) => [0u8; 32],
        }
    }

    pub fn sighash(&self, input_index: usize) -> Hash {
        let mut clone = self.clone();
        for (i, inp) in clone.inputs.iter_mut().enumerate() {
            inp.signature = Vec::new();
            if i != input_index {
                inp.pubkey = Vec::new();
            }
        }
        match bincode::serialize(&clone) {
            Ok(bytes) => sha256d(&bytes),
            Err(_) => [0u8; 32],
        }
    }

    pub fn is_coinbase(&self) -> bool {
        self.inputs.len() == 1 && self.inputs[0].prev.txid == [0u8; 32]
    }

    pub fn has_duplicate_inputs(&self) -> bool {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for inp in &self.inputs {
            if !seen.insert((inp.prev.txid, inp.prev.vout)) {
                return true;
            }
        }
        false
    }

    pub fn sigops(&self) -> usize {
        if self.is_coinbase() { 0 } else { self.inputs.len() }
    }

    pub fn size(&self) -> usize {
        bincode::serialize(self).map(|b| b.len()).unwrap_or(usize::MAX)
    }

    pub fn coinbase(height: u64, reward: u64, script_pubkey: Vec<u8>) -> Self {
        Transaction {
            version: 1,
            inputs: vec![TxIn {
                prev: OutPoint { txid: [0u8; 32], vout: u32::MAX },
                signature: height.to_le_bytes().to_vec(),
                pubkey: vec![],
                sequence: u32::MAX,
            }],
            outputs: vec![TxOut { value: reward, script_pubkey }],
            lock_time: height as u32,
        }
    }
}
