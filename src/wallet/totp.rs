use base32::Alphabet;

pub fn verify(secret_b32: &str, code: &str) -> bool {
    let code = code.trim().replace(' ', "");
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let Some(bytes) = base32::decode(Alphabet::Rfc4648 { padding: false }, secret_b32) else {
        return false;
    };
    match totp_rs::TOTP::new(totp_rs::Algorithm::SHA1, 6, 2, 30, bytes) {
        Ok(t) => t.check_current(&code).unwrap_or(false),
        Err(_) => false,
    }
}

pub fn load_secret_beside(wallet_path: &str) -> Option<String> {
    let p = std::path::Path::new(wallet_path).parent()?.join("wallet.totp");
    let s = std::fs::read_to_string(p).ok()?;
    let s = s.trim();
    if s.is_empty() { None } else { Some(s.to_string()) }
}
