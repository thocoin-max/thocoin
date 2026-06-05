use rand::rngs::OsRng;
use anyhow::{Result, anyhow};
use bip39::{Mnemonic, Language};
use fips204::ml_dsa_44;
use fips204::traits::{KeyGen, SerDes, Signer, Verifier};
use crate::core::hash::{hash160, sha256d};
use crate::core::consensus::ADDRESS_PREFIX;

pub const PQ_PUBKEY_LEN: usize = 1312;
pub const PQ_SIG_LEN: usize = 2420;

pub struct KeyPair {
    pub seed: [u8; 32],
    pub public: ml_dsa_44::PublicKey,
    pub secret: ml_dsa_44::PrivateKey,
    pub mnemonic: String,
}

impl Drop for KeyPair {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.seed.zeroize();
        self.mnemonic.zeroize();
    }
}

impl KeyPair {
    pub fn new() -> Self {
        let mut entropy = [0u8; 16];
        use rand::RngCore;
        OsRng.fill_bytes(&mut entropy);
        let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy).unwrap();
        Self::from_mnemonic(&mnemonic.to_string()).unwrap()
    }

    pub fn from_mnemonic(phrase: &str) -> Result<Self> {
        let mnemonic = Mnemonic::parse_in(Language::English, phrase)?;
        let seed_full = mnemonic.to_seed("");
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&seed_full[..32]);
        let (public, secret) = ml_dsa_44::KG::keygen_from_seed(&seed);
        Ok(KeyPair { seed, public, secret, mnemonic: phrase.to_string() })
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() < 32 { return Err(anyhow!("seed too short")); }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&b[..32]);
        let (public, secret) = ml_dsa_44::KG::keygen_from_seed(&seed);
        Ok(KeyPair { seed, public, secret, mnemonic: String::new() })
    }

    pub fn pubkey_bytes(&self) -> Vec<u8> { self.public.clone().into_bytes().to_vec() }
    pub fn secret_bytes(&self) -> [u8; 32] { self.seed }
    pub fn mnemonic(&self) -> &str { &self.mnemonic }

    pub fn address(&self) -> String {
        let h = hash160(&self.pubkey_bytes());
        encode_address(&h)
    }

    pub fn script_pubkey(&self) -> Vec<u8> {
        let h = hash160(&self.pubkey_bytes());
        script_p2pkh(&h)
    }

    pub fn sign(&self, msg: &[u8; 32]) -> Vec<u8> {
        self.secret.try_sign(msg, &[]).expect("ml-dsa sign").to_vec()
    }
}

pub fn verify(pubkey: &[u8], msg: &[u8; 32], sig: &[u8]) -> bool {
    let Ok(pk_arr): Result<[u8; PQ_PUBKEY_LEN], _> = pubkey.try_into() else { return false };
    let Ok(sig_arr): Result<[u8; PQ_SIG_LEN], _> = sig.try_into() else { return false };
    let Ok(pk) = ml_dsa_44::PublicKey::try_from_bytes(pk_arr) else { return false };
    pk.verify(msg, &sig_arr, &[])
}

pub fn encode_address(h: &[u8; 20]) -> String {
    let mut payload = Vec::with_capacity(25);
    payload.push(ADDRESS_PREFIX);
    payload.extend_from_slice(h);
    let chk = sha256d(&payload);
    payload.extend_from_slice(&chk[..4]);
    bs58::encode(payload).into_string()
}

pub fn decode_address(addr: &str) -> Result<[u8; 20]> {
    let bytes = bs58::decode(addr).into_vec()?;
    if bytes.len() != 25 { anyhow::bail!("bad addr len"); }
    if bytes[0] != ADDRESS_PREFIX { anyhow::bail!("bad prefix"); }
    let chk = sha256d(&bytes[..21]);
    if chk[..4] != bytes[21..] { anyhow::bail!("bad checksum"); }
    let mut h = [0u8; 20];
    h.copy_from_slice(&bytes[1..21]);
    Ok(h)
}

pub fn script_p2pkh(h: &[u8; 20]) -> Vec<u8> {
    let mut s = Vec::with_capacity(25);
    s.push(0x76); s.push(0xa9); s.push(0x14);
    s.extend_from_slice(h);
    s.push(0x88); s.push(0xac);
    s
}

pub fn genesis_script() -> Vec<u8> { script_p2pkh(&[0u8; 20]) }
