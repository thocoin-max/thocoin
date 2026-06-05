pub mod address;
pub mod encrypt;
pub mod totp;

use std::sync::Arc;
use parking_lot::RwLock;
use anyhow::{Result, anyhow};
use crate::wallet::address::{KeyPair, decode_address, script_p2pkh};
use crate::core::chain::ChainState;
use crate::core::tx::{Transaction, TxIn, TxOut};

pub struct Wallet {
    pub key: Arc<RwLock<KeyPair>>,
    pub path: String,
}

impl Wallet {
    fn passphrase() -> Option<String> {
        std::env::var("THOCOIN_WALLET_PASS").ok().filter(|s| !s.is_empty())
    }

    fn allow_plaintext() -> bool {
        std::env::var("THOCOIN_WALLET_ALLOW_PLAINTEXT").ok().as_deref() == Some("1")
    }

    fn persist(path: &str, mnemonic: &str) -> Result<()> {
        match Self::passphrase() {
            Some(pass) => {
                let blob = encrypt::encrypt(mnemonic.as_bytes(), &pass)?;
                std::fs::write(path, blob)?;
            }
            None => {

                if Self::allow_plaintext() {
                    eprintln!("[WALLET] CẢNH BÁO: seed được lưu PLAINTEXT (THOCOIN_WALLET_ALLOW_PLAINTEXT=1).");
                    std::fs::write(path, mnemonic)?;
                } else {
                    return Err(anyhow!(
                        "từ chối lưu ví plaintext: đặt THOCOIN_WALLET_PASS để mã hóa \
                         (hoặc THOCOIN_WALLET_ALLOW_PLAINTEXT=1 để ép lưu plaintext)"));
                }
            }
        }
        Ok(())
    }

    pub fn load_or_create(path: &str) -> Result<Self> {
        let key = match std::fs::read(path) {
            Ok(raw) => {
                let text = if encrypt::is_encrypted(&raw) {
                    let pass = Self::passphrase()
                        .ok_or_else(|| anyhow!("ví đã mã hóa: cần đặt THOCOIN_WALLET_PASS"))?;
                    let pt = encrypt::decrypt(&raw, &pass)?;
                    String::from_utf8(pt).map_err(|_| anyhow!("ví giải mã không hợp lệ"))?
                } else {
                    String::from_utf8(raw).map_err(|_| anyhow!("ví không phải UTF-8"))?
                };
                let s = text.trim();
                if s.split_whitespace().count() >= 12 {
                    KeyPair::from_mnemonic(s)?
                } else if let Ok(bytes) = hex::decode(s) {
                    KeyPair::from_bytes(&bytes)?
                } else {
                    let k = KeyPair::new();
                    Self::persist(path, k.mnemonic())?;
                    k
                }
            }
            Err(_) => {
                let k = KeyPair::new();
                Self::persist(path, k.mnemonic())?;
                k
            }
        };
        Ok(Wallet { key: Arc::new(RwLock::new(key)), path: path.into() })
    }

    pub fn replace_from_mnemonic(&self, phrase: &str) -> Result<()> {
        let k = KeyPair::from_mnemonic(phrase)?;
        Self::persist(&self.path, k.mnemonic())?;
        *self.key.write() = k;
        Ok(())
    }

    pub fn generate_new(&self) -> Result<()> {
        let k = KeyPair::new();
        Self::persist(&self.path, k.mnemonic())?;
        *self.key.write() = k;
        Ok(())
    }

    pub fn address(&self) -> String { self.key.read().address() }
    pub fn mnemonic(&self) -> String { self.key.read().mnemonic().to_string() }

    pub fn balance(&self, chain: &ChainState) -> u64 {
        chain.balance_for_script(&self.key.read().script_pubkey())
    }

    pub fn send(&self, chain: &ChainState, to: &str, amount: u64, fee: u64) -> Result<Transaction> {
        let to_hash = decode_address(to)?;
        let to_script = script_p2pkh(&to_hash);
        let my_script = self.key.read().script_pubkey();

        let utxo = chain.utxo.read();
        let mut inputs = Vec::new();
        let mut collected = 0u64;
        let need = amount.checked_add(fee).ok_or_else(|| anyhow!("amount+fee overflow"))?;
        for (op, out) in utxo.iter() {
            if out.script_pubkey == my_script {
                inputs.push(op.clone());
                collected += out.value;
                if collected >= need { break; }
            }
        }
        if collected < need { return Err(anyhow!("insufficient funds")); }
        drop(utxo);

        let tx_inputs: Vec<TxIn> = inputs.iter().map(|op| TxIn {
            prev: op.clone(),
            signature: vec![],
            pubkey: self.key.read().pubkey_bytes(),
            sequence: 0xffffffff,
        }).collect();

        let mut outputs = vec![TxOut { value: amount, script_pubkey: to_script }];
        let change = collected - need;
        if change > 0 {
            outputs.push(TxOut { value: change, script_pubkey: my_script });
        }

        let mut tx = Transaction {
            version: 1, inputs: tx_inputs, outputs, lock_time: 0,
        };
        let key = self.key.read();
        for vin in 0..tx.inputs.len() {
            let h = tx.sighash(vin);
            tx.inputs[vin].signature = key.sign(&h);
        }
        drop(key);

        Ok(tx)
    }
}
