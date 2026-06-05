#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::path::PathBuf;
use thocoin::core::chain::ChainState;
use thocoin::core::mempool::Mempool;
use thocoin::wallet::Wallet;
use thocoin::miner::Miner;
use thocoin::miner::gpu::GpuMiner;
use thocoin::net::P2P;
use thocoin::gui::App;

fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| dirs_home())
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("ThoCoin");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn asset_path(name: &str) -> PathBuf {
    let p = exe_dir().join("assets").join(name);
    if p.exists() { p } else { PathBuf::from("assets").join(name) }
}

fn load_icon() -> Arc<eframe::egui::IconData> {
    if let Ok(bytes) = std::fs::read(asset_path("logo.png")) {
        if let Ok(img) = image::load_from_memory(&bytes) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            return Arc::new(eframe::egui::IconData {
                rgba: rgba.into_raw(),
                width: w,
                height: h,
            });
        }
    }
    Arc::new(eframe::egui::IconData { rgba: vec![], width: 0, height: 0 })
}

fn show_error_dialog(msg: &str) {
    eprintln!("{}", msg);
    #[cfg(target_os = "windows")]
    unsafe {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        extern "system" {
            fn MessageBoxW(hwnd: *const std::ffi::c_void, text: *const u16, caption: *const u16, utype: u32) -> i32;
        }
        let text: Vec<u16> = OsStr::new(msg).encode_wide().chain(std::iter::once(0)).collect();
        let cap: Vec<u16> = OsStr::new("ThoCoin").encode_wide().chain(std::iter::once(0)).collect();
        MessageBoxW(std::ptr::null(), text.as_ptr(), cap.as_ptr(), 0x10);
    }
}

fn main() -> eframe::Result<()> {
    let data = data_dir();
    let chain_path = data.join("chain");
    let wallet_path = data.join("wallet.key");

    let chain = match ChainState::open(chain_path.to_str().unwrap()) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            let msg = format!(
                "ThoCoin is already running.\n\nClose the other window, then try again.\n\nDetails: {}",
                e
            );
            show_error_dialog(&msg);
            return Ok(());
        }
    };
    let wallet = Arc::new(Wallet::load_or_create(wallet_path.to_str().unwrap()).expect("wallet"));
    let mempool = Mempool::new();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let bg_threads = 1usize.max(threads / 8);
    let miner = Arc::new(Miner::new(chain.clone(), mempool.clone(), wallet.clone(), bg_threads));
    let gpu_miner = Arc::new(GpuMiner::new(chain.clone(), mempool.clone(), wallet.clone()));

    let rt = tokio::runtime::Runtime::new().unwrap();
    let p2p = Arc::new(P2P::new(chain.clone(), mempool.clone()));
    let p2p_clone = p2p.clone();
    std::thread::spawn(move || {
        rt.block_on(async move { let _ = p2p_clone.run().await; });
    });

    let size = [1180.0_f32, 760.0_f32];
    let opts = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size(size)
            .with_resizable(false)
            .with_maximize_button(false)
            .with_icon(load_icon())
            .with_title("ThoCoin — Wallet & Miner"),
        ..Default::default()
    };
    eframe::run_native(
        "ThoCoin",
        opts,
        Box::new(move |_cc| Box::new(App::new(chain, wallet, mempool, miner, gpu_miner))),
    )
}
