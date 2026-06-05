use anyhow::{Result, anyhow};
use argon2::{Argon2, Algorithm, Version, Params};
use chacha20poly1305::{XChaCha20Poly1305, XNonce, Key, aead::{Aead, KeyInit}};
use rand::RngCore;
use rand::rngs::OsRng;
use zeroize::Zeroize;

const MAGIC: &[u8; 4] = b"THWE";
const VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;

fn derive_key(passphrase: &[u8], salt: &[u8]) -> Result<[u8; 32]> {
    let params = Params::new(64 * 1024, 3, 1, Some(32))
        .map_err(|e| anyhow!("argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon.hash_password_into(passphrase, salt, &mut key)
        .map_err(|e| anyhow!("argon2 derive: {e}"))?;
    Ok(key)
}

pub fn encrypt(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);

    let mut key_bytes = derive_key(passphrase.as_bytes(), &salt)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ct = cipher.encrypt(nonce, plaintext)
        .map_err(|_| anyhow!("mã hóa thất bại"))?;
    key_bytes.zeroize();

    let mut out = Vec::with_capacity(4 + 1 + SALT_LEN + NONCE_LEN + ct.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn decrypt(blob: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    let header = 4 + 1 + SALT_LEN + NONCE_LEN;
    if blob.len() < header + 16 {
        return Err(anyhow!("file ví hỏng hoặc quá ngắn"));
    }
    if &blob[..4] != MAGIC {
        return Err(anyhow!("không phải file ví đã mã hóa"));
    }
    if blob[4] != VERSION {
        return Err(anyhow!("phiên bản ví không hỗ trợ"));
    }
    let salt = &blob[5..5 + SALT_LEN];
    let nonce_bytes = &blob[5 + SALT_LEN..header];
    let ct = &blob[header..];

    let mut key_bytes = derive_key(passphrase.as_bytes(), salt)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let nonce = XNonce::from_slice(nonce_bytes);
    let pt = cipher.decrypt(nonce, ct)
        .map_err(|_| anyhow!("sai mật khẩu hoặc dữ liệu bị sửa đổi"))?;
    key_bytes.zeroize();
    Ok(pt)
}

pub fn is_encrypted(blob: &[u8]) -> bool {
    blob.len() >= 4 && &blob[..4] == MAGIC
}
