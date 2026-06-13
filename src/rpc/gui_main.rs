#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::path::PathBuf;
use std::io::Write;
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

fn log_path() -> PathBuf {
    data_dir().join("crash.log")
}

fn write_log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_path()) {
        let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
    }
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let loc = info.location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let payload = info.payload().downcast_ref::<&str>().map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        let msg = format!("PANIC at {}: {}", loc, payload);
        write_log(&msg);
        show_error_dialog(&format!(
            "ThoCoin crashed.\n\n{}\n\nLog: {}", msg, log_path().display()));
    }));
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
    write_log(msg);
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

#[cfg(target_os = "windows")]
fn focus_existing_window() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    extern "system" {
        fn FindWindowW(class: *const u16, window: *const u16) -> *mut std::ffi::c_void;
        fn SetForegroundWindow(hwnd: *mut std::ffi::c_void) -> i32;
        fn ShowWindow(hwnd: *mut std::ffi::c_void, cmd: i32) -> i32;
    }
    let title: Vec<u16> = OsStr::new("ThoCoin \u{2014} Wallet & Miner")
        .encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if !hwnd.is_null() {
            ShowWindow(hwnd, 9);
            SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(target_os = "windows")]
fn already_running() -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    extern "system" {
        fn CreateMutexW(attr: *const std::ffi::c_void, owner: i32, name: *const u16) -> *mut std::ffi::c_void;
        fn GetLastError() -> u32;
    }
    let name: Vec<u16> = OsStr::new("ThoCoin_SingleInstance_Mutex")
        .encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
        let err = GetLastError();
        std::mem::forget(h);
        err == 183
    }
}

#[cfg(not(target_os = "windows"))]
fn focus_existing_window() {}
#[cfg(not(target_os = "windows"))]
fn already_running() -> bool { false }

#[cfg(target_os = "windows")]
fn cleanup_stray_gl() {
    // Mesa opengl32.dll/libgallium must live in mesa\, never next to the exe.
    // Stray flat copies (left by old installers) get auto-loaded by Windows and break GL.
    // Remove them at startup, before any GL load, so the install self-heals.
    let dir = exe_dir();
    for name in ["opengl32.dll", "libgallium_wgl.dll", "libglapi.dll"] {
        let flat = dir.join(name);
        if flat.exists() {
            match std::fs::remove_file(&flat) {
                Ok(_) => write_log(&format!("cleanup: removed stray {}", name)),
                Err(e) => write_log(&format!("cleanup: cannot remove {} ({})", name, e)),
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn cleanup_stray_gl() {}

#[cfg(target_os = "windows")]
fn setup_software_gl() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    std::env::set_var("GALLIUM_DRIVER", "llvmpipe");
    std::env::set_var("MESA_GL_VERSION_OVERRIDE", "3.3");
    let mesa = exe_dir().join("mesa");
    let gl = mesa.join("opengl32.dll");
    if !gl.exists() {
        write_log("software_gl: mesa\\opengl32.dll missing, using system GL");
        return;
    }
    extern "system" {
        fn SetDllDirectoryW(path: *const u16) -> i32;
        fn LoadLibraryW(name: *const u16) -> *mut std::ffi::c_void;
    }
    let dir_w: Vec<u16> = OsStr::new(&mesa).encode_wide().chain(std::iter::once(0)).collect();
    unsafe { SetDllDirectoryW(dir_w.as_ptr()); }
    // Load dependencies first (full path) so Mesa opengl32 resolves already-loaded modules.
    for dep in ["libglapi.dll", "libgallium_wgl.dll", "opengl32.dll"] {
        let p = mesa.join(dep);
        if !p.exists() { continue; }
        let w: Vec<u16> = OsStr::new(&p).encode_wide().chain(std::iter::once(0)).collect();
        let h = unsafe { LoadLibraryW(w.as_ptr()) };
        write_log(&format!("software_gl: load {} -> {}", dep, !h.is_null()));
    }
}

#[cfg(not(target_os = "windows"))]
fn setup_software_gl() {}

fn main() -> eframe::Result<()> {
    install_panic_hook();
    write_log("=== start ===");
    cleanup_stray_gl();

    if already_running() {
        write_log("already_running -> focus & exit");
        focus_existing_window();
        return Ok(());
    }
    write_log("not running, continuing");

    let data = data_dir();
    let chain_path = data.join("chain");
    let wallet_path = data.join("wallet.key");

    let chain = match ChainState::open(chain_path.to_str().unwrap()) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            show_error_dialog(&format!(
                "ThoCoin could not open its data folder.\n\n\
                 Close any other ThoCoin window, or delete the 'chain' folder in %APPDATA%\\ThoCoin.\n\nDetails: {}", e));
            return Ok(());
        }
    };

    let wallet = match Wallet::load_or_create(wallet_path.to_str().unwrap()) {
        Ok(w) => Arc::new(w),
        Err(e) => {
            show_error_dialog(&format!("Wallet error: {}", e));
            return Ok(());
        }
    };

    let mempool = Mempool::new();
    let mode = thocoin::gui::MinerMode::detect();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let bg_threads = 1usize.max(threads / 8);
    let miner = Arc::new(Miner::new(chain.clone(), mempool.clone(), wallet.clone(), bg_threads));
    let gpu_miner = Arc::new(GpuMiner::new(chain.clone(), mempool.clone(), wallet.clone()));

    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            show_error_dialog(&format!("Tokio runtime error: {}", e));
            return Ok(());
        }
    };
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

    write_log("calling run_native");
    if matches!(mode, thocoin::gui::MinerMode::Cpu) {
        setup_software_gl();
    }
    let result = eframe::run_native(
        "ThoCoin",
        opts,
        Box::new(move |_cc| Box::new(App::new(chain, wallet, mempool, miner, gpu_miner, mode))),
    );
    write_log(&format!("run_native returned: {:?}", result.as_ref().map(|_| ())));

    if let Err(e) = &result {
        show_error_dialog(&format!(
            "Cannot start the graphics window.\n\n\
             This machine may lack OpenGL/GPU drivers (common on VMs or RDP).\n\n\
             Details: {}", e));
    }
    result
}