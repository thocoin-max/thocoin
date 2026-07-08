// Lightweight update checker. Runs once at GUI startup in a background thread.
// It queries two sources in order and reports the newest version found:
//   1. GitHub Releases API (latest tag)
//   2. https://www.thocoin.org/version.json  (fallback / override)
// The GUI shows a non-blocking banner if a newer version than CURRENT is found.
// Nothing is downloaded or installed automatically.

use std::sync::Arc;
use parking_lot::RwLock;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASES_URL: &str = "https://github.com/thocoin-max/thocoin/releases/latest";
pub const DOWNLOAD_URL: &str = "https://www.thocoin.org/download";

const GITHUB_API: &str = "https://api.github.com/repos/thocoin-max/thocoin/releases/latest";
const SITE_JSON: &str = "https://www.thocoin.org/version.json";

#[derive(Clone, Default)]
pub struct UpdateState {
    pub latest: Arc<RwLock<Option<String>>>,   // newest version string found, e.g. "1.1.0"
    pub url: Arc<RwLock<String>>,               // where to send the user to download
}

impl UpdateState {
    pub fn new() -> Self {
        Self {
            latest: Arc::new(RwLock::new(None)),
            url: Arc::new(RwLock::new(DOWNLOAD_URL.to_string())),
        }
    }

    /// True if a version newer than what we run was found.
    pub fn update_available(&self) -> Option<String> {
        let latest = self.latest.read().clone()?;
        if is_newer(&latest, CURRENT_VERSION) { Some(latest) } else { None }
    }

    pub fn download_url(&self) -> String { self.url.read().clone() }

    /// Kick off the check in a background thread. Never blocks the UI.
    pub fn spawn_check(&self) {
        let latest = self.latest.clone();
        let url = self.url.clone();
        let _ = &url; // download link is always DOWNLOAD_URL
        std::thread::spawn(move || {
            if let Some(v) = check_github() {
                *latest.write() = Some(v);
                return;
            }
            if let Some(v) = check_site() {
                *latest.write() = Some(v);
            }
        });
    }
}

fn http_get(url: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("thocoin-wallet")
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;
    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() { return None; }
    resp.text().ok()
}

fn check_github() -> Option<String> {
    let body = http_get(GITHUB_API)?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let tag = v.get("tag_name")?.as_str()?.trim_start_matches('v').to_string();
    Some(tag)
}

fn check_site() -> Option<String> {
    let body = http_get(SITE_JSON)?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let ver = v.get("version")?.as_str()?.trim_start_matches('v').to_string();
    Some(ver)
}

/// Semantic-ish compare: returns true if `a` > `b`. Non-numeric parts ignored.
fn is_newer(a: &str, b: &str) -> bool {
    let pa = parse_ver(a);
    let pb = parse_ver(b);
    pa > pb
}

fn parse_ver(s: &str) -> (u64, u64, u64) {
    let mut it = s.trim().trim_start_matches('v').split('.');
    let x = it.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let y = it.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let z = it.next()
        .map(|n| n.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    (x, y, z)
}
