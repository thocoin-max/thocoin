use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Instant, Duration};
use parking_lot::Mutex;
use eframe::egui;
use eframe::egui::{Color32, RichText, Stroke, Rounding, Margin, Vec2, FontId, FontFamily};
use qrcode::QrCode;
use crate::core::chain::ChainState;
use crate::core::mempool::Mempool;
use crate::core::consensus::COIN;
use crate::core::hash::{hash_to_hex, sha256d};
use crate::miner::Miner;
use crate::miner::gpu::GpuMiner;
use crate::wallet::Wallet;

#[derive(Debug, Clone)]
pub enum HistoryEntry {
    Tx { txid: String, amount: u64, is_received: bool, timestamp: u64, confirmed: bool, address: String },
    Mining { height: u64, block_hash: String, reward: u64, timestamp: u64 },
}
impl HistoryEntry {
    fn ts(&self) -> u64 { match self {
        HistoryEntry::Tx { timestamp, .. } => *timestamp,
        HistoryEntry::Mining { timestamp, .. } => *timestamp,
    }}
}

#[derive(Debug, Clone)]
struct Invoice { address: String, label: String, amount: u64, created: u64 }

#[derive(Debug, PartialEq, Clone, Copy)]
enum PassModal { None, AskSend, AskReveal, SetNew, ChangePass, AskCreateNew, AskDisable2FA }

#[derive(Debug, PartialEq, Clone, Copy)]
enum ChangeStep { OldPass, NewPass }

#[derive(Debug, PartialEq, Clone, Copy)]
enum Tab { Overview, Send, Receive, Transactions, Addresses, Coins, Contacts, Console, Mining }

#[derive(Clone, Copy)]
struct Theme {
    bg: Color32, sidebar: Color32, panel_alt: Color32, border: Color32, divider: Color32,
    text: Color32, text_dim: Color32, text_strong: Color32, text_muted: Color32,
    accent: Color32, link: Color32, orange: Color32,
    success: Color32, danger: Color32, warn: Color32,
    input_bg: Color32, sidebar_selected: Color32, topbar: Color32,
}
impl Theme {
    fn electrum() -> Self {
        Theme {
            bg: Color32::from_rgb(24, 27, 31),
            sidebar: Color32::from_rgb(18, 20, 24),
            panel_alt: Color32::from_rgb(32, 36, 42),
            border: Color32::from_rgb(46, 50, 58),
            divider: Color32::from_rgb(38, 42, 48),
            text: Color32::from_rgb(218, 222, 230),
            text_dim: Color32::from_rgb(140, 146, 158),
            text_strong: Color32::from_rgb(245, 247, 250),
            text_muted: Color32::from_rgb(105, 110, 120),
            accent: Color32::from_rgb(50, 130, 246),
            link: Color32::from_rgb(80, 155, 250),
            orange: Color32::from_rgb(242, 169, 0),
            success: Color32::from_rgb(60, 200, 130),
            danger: Color32::from_rgb(235, 100, 100),
            warn: Color32::from_rgb(242, 169, 0),
            input_bg: Color32::from_rgb(32, 35, 41),
            sidebar_selected: Color32::from_rgb(36, 40, 48),
            topbar: Color32::from_rgb(18, 20, 24),
        }
    }
}

pub struct App {
    pub chain: Arc<ChainState>,
    pub wallet: Arc<Wallet>,
    pub mempool: Mempool,
    pub miner: Arc<Miner>,
    pub gpu_miner: Arc<GpuMiner>,

    theme: Theme,
    tab: Tab,

    to_address: String,
    amount: String,
    label: String,
    fee_rate: String,

    request_label: String,
    request_amount: String,
    invoices: Vec<Invoice>,

    contacts: Vec<(String, String)>,
    contact_name: String,
    contact_addr: String,

    console_input: String,
    console_log: Vec<String>,

    status: Option<(String, bool, Instant)>,
    last_supply_seen: u64,
    last_height_seen: u64,
    history: Arc<Mutex<Vec<HistoryEntry>>>,
    tx_filter: String,

    mnemonic_input: String,
    mnemonic_display: String,
    show_mnemonic: bool,
    receive_addresses: Vec<(String, String)>,

    qr_texture: Option<egui::TextureHandle>,
    qr_for_address: String,

    last_hr_cpu_t: Instant, last_hr_cpu_v: u64, cpu_hashrate: f64,
    last_hr_gpu_t: Instant, last_hr_gpu_v: u64, gpu_hashrate: f64,

    threads: usize,
    use_cpu: bool,
    use_gpu: bool,

    password_hash: Option<[u8; 32]>,
    pass_modal: PassModal,
    pass_input: String,
    pass_input2: String,
    pass_input3: String,
    pass_err: String,
    change_step: ChangeStep,
    seed_acknowledged: bool,
    seed_ack_checkbox: bool,
    totp_secret: Option<String>,
    totp_input: String,
    totp_setup_secret: Option<String>,
    totp_setup_qr: Option<egui::TextureHandle>,
    totp_setup_open: bool,
    totp_setup_step: u8,

    logo_texture: Option<egui::TextureHandle>,

    style_applied: bool,
}

impl App {
    pub fn new(
        chain: Arc<ChainState>, wallet: Arc<Wallet>, mempool: Mempool,
        miner: Arc<Miner>, gpu_miner: Arc<GpuMiner>,
    ) -> Self {
        let mnemonic_display = wallet.mnemonic();
        let supply = *chain.supply.read();
        let height = *chain.height.read();
        let addr = wallet.address();
        App {
            chain, wallet, mempool, miner, gpu_miner,
            theme: Theme::electrum(),
            tab: Tab::Overview,
            to_address: String::new(), amount: String::new(), label: String::new(),
            fee_rate: "0.00001000".into(),
            request_label: String::new(), request_amount: String::new(),
            invoices: Vec::new(),
            contacts: Vec::new(),
            contact_name: String::new(), contact_addr: String::new(),
            console_input: String::new(),
            console_log: vec!["ThoCoin console v0.1.0".into(), "Type 'help' for commands.".into()],
            status: None,
            last_supply_seen: supply, last_height_seen: height,
            history: Arc::new(Mutex::new(Vec::new())),
            tx_filter: String::new(),
            mnemonic_input: String::new(), mnemonic_display, show_mnemonic: false,
            receive_addresses: vec![("(default)".into(), addr)],
            qr_texture: None, qr_for_address: String::new(),
            last_hr_cpu_t: Instant::now(), last_hr_cpu_v: 0, cpu_hashrate: 0.0,
            last_hr_gpu_t: Instant::now(), last_hr_gpu_v: 0, gpu_hashrate: 0.0,
            threads: num_cpus().max(1),
            use_cpu: true, use_gpu: false,
            password_hash: load_password_hash(),
            pass_modal: PassModal::None,
            pass_input: String::new(),
            pass_input2: String::new(),
            pass_input3: String::new(),
            pass_err: String::new(),
            change_step: ChangeStep::OldPass,
            seed_acknowledged: load_password_hash().is_some(),
            seed_ack_checkbox: false,
            totp_secret: load_totp_secret(),
            totp_input: String::new(),
            totp_setup_secret: None,
            totp_setup_qr: None,
            totp_setup_open: false,
            totp_setup_step: 0,
            logo_texture: None,
            style_applied: false,
        }
    }

    fn apply_style(&mut self, ctx: &egui::Context) {
        if self.style_applied { return; }
        let t = self.theme;
        let mut style = (*ctx.style()).clone();
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = Some(t.text);
        v.window_fill = t.bg; v.panel_fill = t.bg;
        v.faint_bg_color = t.panel_alt;
        v.extreme_bg_color = t.input_bg;
        v.code_bg_color = t.input_bg;
        v.window_stroke = Stroke::new(1.0, t.border);
        v.window_rounding = Rounding::same(6.0);
        v.menu_rounding = Rounding::same(6.0);
        v.selection.bg_fill = Color32::from_rgba_unmultiplied(50, 130, 246, 90);
        v.selection.stroke = Stroke::new(1.0, t.accent);
        v.hyperlink_color = t.link;
        v.widgets.noninteractive.bg_fill = t.bg;
        v.widgets.noninteractive.weak_bg_fill = t.bg;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
        v.widgets.noninteractive.rounding = Rounding::same(6.0);
        v.widgets.inactive.bg_fill = t.input_bg;
        v.widgets.inactive.weak_bg_fill = t.input_bg;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.border);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
        v.widgets.inactive.rounding = Rounding::same(6.0);
        v.widgets.hovered.bg_fill = t.panel_alt;
        v.widgets.hovered.weak_bg_fill = t.panel_alt;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.accent);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, t.text_strong);
        v.widgets.hovered.rounding = Rounding::same(6.0);
        v.widgets.active.bg_fill = t.accent;
        v.widgets.active.weak_bg_fill = t.accent;
        v.widgets.active.bg_stroke = Stroke::new(1.0, t.accent);
        v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        v.widgets.active.rounding = Rounding::same(6.0);
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 7.0);
        style.spacing.window_margin = Margin::same(0.0);
        style.spacing.indent = 14.0;
        style.spacing.scroll.bar_width = 8.0;
        ctx.set_style(style);
        self.style_applied = true;
    }

    fn poll(&mut self) {
        let height = *self.chain.height.read();
        let supply = *self.chain.supply.read();
        if height > self.last_height_seen {

            let mut new_blocks: Vec<(crate::core::block::Block, u64)> = self.chain.headers.read()
                .values()
                .filter(|(_, h)| *h > self.last_height_seen && *h <= height)
                .cloned()
                .collect();
            new_blocks.sort_by_key(|(_, h)| *h);

            let my_script = self.wallet.key.read().script_pubkey();
            let mut hist = self.history.lock();

            for (blk, blk_h) in &new_blocks {

                if !blk.transactions.is_empty() && blk.transactions[0].is_coinbase() {
                    let cb_reward: u64 = blk.transactions[0].outputs.iter()
                        .filter(|o| o.script_pubkey == my_script)
                        .map(|o| o.value).sum();
                    if cb_reward > 0 {
                        hist.push(HistoryEntry::Mining {
                            height: *blk_h,
                            block_hash: hash_to_hex(&blk.hash()),
                            reward: cb_reward,
                            timestamp: blk.header.timestamp,
                        });
                    }
                }

                for tx in blk.transactions.iter().skip(1) {
                    let txid = tx.txid();
                    let txid_hex = hash_to_hex(&txid);
                    let mut found_sent = false;
                    for e in hist.iter_mut() {
                        if let HistoryEntry::Tx { txid: tid, confirmed, is_received, .. } = e {
                            if *tid == txid_hex {
                                *confirmed = true;
                                if !*is_received { found_sent = true; }
                            }
                        }
                    }
                    if !found_sent {
                        let received: u64 = tx.outputs.iter()
                            .filter(|o| o.script_pubkey == my_script)
                            .map(|o| o.value).sum();
                        if received > 0 {
                            hist.push(HistoryEntry::Tx {
                                txid: txid_hex,
                                amount: received,
                                is_received: true,
                                timestamp: blk.header.timestamp,
                                confirmed: true,
                                address: String::new(),
                            });
                        }
                    }
                }
            }
            self.last_height_seen = height;
            self.last_supply_seen = supply;
        }
        let now = Instant::now();
        let dc = now.duration_since(self.last_hr_cpu_t);
        if dc >= Duration::from_secs(2) {
            let cur = self.miner.hashrate.load(Ordering::Relaxed);
            self.cpu_hashrate = cur.saturating_sub(self.last_hr_cpu_v) as f64 / dc.as_secs_f64();
            self.last_hr_cpu_v = cur; self.last_hr_cpu_t = now;
        }
        let dg = now.duration_since(self.last_hr_gpu_t);
        if dg >= Duration::from_secs(2) {
            let cur = self.gpu_miner.hashrate.load(Ordering::Relaxed);
            self.gpu_hashrate = cur.saturating_sub(self.last_hr_gpu_v) as f64 / dg.as_secs_f64();
            self.last_hr_gpu_v = cur; self.last_hr_gpu_t = now;
        }
        if let Some((_, _, t)) = &self.status {
            if t.elapsed() > Duration::from_secs(4) { self.status = None; }
        }
    }

    fn reset_history_for_new_wallet(&mut self) {
        self.history.lock().clear();
        self.last_height_seen = 0;
        self.last_supply_seen = 0;
        self.qr_texture = None;
        self.qr_for_address = String::new();
        delete_password_file();
        delete_totp();
        self.password_hash = None;
        self.show_mnemonic = false;
        self.seed_acknowledged = false;
        self.seed_ack_checkbox = false;
        self.totp_secret = None;
        self.totp_setup_open = false;
    }

    fn open_pass_modal(&mut self, kind: PassModal) {
        self.pass_modal = kind;
        self.pass_input.clear();
        self.pass_input2.clear();
        self.pass_input3.clear();
        self.totp_input.clear();
        self.pass_err.clear();
        self.change_step = ChangeStep::OldPass;
    }

    fn close_pass_modal(&mut self) {
        self.pass_modal = PassModal::None;
        self.pass_input.clear();
        self.pass_input2.clear();
        self.pass_input3.clear();
        self.totp_input.clear();
        self.pass_err.clear();
        self.change_step = ChangeStep::OldPass;
    }

    fn verify_pass(&self) -> bool {
        match self.password_hash {
            Some(h) => hash_password(&self.pass_input) == h,
            None => true,
        }
    }

    fn seed_backup_ui(&mut self, ctx: &egui::Context) {
        let t = self.theme;
        let mnemonic = self.wallet.mnemonic();
        let words: Vec<&str> = mnemonic.split_whitespace().collect();

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(t.bg).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
                let avail = ui.available_size();
                ui.allocate_ui_with_layout(avail,
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            ui.add_space(24.0);
                            if let Some(tex) = &self.logo_texture {
                                let sized = egui::load::SizedTexture::new(tex.id(), Vec2::new(72.0, 72.0));
                                ui.add(egui::Image::from_texture(sized).rounding(Rounding::same(36.0)));
                            }
                            ui.add_space(14.0);
                            ui.label(RichText::new("Backup your recovery phrase").size(22.0).strong().color(t.text_strong));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Write these 12 words down on paper and keep them somewhere safe.")
                                .size(13.0).color(t.text_dim));
                            ui.label(RichText::new("This is the ONLY way to recover your wallet and reset your password.")
                                .size(13.0).color(t.warn));
                            ui.add_space(20.0);

                            ui.allocate_ui(Vec2::new(560.0, 0.0), |ui| {
                                egui::Frame::none()
                                    .fill(t.panel_alt)
                                    .stroke(Stroke::new(1.0, t.border))
                                    .rounding(Rounding::same(10.0))
                                    .inner_margin(Margin::same(18.0))
                                    .show(ui, |ui| {
                                        let cols = 3usize;
                                        let cell_w = 170.0_f32;
                                        for chunk in words.chunks(cols) {
                                            ui.horizontal(|ui| {
                                                for (i, w) in chunk.iter().enumerate() {
                                                    let idx = (chunk.as_ptr() as usize - words.as_ptr() as usize)
                                                        / std::mem::size_of::<&str>() + i;
                                                    egui::Frame::none()
                                                        .fill(t.input_bg)
                                                        .stroke(Stroke::new(1.0, t.border))
                                                        .rounding(Rounding::same(6.0))
                                                        .inner_margin(Margin::symmetric(10.0, 8.0))
                                                        .show(ui, |ui| {
                                                            ui.set_min_width(cell_w);
                                                            ui.horizontal(|ui| {
                                                                ui.label(RichText::new(format!("{:>2}.", idx + 1))
                                                                    .size(11.5).color(t.text_muted).family(FontFamily::Monospace));
                                                                ui.add_space(4.0);
                                                                ui.label(RichText::new(*w).size(13.0).strong()
                                                                    .color(t.text_strong).family(FontFamily::Monospace));
                                                            });
                                                        });
                                                }
                                            });
                                            ui.add_space(6.0);
                                        }
                                    });
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    let mnemonic_clone = mnemonic.clone();
                                    if ghost_btn(ui, &t, "⧉ Copy phrase").clicked() {
                                        ui.output_mut(|o| o.copied_text = mnemonic_clone);
                                        self.notify("Copied", false);
                                    }
                                });
                            });

                            ui.add_space(22.0);
                            ui.allocate_ui(Vec2::new(560.0, 0.0), |ui| {
                                ui.checkbox(&mut self.seed_ack_checkbox,
                                    RichText::new("I have written down my 12 words and stored them safely")
                                        .size(13.0).color(t.text));
                            });
                            ui.add_space(14.0);
                            ui.allocate_ui(Vec2::new(560.0, 0.0), |ui| {
                                let resp = ui.add_enabled(self.seed_ack_checkbox,
                                    egui::Button::new(RichText::new("Continue to wallet")
                                        .size(14.0).color(Color32::WHITE).strong())
                                        .fill(t.accent).rounding(Rounding::same(6.0))
                                        .min_size(Vec2::new(560.0, 46.0)));
                                if resp.clicked() {
                                    self.seed_acknowledged = true;
                                    self.seed_ack_checkbox = false;
                                }
                            });
                            ui.add_space(30.0);
                        });
                    });
            });
    }

    fn lock_screen_ui(&mut self, ctx: &egui::Context) {
        let t = self.theme;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(t.bg).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
                let avail = ui.available_size();
                ui.allocate_ui_with_layout(avail,
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);

                            if let Some(tex) = &self.logo_texture {
                                let sized = egui::load::SizedTexture::new(tex.id(), Vec2::new(96.0, 96.0));
                                ui.add(egui::Image::from_texture(sized).rounding(Rounding::same(48.0)));
                            }
                            ui.add_space(20.0);
                            ui.label(RichText::new("ThoCoin Wallet").size(26.0).strong().color(t.text_strong));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Set a password to protect this wallet").size(13.0).color(t.text_dim));
                            ui.add_space(36.0);

                            ui.allocate_ui(Vec2::new(380.0, 0.0), |ui| {
                                ui.label(RichText::new("New password").size(12.5).color(t.text_dim));
                                ui.add_space(6.0);
                                ui.add(egui::TextEdit::singleline(&mut self.pass_input2)
                                    .password(true).desired_width(f32::INFINITY)
                                    .hint_text("min 4 characters")
                                    .margin(Vec2::new(12.0, 12.0)));
                                ui.add_space(14.0);
                                ui.label(RichText::new("Confirm password").size(12.5).color(t.text_dim));
                                ui.add_space(6.0);
                                ui.add(egui::TextEdit::singleline(&mut self.pass_input3)
                                    .password(true).desired_width(f32::INFINITY)
                                    .margin(Vec2::new(12.0, 12.0)));

                                if !self.pass_err.is_empty() {
                                    ui.add_space(10.0);
                                    ui.colored_label(t.danger, RichText::new(&self.pass_err).size(12.0));
                                }
                                ui.add_space(20.0);
                                if ui.add(egui::Button::new(RichText::new("Set password & continue")
                                        .size(13.5).color(Color32::WHITE).strong())
                                    .fill(t.accent).rounding(Rounding::same(6.0))
                                    .min_size(Vec2::new(380.0, 44.0))).clicked()
                                {
                                    if self.pass_input2.is_empty() {
                                        self.pass_err = "Password cannot be empty".into();
                                    } else if self.pass_input2.len() < 4 {
                                        self.pass_err = "Password too short (min 4 chars)".into();
                                    } else if self.pass_input2 != self.pass_input3 {
                                        self.pass_err = "Passwords do not match".into();
                                    } else {
                                        let h = hash_password(&self.pass_input2);
                                        if save_password_hash(&h).is_ok() {
                                            self.password_hash = Some(h);
                                            self.pass_input2.clear();
                                            self.pass_input3.clear();
                                            self.pass_err.clear();
                                            self.notify("Wallet protected", false);
                                        } else {
                                            self.pass_err = "Failed to save password".into();
                                        }
                                    }
                                }
                            });
                        });
                    });
            });
    }

    fn pass_modal_ui(&mut self, ctx: &egui::Context) {
        if self.pass_modal == PassModal::None { return; }
        let t = self.theme;
        let kind = self.pass_modal;
        let title = match kind {
            PassModal::AskSend => "Confirm — Send transaction",
            PassModal::AskReveal => "Confirm — Reveal recovery phrase",
            PassModal::AskCreateNew => "Confirm — Create new wallet",
            PassModal::AskDisable2FA => "Confirm — Disable 2FA",
            PassModal::SetNew => "Set wallet password",
            PassModal::ChangePass => match self.change_step {
                ChangeStep::OldPass => "Change password — Step 1/2: enter current password",
                ChangeStep::NewPass => "Change password — Step 2/2: enter new password",
            },
            PassModal::None => return,
        };

        let screen = ctx.screen_rect();
        egui::Area::new("pass_modal_bg".into())
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let painter = ui.painter();
                painter.rect_filled(screen, Rounding::ZERO,
                    Color32::from_rgba_unmultiplied(0, 0, 0, 60));
            });

        egui::Window::new(title)
            .collapsible(false).resizable(false).movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.style())
                .fill(t.panel_alt)
                .stroke(Stroke::new(1.0, t.border))
                .inner_margin(Margin::same(22.0))
                .rounding(Rounding::same(10.0)))
            .show(ctx, |ui| {
                ui.set_min_width(380.0);

                if matches!(kind, PassModal::AskCreateNew) {
                    ui.label(RichText::new("Create a new wallet? Your current wallet will be REPLACED. If you have not backed up the recovery phrase, you will permanently LOSE access to this balance.")
                        .size(12.0).color(t.danger));
                    ui.add_space(12.0);
                }

                let show_new = matches!(kind, PassModal::SetNew)
                    || (matches!(kind, PassModal::ChangePass) && self.change_step == ChangeStep::NewPass);

                if show_new {
                    ui.label(RichText::new("New password").size(12.5).color(t.text_dim));
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(&mut self.pass_input2)
                        .password(true).desired_width(f32::INFINITY)
                        .margin(Vec2::new(12.0, 10.0)));
                    ui.add_space(12.0);
                    ui.label(RichText::new("Confirm new password").size(12.5).color(t.text_dim));
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(&mut self.pass_input3)
                        .password(true).desired_width(f32::INFINITY)
                        .margin(Vec2::new(12.0, 10.0)));
                } else {
                    let lbl = if matches!(kind, PassModal::ChangePass) { "Current password" } else { "Password" };
                    ui.label(RichText::new(lbl).size(12.5).color(t.text_dim));
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(&mut self.pass_input)
                        .password(true).desired_width(f32::INFINITY)
                        .margin(Vec2::new(12.0, 10.0)));

                    if matches!(kind, PassModal::AskSend | PassModal::AskReveal | PassModal::AskCreateNew | PassModal::AskDisable2FA)
                        && self.totp_secret.is_some()
                    {
                        ui.add_space(12.0);
                        ui.label(RichText::new("2FA code (6 digits)").size(12.5).color(t.text_dim));
                        ui.add_space(6.0);
                        ui.add(egui::TextEdit::singleline(&mut self.totp_input)
                            .desired_width(f32::INFINITY)
                            .hint_text("123 456")
                            .font(egui::TextStyle::Monospace)
                            .margin(Vec2::new(12.0, 10.0)));
                    }
                }

                if !self.pass_err.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(&self.pass_err).size(12.0).color(t.danger));
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ok_label = if matches!(kind, PassModal::ChangePass) && self.change_step == ChangeStep::OldPass {
                            "Next"
                        } else if matches!(kind, PassModal::AskCreateNew) {
                            "Yes, create"
                        } else if matches!(kind, PassModal::AskDisable2FA) {
                            "Disable 2FA"
                        } else { "OK" };
                        if ui.add(egui::Button::new(RichText::new(ok_label).color(Color32::WHITE).strong())
                            .fill(t.accent).rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(96.0, 34.0))).clicked() {
                            self.confirm_pass(kind);
                        }
                        ui.add_space(8.0);
                        if ui.add(egui::Button::new(RichText::new("Cancel").color(t.text))
                            .fill(t.input_bg).stroke(Stroke::new(1.0, t.border))
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(96.0, 34.0))).clicked() {
                            self.close_pass_modal();
                        }
                    });
                });
            });
    }

    fn confirm_pass(&mut self, kind: PassModal) {
        match kind {
            PassModal::AskSend => {
                if !self.verify_pass() {
                    self.pass_err = "Incorrect password".into(); return;
                }
                if let Some(secret) = self.totp_secret.clone() {
                    if !verify_totp(&secret, &self.totp_input) {
                        self.pass_err = "Invalid 2FA code".into(); return;
                    }
                }
                self.close_pass_modal();
                self.execute_send();
            }
            PassModal::AskReveal => {
                if !self.verify_pass() {
                    self.pass_err = "Incorrect password".into(); return;
                }
                if let Some(secret) = self.totp_secret.clone() {
                    if !verify_totp(&secret, &self.totp_input) {
                        self.pass_err = "Invalid 2FA code".into(); return;
                    }
                }
                self.show_mnemonic = true;
                self.close_pass_modal();
            }
            PassModal::SetNew => {
                if self.pass_input2.is_empty() {
                    self.pass_err = "Password cannot be empty".into(); return;
                }
                if self.pass_input2.len() < 4 {
                    self.pass_err = "Password too short (min 4 chars)".into(); return;
                }
                if self.pass_input2 != self.pass_input3 {
                    self.pass_err = "Passwords do not match".into(); return;
                }
                let h = hash_password(&self.pass_input2);
                if save_password_hash(&h).is_ok() {
                    self.password_hash = Some(h);
                    self.close_pass_modal();
                    self.notify("Password set", false);
                } else {
                    self.pass_err = "Failed to save password".into();
                }
            }
            PassModal::ChangePass => {
                match self.change_step {
                    ChangeStep::OldPass => {
                        if !self.verify_pass() {
                            self.pass_err = "Incorrect current password".into();
                            return;
                        }
                        self.change_step = ChangeStep::NewPass;
                        self.pass_err.clear();
                    }
                    ChangeStep::NewPass => {
                        if self.pass_input2.is_empty() {
                            self.pass_err = "New password cannot be empty".into(); return;
                        }
                        if self.pass_input2.len() < 4 {
                            self.pass_err = "Password too short (min 4 chars)".into(); return;
                        }
                        if self.pass_input2 != self.pass_input3 {
                            self.pass_err = "Passwords do not match".into(); return;
                        }
                        if Some(hash_password(&self.pass_input2)) == self.password_hash {
                            self.pass_err = "New password must differ from current".into(); return;
                        }
                        let h = hash_password(&self.pass_input2);
                        if save_password_hash(&h).is_ok() {
                            self.password_hash = Some(h);
                            self.close_pass_modal();
                            self.notify("Password changed", false);
                        } else {
                            self.pass_err = "Failed to save password".into();
                        }
                    }
                }
            }
            PassModal::AskDisable2FA => {
                if !self.verify_pass() {
                    self.pass_err = "Incorrect password".into(); return;
                }
                if let Some(secret) = self.totp_secret.clone() {
                    if !verify_totp(&secret, &self.totp_input) {
                        self.pass_err = "Invalid 2FA code".into(); return;
                    }
                }
                delete_totp();
                self.totp_secret = None;
                self.close_pass_modal();
                self.notify("2FA disabled", false);
            }
            PassModal::AskCreateNew => {
                if !self.verify_pass() {
                    self.pass_err = "Incorrect password".into(); return;
                }
                if let Some(secret) = self.totp_secret.clone() {
                    if !verify_totp(&secret, &self.totp_input) {
                        self.pass_err = "Invalid 2FA code".into(); return;
                    }
                }
                self.close_pass_modal();
                if self.wallet.generate_new().is_ok() {
                    let new_addr = self.wallet.address();
                    self.receive_addresses = vec![("(default)".into(), new_addr)];
                    self.mnemonic_display = self.wallet.mnemonic();
                    self.reset_history_for_new_wallet();
                    self.notify("New wallet created", false);
                } else {
                    self.notify("Failed to create wallet", true);
                }
            }
            PassModal::None => {}
        }
    }

    fn notify(&mut self, msg: impl Into<String>, err: bool) {
        self.status = Some((msg.into(), err, Instant::now()));
    }

    fn ensure_qr(&mut self, ctx: &egui::Context, address: &str) {
        if self.qr_for_address == address && self.qr_texture.is_some() { return; }
        let Ok(qr) = QrCode::new(address.as_bytes()) else { return };
        let modules = qr.to_colors();
        let width = qr.width();
        let scale = 6usize;
        let pad = 3usize;
        let total = width + pad * 2;
        let img_size = total * scale;
        let mut pixels = vec![255u8; img_size * img_size * 4];
        for y in 0..width {
            for x in 0..width {
                if modules[y * width + x] == qrcode::Color::Dark {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = (x + pad) * scale + dx;
                            let py = (y + pad) * scale + dy;
                            let idx = (py * img_size + px) * 4;
                            pixels[idx] = 0; pixels[idx + 1] = 0; pixels[idx + 2] = 0; pixels[idx + 3] = 255;
                        }
                    }
                }
            }
        }
        let color_img = egui::ColorImage::from_rgba_unmultiplied([img_size, img_size], &pixels);
        self.qr_texture = Some(ctx.load_texture("addr_qr", color_img, egui::TextureOptions::NEAREST));
        self.qr_for_address = address.to_string();
    }
}

fn num_cpus() -> usize { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2) }
fn fmt_coin(v: u64) -> String {
    let whole = v / COIN; let frac = v % COIN;
    let s = format!("{}", whole);
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 { out.insert(0, ','); }
        out.insert(0, c);
    }
    format!("{}.{:08}", out, frac)
}
fn fmt_hashrate(hps: f64) -> String {
    if hps >= 1e9 { format!("{:.2} GH/s", hps / 1e9) }
    else if hps >= 1e6 { format!("{:.2} MH/s", hps / 1e6) }
    else if hps >= 1e3 { format!("{:.2} KH/s", hps / 1e3) }
    else { format!("{:.0} H/s", hps) }
}
fn fmt_ts(ts: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(ts as i64, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}
fn short_addr(s: &str) -> String {
    if s.len() <= 24 { return s.to_string(); }
    format!("{}...{}", &s[..14], &s[s.len()-6..])
}
fn short_id(s: &str) -> String {
    if s.len() <= 14 { return s.to_string(); }
    format!("{}...{}", &s[..6], &s[s.len()-6..])
}

fn icon_btn(ui: &mut egui::Ui, t: &Theme, icon: &str, tip: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(icon).size(14.0).color(t.text_dim))
        .fill(Color32::TRANSPARENT).stroke(Stroke::NONE)
        .min_size(Vec2::new(30.0, 30.0)).rounding(Rounding::same(5.0)))
        .on_hover_text(tip)
}
fn primary_btn(ui: &mut egui::Ui, t: &Theme, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(label).size(13.0).color(Color32::WHITE).strong())
        .fill(t.accent).stroke(Stroke::NONE)
        .rounding(Rounding::same(6.0)).min_size(Vec2::new(0.0, 36.0)))
}
fn ghost_btn(ui: &mut egui::Ui, t: &Theme, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(label).size(12.0).color(t.text))
        .fill(t.input_bg).stroke(Stroke::new(1.0, t.border))
        .rounding(Rounding::same(6.0)).min_size(Vec2::new(0.0, 30.0)))
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_style(ctx);
        ctx.request_repaint_after(Duration::from_millis(500));

        if self.password_hash.is_none() {
            self.ensure_logo(ctx);
            self.lock_screen_ui(ctx);
            return;
        }

        if !self.seed_acknowledged {
            self.ensure_logo(ctx);
            self.seed_backup_ui(ctx);
            return;
        }

        self.poll();

        egui::TopBottomPanel::top("topbar")
            .exact_height(56.0)
            .frame(egui::Frame::none().fill(self.theme.topbar).inner_margin(Margin::symmetric(16.0, 10.0)))
            .show(ctx, |ui| self.topbar_ui(ui));

        egui::TopBottomPanel::bottom("statusbar")
            .exact_height(32.0)
            .frame(egui::Frame::none().fill(self.theme.topbar).inner_margin(Margin::symmetric(16.0, 6.0)))
            .show(ctx, |ui| self.statusbar_ui(ui));

        egui::SidePanel::left("sidebar")
            .resizable(false).exact_width(190.0)
            .show_separator_line(false)
            .frame(egui::Frame::none().fill(self.theme.sidebar).inner_margin(Margin {
                left: 14.0, right: 14.0, top: 16.0, bottom: 16.0,
            }))
            .show(ctx, |ui| self.sidebar_ui(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.theme.bg).inner_margin(Margin {
                left: 28.0, right: 28.0, top: 22.0, bottom: 22.0,
            }))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Frame::none()
                        .inner_margin(Margin { left: 6.0, right: 14.0, top: 4.0, bottom: 14.0 })
                        .show(ui, |ui| {
                            match self.tab {
                                Tab::Overview     => self.overview_ui(ui),
                                Tab::Send         => self.send_ui(ui),
                                Tab::Receive      => { let c = ui.ctx().clone(); self.receive_full_ui(ui, &c); }
                                Tab::Transactions => self.transactions_ui(ui),
                                Tab::Addresses    => self.addresses_ui(ui),
                                Tab::Coins        => self.coins_ui(ui),
                                Tab::Contacts     => self.contacts_ui(ui),
                                Tab::Console      => self.console_ui(ui),
                                Tab::Mining       => self.mining_ui(ui),
                            }
                        });
                });
            });

        if let Some((msg, is_err, _)) = self.status.clone() {
            egui::Area::new("toast".into())
                .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -50.0))
                .show(ctx, |ui| {
                    let (color, bg) = if is_err {
                        (self.theme.danger, Color32::from_rgba_unmultiplied(50, 25, 25, 245))
                    } else { (self.theme.success, Color32::from_rgba_unmultiplied(25, 45, 35, 245)) };
                    egui::Frame::none()
                        .fill(bg).stroke(Stroke::new(1.0, color))
                        .rounding(Rounding::same(6.0)).inner_margin(Margin::symmetric(14.0, 8.0))
                        .show(ui, |ui| { ui.colored_label(color, RichText::new(msg).size(12.0)); });
                });
        }

        self.pass_modal_ui(ctx);
        self.totp_setup_ui(ctx);
    }
}

impl App {
    fn ensure_totp_qr(&mut self, ctx: &egui::Context) {
        if self.totp_setup_qr.is_some() { return; }
        let Some(secret) = self.totp_setup_secret.clone() else { return; };
        let uri = totp_uri(&secret);
        let Ok(qr) = QrCode::new(uri.as_bytes()) else { return; };
        let modules = qr.to_colors();
        let width = qr.width();
        let scale = 6usize; let pad = 3usize;
        let total = width + pad * 2;
        let img_size = total * scale;
        let mut pixels = vec![255u8; img_size * img_size * 4];
        for y in 0..width {
            for x in 0..width {
                if modules[y * width + x] == qrcode::Color::Dark {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = (x + pad) * scale + dx;
                            let py = (y + pad) * scale + dy;
                            let idx = (py * img_size + px) * 4;
                            pixels[idx] = 0; pixels[idx+1] = 0; pixels[idx+2] = 0; pixels[idx+3] = 255;
                        }
                    }
                }
            }
        }
        let color_img = egui::ColorImage::from_rgba_unmultiplied([img_size, img_size], &pixels);
        self.totp_setup_qr = Some(ctx.load_texture("totp_qr", color_img, egui::TextureOptions::NEAREST));
    }

    fn totp_setup_ui(&mut self, ctx: &egui::Context) {
        if !self.totp_setup_open { return; }
        let t = self.theme;
        self.ensure_totp_qr(ctx);
        let secret = self.totp_setup_secret.clone().unwrap_or_default();

        let screen = ctx.screen_rect();
        egui::Area::new("totp_bg".into())
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen, Rounding::ZERO,
                    Color32::from_rgba_unmultiplied(0, 0, 0, 60));
            });

        egui::Window::new("Enable 2FA — Authenticator app")
            .collapsible(false).resizable(false).movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.style())
                .fill(t.panel_alt).stroke(Stroke::new(1.0, t.border))
                .inner_margin(Margin::same(22.0)).rounding(Rounding::same(10.0)))
            .show(ctx, |ui| {
                ui.set_min_width(440.0);
                ui.label(RichText::new("1. Scan the QR code with Google Authenticator, Authy, or Microsoft Authenticator").size(12.5).color(t.text));
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    if let Some(tex) = &self.totp_setup_qr {
                        let size = 220.0;
                        let sized = egui::load::SizedTexture::new(tex.id(), Vec2::splat(size));
                        ui.add(egui::Image::from_texture(sized));
                    }
                });
                ui.add_space(10.0);
                ui.label(RichText::new("Or enter this secret manually:").size(12.0).color(t.text_dim));
                ui.add_space(4.0);
                egui::Frame::none()
                    .fill(t.input_bg).stroke(Stroke::new(1.0, t.border))
                    .rounding(Rounding::same(6.0)).inner_margin(Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(RichText::new(&secret).monospace().size(12.5)
                                .color(t.text_strong)).wrap(true));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ghost_btn(ui, &t, "Copy").clicked() {
                                    ui.output_mut(|o| o.copied_text = secret.clone());
                                    self.notify("Copied", false);
                                }
                            });
                        });
                    });

                ui.add_space(16.0);
                ui.label(RichText::new("2. Enter the 6-digit code from the app to confirm").size(12.5).color(t.text));
                ui.add_space(6.0);
                ui.add(egui::TextEdit::singleline(&mut self.totp_input)
                    .desired_width(f32::INFINITY)
                    .hint_text("123 456")
                    .font(egui::TextStyle::Monospace).margin(Vec2::new(12.0, 10.0)));

                if !self.pass_err.is_empty() {
                    ui.add_space(8.0);
                    ui.colored_label(t.danger, RichText::new(&self.pass_err).size(12.0));
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(RichText::new("Verify & enable").color(Color32::WHITE).strong())
                            .fill(t.accent).rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(140.0, 34.0))).clicked()
                        {
                            if verify_totp(&secret, &self.totp_input) {
                                if save_totp_secret(&secret).is_ok() {
                                    self.totp_secret = Some(secret.clone());
                                    self.totp_setup_open = false;
                                    self.totp_setup_secret = None;
                                    self.totp_setup_qr = None;
                                    self.totp_input.clear();
                                    self.pass_err.clear();
                                    self.notify("2FA enabled", false);
                                } else {
                                    self.pass_err = "Failed to save secret".into();
                                }
                            } else {
                                self.pass_err = "Invalid 2FA code".into();
                            }
                        }
                        ui.add_space(8.0);
                        if ui.add(egui::Button::new(RichText::new("Cancel").color(t.text))
                            .fill(t.input_bg).stroke(Stroke::new(1.0, t.border))
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(100.0, 34.0))).clicked()
                        {
                            self.totp_setup_open = false;
                            self.totp_setup_secret = None;
                            self.totp_setup_qr = None;
                            self.totp_input.clear();
                            self.pass_err.clear();
                        }
                    });
                });
            });
    }

    fn ensure_logo(&mut self, ctx: &egui::Context) {
        if self.logo_texture.is_some() { return; }
        let exe_assets = std::env::current_exe().ok()
            .and_then(|p| p.parent().map(|p| p.join("assets").join("logo.png")));
        let bytes = exe_assets
            .and_then(|p| std::fs::read(p).ok())
            .or_else(|| std::fs::read("assets/logo.png").ok());
        if let Some(bytes) = bytes {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                let color_img = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize], rgba.as_raw());
                self.logo_texture = Some(ctx.load_texture("logo", color_img, egui::TextureOptions::LINEAR));
            }
        }
    }

    fn topbar_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let ctx = ui.ctx().clone();
        self.ensure_logo(&ctx);
        ui.horizontal_centered(|ui| {
            if let Some(tex) = &self.logo_texture {
                let sized = egui::load::SizedTexture::new(tex.id(), Vec2::new(36.0, 36.0));
                ui.add(egui::Image::from_texture(sized).rounding(Rounding::same(18.0)));
            } else {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(36.0, 36.0), egui::Sense::hover());
                let p = ui.painter();
                p.circle_filled(rect.center(), 18.0, t.orange);
                p.text(rect.center(), egui::Align2::CENTER_CENTER, "T",
                    FontId::new(19.0, FontFamily::Proportional), Color32::WHITE);
            }
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("ThoCoin Wallet").size(14.0).strong().color(t.text_strong));
                ui.label(RichText::new("v0.1.0").size(10.0).color(t.text_dim));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_btn(ui, &t, "🔧", "Tools").clicked() { self.tab = Tab::Console; }
                if icon_btn(ui, &t, "↻", "Refresh").clicked() { self.notify("Refreshed", false); }
                if icon_btn(ui, &t, "🔒", "Lock").clicked() {
                    self.show_mnemonic = false; self.notify("Locked", false);
                }
            });
        });
    }

    fn sidebar_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let items: &[(Tab, &str, &str)] = &[
            (Tab::Overview, "Overview", "⌂"),
            (Tab::Send, "Send", "✈"),
            (Tab::Receive, "Receive", "⬇"),
            (Tab::Transactions, "Transactions", "≡"),
            (Tab::Coins, "Coins", "◉"),
            (Tab::Console, "Console", ">_"),
            (Tab::Mining, "Mining", "⛏"),
        ];
        for (tab, label, icon) in items.iter().copied() {
            sidebar_item(ui, &t, self.tab == tab, icon, label, || self.tab = tab);
            ui.add_space(2.0);
        }
    }

    fn statusbar_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let cpu_on = self.miner.running.load(Ordering::SeqCst);
        let gpu_on = self.gpu_miner.running.load(Ordering::SeqCst);
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new("●").size(10.0).color(t.success));
            ui.label(RichText::new("Connected").size(11.0).color(t.text));
            ui.add_space(12.0);
            if cpu_on || gpu_on {
                let total = self.cpu_hashrate + self.gpu_hashrate;
                ui.label(RichText::new("⛏").size(11.0).color(t.warn));
                ui.label(RichText::new(fmt_hashrate(total)).size(11.0).color(t.text_dim));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let bal = self.wallet.balance(&self.chain);
                ui.label(RichText::new(format!("Balance: {} THO", fmt_coin(bal))).size(11.0).color(t.text));
            });
        });
    }

    fn overview_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let bal = self.wallet.balance(&self.chain);
        let address = self.wallet.address();
        let avail = ui.available_width();

        ui.label(RichText::new("Overview").size(20.0).strong().color(t.text_strong));
        ui.add_space(18.0);

        ui.horizontal(|ui| {
            ui.set_max_width(avail);
            ui.vertical(|ui| {
                ui.label(RichText::new("Balance").size(12.0).color(t.text_dim));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(fmt_coin(bal)).size(26.0).strong().color(t.text_strong));
                    ui.add_space(4.0);
                    ui.label(RichText::new("THO").size(15.0).color(t.text_strong));
                });
                ui.add_space(4.0);
                ui.label(RichText::new("≈ — USD").size(12.0).color(t.text_dim));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Network").size(12.0).color(t.text_dim));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("ThoCoin Mainnet").size(13.0).strong().color(t.success));
                        ui.label(RichText::new("✓").size(13.0).color(t.success));
                    });
                    ui.add_space(4.0);
                    let h = *self.chain.height.read();
                    ui.label(RichText::new(format!("Height: {}", h)).size(11.0).color(t.text_dim).family(FontFamily::Monospace));
                });
            });
        });

        ui.add_space(22.0);

        ui.label(RichText::new("Your address").size(12.0).color(t.text_dim));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let mut a = address.clone();
            ui.add(egui::TextEdit::singleline(&mut a)
                .desired_width(ui.available_width() - 80.0)
                .font(egui::TextStyle::Monospace).margin(Vec2::new(12.0, 10.0)).interactive(false));
            if icon_btn(ui, &t, "⧉", "Copy").clicked() {
                ui.output_mut(|o| o.copied_text = address.clone()); self.notify("Copied", false);
            }
            if icon_btn(ui, &t, "▦", "QR").clicked() { self.tab = Tab::Receive; }
        });

        ui.add_space(22.0);
        ui.separator();
        ui.add_space(16.0);

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("New wallet").size(13.0).strong().color(t.text_strong));
                ui.label(RichText::new("Generate a fresh wallet with a new recovery phrase. Current wallet will be replaced.")
                    .size(11.5).color(t.text_dim));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("+ Create new wallet").size(12.5).color(Color32::WHITE).strong())
                    .fill(t.accent).stroke(Stroke::NONE)
                    .rounding(Rounding::same(6.0))
                    .min_size(Vec2::new(180.0, 36.0))).clicked()
                {
                    self.open_pass_modal(PassModal::AskCreateNew);
                }
            });
        });
    }

    fn send_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        ui.label(RichText::new("Send").size(20.0).strong().color(t.text_strong));
        ui.add_space(22.0);

        ui.label(RichText::new("Pay to").size(12.0).color(t.text_dim));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.to_address)
                .desired_width(ui.available_width() - 80.0)
                .hint_text("Enter a ThoCoin address")
                .font(egui::TextStyle::Monospace).margin(Vec2::new(12.0, 10.0)));
            let _ = icon_btn(ui, &t, "📷", "Scan");
            let _ = icon_btn(ui, &t, "📋", "Paste");
        });
        ui.add_space(22.0);

        ui.label(RichText::new("Description").size(12.0).color(t.text_dim));
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(&mut self.label)
            .desired_width(f32::INFINITY)
            .hint_text("Local label (only stored locally)")
            .margin(Vec2::new(12.0, 10.0)));
        ui.add_space(22.0);

        ui.label(RichText::new("Amount").size(12.0).color(t.text_dim));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.amount)
                .desired_width(240.0)
                .hint_text("0.00000000")
                .font(egui::TextStyle::Monospace).margin(Vec2::new(12.0, 10.0)));
            ui.label(RichText::new("THO").size(12.5).color(t.text_dim));
            if ghost_btn(ui, &t, "Max").clicked() {
                let bal = self.wallet.balance(&self.chain);
                let max = bal.saturating_sub(1000) as f64 / COIN as f64;
                self.amount = format!("{:.8}", max);
            }
        });
        ui.add_space(22.0);

        ui.label(RichText::new("Fee").size(12.0).color(t.text_dim));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.fee_rate)
                .desired_width(240.0)
                .font(egui::TextStyle::Monospace).margin(Vec2::new(12.0, 10.0)));
            ui.label(RichText::new("THO").size(12.5).color(t.text_dim));
        });
        ui.add_space(22.0);

        ui.separator();
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let bal = self.wallet.balance(&self.chain);
            ui.label(RichText::new(format!("Balance: {} THO", fmt_coin(bal))).size(12.0).color(t.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                if ui.add(egui::Button::new(RichText::new("Send").size(13.5).color(Color32::WHITE).strong())
                    .fill(t.accent).stroke(Stroke::NONE)
                    .rounding(Rounding::same(6.0)).min_size(Vec2::new(110.0, 38.0))).clicked() {
                    self.do_send();
                }
                ui.add_space(10.0);
                if ui.add(egui::Button::new(RichText::new("Clear").size(13.0).color(t.text))
                    .fill(t.input_bg).stroke(Stroke::new(1.0, t.border))
                    .rounding(Rounding::same(6.0)).min_size(Vec2::new(90.0, 38.0))).clicked() {
                    self.to_address.clear(); self.amount.clear(); self.label.clear();
                }
            });
        });

        ui.add_space(28.0);
        ui.separator();
        ui.add_space(16.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("Contacts").size(15.0).strong().color(t.text_strong));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{} saved", self.contacts.len()))
                    .size(11.5).color(t.text_dim));
            });
        });
        ui.add_space(10.0);
        egui::Frame::none()
            .fill(t.panel_alt).stroke(Stroke::new(1.0, t.border))
            .rounding(Rounding::same(8.0)).inner_margin(Margin::same(12.0))
            .show(ui, |ui| {
                ui.label(RichText::new("Add new contact").size(11.5).color(t.text_dim));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.contact_name)
                        .desired_width(140.0).hint_text("Name").margin(Vec2::new(10.0, 8.0)));
                    ui.add(egui::TextEdit::singleline(&mut self.contact_addr)
                        .desired_width(ui.available_width() - 80.0)
                        .hint_text("Address")
                        .font(egui::TextStyle::Monospace).margin(Vec2::new(10.0, 8.0)));
                    if ghost_btn(ui, &t, "Add").clicked() {
                        if self.contact_name.trim().is_empty() || self.contact_addr.trim().is_empty() {
                            self.notify("Required", true);
                        } else if crate::wallet::address::decode_address(self.contact_addr.trim()).is_err() {
                            self.notify("Invalid address", true);
                        } else {
                            self.contacts.push((self.contact_name.clone(), self.contact_addr.clone()));
                            self.contact_name.clear(); self.contact_addr.clear();
                            self.notify("Added", false);
                        }
                    }
                });
            });
        ui.add_space(10.0);

        if self.contacts.is_empty() {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No contacts yet").size(12.0).color(t.text_muted));
            });
        } else {
            let list = self.contacts.clone();
            let mut to_remove = None;
            let mut to_pay = None;
            for (i, (name, addr)) in list.iter().enumerate() {
                egui::Frame::none()
                    .fill(t.panel_alt).stroke(Stroke::new(1.0, t.border))
                    .rounding(Rounding::same(6.0)).inner_margin(Margin::symmetric(12.0, 10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(name).size(12.5).strong().color(t.text_strong));
                                ui.add(egui::Label::new(RichText::new(addr).size(11.0).monospace().color(t.text_dim)).wrap(true));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_btn(ui, &t, "✕", "Remove").clicked() { to_remove = Some(i); }
                                if icon_btn(ui, &t, "↗", "Pay to this").clicked() { to_pay = Some(addr.clone()); }
                                if icon_btn(ui, &t, "⧉", "Copy").clicked() {
                                    ui.output_mut(|o| o.copied_text = addr.clone());
                                }
                            });
                        });
                    });
                ui.add_space(5.0);
            }
            if let Some(i) = to_remove { self.contacts.remove(i); }
            if let Some(a) = to_pay { self.to_address = a; }
        }
    }

    fn do_send(&mut self) {
        if self.to_address.trim().is_empty() { self.notify("Address empty", true); return; }
        let amt = self.amount.trim().parse::<f64>().unwrap_or(-1.0);
        if amt <= 0.0 { self.notify("Invalid amount", true); return; }
        if self.password_hash.is_some() {
            self.open_pass_modal(PassModal::AskSend);
        } else {
            self.execute_send();
        }
    }

    fn execute_send(&mut self) {
        let amt = self.amount.trim().parse::<f64>().unwrap_or(-1.0);
        if amt <= 0.0 { self.notify("Invalid amount", true); return; }
        let satoshi = (amt * COIN as f64) as u64;
        let fee = (self.fee_rate.trim().parse::<f64>().unwrap_or(0.00001) * COIN as f64) as u64;
        let to = self.to_address.trim().to_string();
        match self.wallet.send(&self.chain, &to, satoshi, fee) {
            Ok(tx) => {
                let txid = hash_to_hex(&tx.txid());
                self.mempool.add(tx);
                self.history.lock().push(HistoryEntry::Tx {
                    txid: txid.clone(), amount: satoshi, is_received: false,
                    timestamp: chrono::Utc::now().timestamp() as u64, confirmed: false, address: to,
                });
                self.notify(format!("Sent. tx {}", short_id(&txid)), false);
                self.amount.clear(); self.to_address.clear(); self.label.clear();
            }
            Err(e) => self.notify(format!("{}", e), true),
        }
    }

    fn receive_full_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.theme;
        let address = self.wallet.address();
        self.ensure_qr(ctx, &address);
        self.ensure_logo(ctx);
        ui.label(RichText::new("Receive").size(20.0).strong().color(t.text_strong));
        ui.add_space(22.0);

        egui::Frame::none()
            .fill(t.panel_alt).stroke(Stroke::new(1.0, t.border))
            .rounding(Rounding::same(8.0)).inner_margin(Margin::same(16.0))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(tex) = &self.qr_texture {
                        let size = 240.0;
                        let sized = egui::load::SizedTexture::new(tex.id(), Vec2::splat(size));
                        let resp = ui.add(egui::Image::from_texture(sized));
                        let center = resp.rect.center();
                        let logo_r = size * 0.11;
                        ui.painter().circle_filled(center, logo_r + 4.0, Color32::WHITE);
                        if let Some(logo) = &self.logo_texture {
                            let d = logo_r * 2.0;
                            let logo_rect = egui::Rect::from_center_size(center, Vec2::splat(d));
                            egui::Image::from_texture(egui::load::SizedTexture::new(logo.id(), Vec2::splat(d)))
                                .rounding(Rounding::same(d / 2.0))
                                .paint_at(ui, logo_rect);
                        } else {
                            ui.painter().circle_filled(center, logo_r, t.orange);
                            ui.painter().text(center, egui::Align2::CENTER_CENTER, "T",
                                FontId::new(logo_r * 1.4, FontFamily::Proportional), Color32::WHITE);
                        }
                    }
                });
                ui.add_space(14.0);
                ui.label(RichText::new("Address").size(12.0).color(t.text_dim));
                ui.add_space(6.0);
                egui::Frame::none()
                    .fill(t.input_bg).stroke(Stroke::new(1.0, t.border))
                    .rounding(Rounding::same(6.0)).inner_margin(Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&address).monospace().size(12.5).color(t.text_strong)).wrap(true));
                    });
            });
        ui.add_space(22.0);

        ui.label(RichText::new("Recovery phrase").size(12.0).color(t.text_dim));
        ui.add_space(6.0);
        egui::Frame::none()
            .fill(t.panel_alt).stroke(Stroke::new(1.0, t.border))
            .rounding(Rounding::same(8.0)).inner_margin(Margin::same(14.0))
            .show(ui, |ui| {

                egui::Frame::none()
                    .fill(t.input_bg).stroke(Stroke::new(1.0, t.border))
                    .rounding(Rounding::same(6.0)).inner_margin(Margin::same(10.0))
                    .show(ui, |ui| {
                        if self.show_mnemonic {
                            ui.add(egui::Label::new(RichText::new(&self.mnemonic_display).monospace().size(12.0)).wrap(true));
                        } else {
                            ui.label(RichText::new("•••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• ••••")
                                .monospace().size(12.0).color(t.text_muted));
                        }
                    });
                ui.add_space(12.0);

                ui.label(RichText::new("Restore from phrase").size(11.5).color(t.text_dim));
                ui.add_space(4.0);
                ui.add(egui::TextEdit::multiline(&mut self.mnemonic_input)
                    .desired_width(f32::INFINITY).desired_rows(2)
                    .hint_text("12 words separated by spaces")
                    .font(egui::TextStyle::Monospace).margin(Vec2::new(10.0, 8.0)));
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    let label = if self.show_mnemonic { "Hide" } else { "Reveal" };
                    if ghost_btn(ui, &t, label).clicked() {
                        if self.show_mnemonic {
                            self.show_mnemonic = false;
                        } else if self.password_hash.is_some() {
                            self.open_pass_modal(PassModal::AskReveal);
                        } else {
                            self.show_mnemonic = true;
                        }
                    }
                    if ghost_btn(ui, &t, "Copy phrase").clicked() {
                        ui.output_mut(|o| o.copied_text = self.mnemonic_display.clone());
                        self.notify("Copied", false);
                    }
                    if ghost_btn(ui, &t, "Restore wallet").clicked() {
                        match self.wallet.replace_from_mnemonic(self.mnemonic_input.trim()) {
                            Ok(_) => {
                                self.mnemonic_display = self.wallet.mnemonic();
                                self.mnemonic_input.clear();
                                let a = self.wallet.address();
                                self.receive_addresses = vec![("(default)".into(), a)];
                                self.reset_history_for_new_wallet();
                                self.notify("Wallet restored", false);
                            }
                            Err(e) => self.notify(format!("{}", e), true),
                        }
                    }
                });
            });
        ui.add_space(22.0);

        ui.label(RichText::new("Security").size(13.0).strong().color(t.text_strong));
        ui.add_space(10.0);
        egui::Frame::none()
            .fill(t.panel_alt).stroke(Stroke::new(1.0, t.border))
            .rounding(Rounding::same(8.0)).inner_margin(Margin::same(14.0))
            .show(ui, |ui| {

                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Password").size(13.0).strong().color(t.text_strong));
                        ui.label(RichText::new(if self.password_hash.is_some() {
                            "Enabled — required for sending and revealing seed"
                        } else { "Not set" }).size(11.5).color(t.text_dim));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (label, fill, fg) = if self.password_hash.is_some() {
                            ("Change password", t.accent, Color32::WHITE)
                        } else {
                            ("Set password", t.accent, Color32::WHITE)
                        };
                        if ui.add(egui::Button::new(RichText::new(label).size(12.5).color(fg).strong())
                            .fill(fill).stroke(Stroke::NONE)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(160.0, 36.0))).clicked()
                        {
                            let kind = if self.password_hash.is_some() { PassModal::ChangePass } else { PassModal::SetNew };
                            self.open_pass_modal(kind);
                        }
                    });
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Two-factor authentication (TOTP)")
                                .size(13.0).strong().color(t.text_strong));
                            if self.totp_secret.is_some() {
                                ui.label(RichText::new("● Enabled").size(11.0).color(t.success));
                            } else {
                                ui.label(RichText::new("● Off").size(11.0).color(t.text_muted));
                            }
                        });
                        ui.label(RichText::new("Use Google Authenticator, Authy, or Microsoft Authenticator")
                            .size(11.5).color(t.text_dim));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.totp_secret.is_some() {
                            if ui.add(egui::Button::new(RichText::new("Disable 2FA").size(12.5).color(t.danger).strong())
                                .fill(t.input_bg).stroke(Stroke::new(1.0, t.danger))
                                .rounding(Rounding::same(6.0))
                                .min_size(Vec2::new(160.0, 36.0))).clicked()
                            {
                                self.open_pass_modal(PassModal::AskDisable2FA);
                            }
                        } else {
                            if ui.add(egui::Button::new(RichText::new("Enable 2FA").size(12.5).color(Color32::WHITE).strong())
                                .fill(t.accent).stroke(Stroke::NONE)
                                .rounding(Rounding::same(6.0))
                                .min_size(Vec2::new(160.0, 36.0))).clicked()
                            {
                                let s = generate_totp_secret();
                                self.totp_setup_secret = Some(s);
                                self.totp_setup_qr = None;
                                self.totp_setup_open = true;
                                self.totp_setup_step = 0;
                                self.totp_input.clear();
                                self.pass_err.clear();
                            }
                        }
                    });
                });
            });
    }

    fn transactions_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Transactions").size(20.0).strong().color(t.text_strong));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.tx_filter)
                    .desired_width(220.0).hint_text("🔍  Search").margin(Vec2::new(10.0, 8.0)));
            });
        });
        ui.add_space(16.0);
        let hist = self.history.lock();
        let mut entries: Vec<_> = hist.iter().collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.ts()));
        let filter = self.tx_filter.to_lowercase();
        let mut shown = 0;
        for e in entries.iter() {
            let s = match e {
                HistoryEntry::Tx { txid, amount, address, .. } => format!("{} {} {}", txid, amount, address),
                HistoryEntry::Mining { block_hash, reward, .. } => format!("{} {}", block_hash, reward),
            };
            if !filter.is_empty() && !s.to_lowercase().contains(&filter) { continue; }
            electrum_row(ui, &t, e);
            shown += 1;
        }
        if shown == 0 {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| { ui.label(RichText::new("No transactions").size(13.0).color(t.text_muted)); });
        }
    }

    fn addresses_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Addresses").size(20.0).strong().color(t.text_strong));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ghost_btn(ui, &t, "+ New address").clicked() {
                    self.open_pass_modal(PassModal::AskCreateNew);
                }
            });
        });
        ui.add_space(16.0);
        let addrs = self.receive_addresses.clone();
        for (label, addr) in addrs.iter().rev() {
            let bal = self.chain.balance_for_script(
                &crate::wallet::address::script_p2pkh(
                    &crate::wallet::address::decode_address(addr).unwrap_or([0u8;20])
                )
            );
            egui::Frame::none()
                .fill(t.panel_alt).stroke(Stroke::new(1.0, t.border))
                .rounding(Rounding::same(6.0)).inner_margin(Margin::symmetric(12.0, 10.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(label).size(12.0).strong().color(t.text_strong));
                            ui.add(egui::Label::new(RichText::new(addr).size(11.0).monospace().color(t.text_dim)).wrap(true));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_btn(ui, &t, "⧉", "Copy").clicked() {
                                ui.output_mut(|o| o.copied_text = addr.clone()); self.notify("Copied", false);
                            }
                            ui.add_space(6.0);
                            ui.label(RichText::new(format!("{} THO", fmt_coin(bal)))
                                .size(12.0).color(t.text).family(FontFamily::Monospace));
                        });
                    });
                });
            ui.add_space(5.0);
        }
    }

    fn coins_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        ui.label(RichText::new("Coins (UTXOs)").size(20.0).strong().color(t.text_strong));
        ui.add_space(16.0);
        let my_script = self.wallet.key.read().script_pubkey();
        let utxo = self.chain.utxo.read();
        let mine: Vec<_> = utxo.iter().filter(|(_, o)| o.script_pubkey == my_script).collect();
        if mine.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| { ui.label(RichText::new("No coins yet").size(13.0).color(t.text_muted)); });
            return;
        }
        for (op, out) in mine {
            egui::Frame::none().inner_margin(Margin::symmetric(12.0, 10.0)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}:{}", short_id(&hex::encode(op.txid)), op.vout))
                        .size(11.5).color(t.text).family(FontFamily::Monospace));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("THO").size(10.5).color(t.text_dim));
                        ui.add_space(4.0);
                        ui.label(RichText::new(fmt_coin(out.value))
                            .size(12.0).strong().color(t.text_strong).family(FontFamily::Monospace));
                    });
                });
            });
            ui.separator();
        }
    }

    fn contacts_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        ui.label(RichText::new("Contacts").size(20.0).strong().color(t.text_strong));
        ui.add_space(16.0);
        egui::Frame::none()
            .fill(t.panel_alt).stroke(Stroke::new(1.0, t.border))
            .rounding(Rounding::same(6.0)).inner_margin(Margin::same(12.0))
            .show(ui, |ui| {
                ui.label(RichText::new("Add contact").size(12.0).color(t.text_dim));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.contact_name)
                        .desired_width(150.0).hint_text("Name").margin(Vec2::new(10.0, 8.0)));
                    ui.add(egui::TextEdit::singleline(&mut self.contact_addr)
                        .desired_width(ui.available_width() - 80.0)
                        .hint_text("Address").font(egui::TextStyle::Monospace).margin(Vec2::new(10.0, 8.0)));
                    if ghost_btn(ui, &t, "Add").clicked() {
                        if self.contact_name.trim().is_empty() || self.contact_addr.trim().is_empty() {
                            self.notify("Required", true);
                        } else if crate::wallet::address::decode_address(self.contact_addr.trim()).is_err() {
                            self.notify("Invalid address", true);
                        } else {
                            self.contacts.push((self.contact_name.clone(), self.contact_addr.clone()));
                            self.contact_name.clear(); self.contact_addr.clear();
                            self.notify("Added", false);
                        }
                    }
                });
            });
        ui.add_space(14.0);
        if self.contacts.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| { ui.label(RichText::new("No contacts").size(12.5).color(t.text_muted)); });
            return;
        }
        let list = self.contacts.clone();
        let mut to_remove = None;
        let mut to_send = None;
        for (i, (name, addr)) in list.iter().enumerate() {
            egui::Frame::none()
                .fill(t.panel_alt).stroke(Stroke::new(1.0, t.border))
                .rounding(Rounding::same(6.0)).inner_margin(Margin::symmetric(12.0, 10.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(name).size(12.0).strong().color(t.text_strong));
                            ui.add(egui::Label::new(RichText::new(addr).size(11.0).monospace().color(t.text_dim)).wrap(true));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_btn(ui, &t, "✕", "Remove").clicked() { to_remove = Some(i); }
                            if icon_btn(ui, &t, "↗", "Send").clicked() { to_send = Some(addr.clone()); }
                            if icon_btn(ui, &t, "⧉", "Copy").clicked() {
                                ui.output_mut(|o| o.copied_text = addr.clone());
                            }
                        });
                    });
                });
            ui.add_space(5.0);
        }
        if let Some(i) = to_remove { self.contacts.remove(i); }
        if let Some(a) = to_send { self.to_address = a; self.tab = Tab::Send; }
    }

    fn console_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        ui.label(RichText::new("Console").size(20.0).strong().color(t.text_strong));
        ui.add_space(16.0);
        egui::Frame::none()
            .fill(Color32::from_rgb(15, 17, 21)).stroke(Stroke::new(1.0, t.border))
            .rounding(Rounding::same(6.0)).inner_margin(Margin::same(10.0))
            .show(ui, |ui| {
                let h = ui.available_height().min(380.0).max(150.0);
                egui::ScrollArea::vertical().max_height(h - 10.0)
                    .stick_to_bottom(true).auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in &self.console_log {
                            let color = if line.starts_with("> ") { t.warn }
                                else if line.starts_with("Error") { t.danger }
                                else { Color32::from_rgb(190, 200, 210) };
                            ui.label(RichText::new(line).size(11.5).color(color).family(FontFamily::Monospace));
                        }
                    });
            });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let avail = ui.available_width();
            let r = ui.add(egui::TextEdit::singleline(&mut self.console_input)
                .desired_width(avail - 70.0).hint_text("Type command")
                .font(egui::TextStyle::Monospace).margin(Vec2::new(10.0, 8.0)));
            let submitted = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let clicked = ghost_btn(ui, &t, "Run").clicked();
            if submitted || clicked {
                let cmd = self.console_input.trim().to_string();
                if !cmd.is_empty() {
                    self.console_log.push(format!("> {}", cmd));
                    let resp = self.run_cmd(&cmd);
                    for line in resp.lines() { self.console_log.push(line.to_string()); }
                    self.console_input.clear();
                    r.request_focus();
                }
            }
        });
    }

    fn run_cmd(&mut self, cmd: &str) -> String {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        match parts.as_slice() {
            ["help"] => "help, getinfo, getbalance, getaddress, height, supply, mempool, gpu, mine cpu|gpu start|stop, send <addr> <amount>".into(),
            ["getinfo"] => format!("h={} supply={} mempool={} cpu={:.0}H/s gpu={:.0}H/s",
                *self.chain.height.read(), fmt_coin(*self.chain.supply.read()), self.mempool.len(),
                self.cpu_hashrate, self.gpu_hashrate),
            ["getbalance"] => format!("{} THO", fmt_coin(self.wallet.balance(&self.chain))),
            ["getaddress"] => self.wallet.address(),
            ["height"] => format!("{}", *self.chain.height.read()),
            ["supply"] => format!("{} / 222,000,000", fmt_coin(*self.chain.supply.read())),
            ["mempool"] => format!("{} transactions", self.mempool.len()),
            ["gpu"] => format!("device: {}\navailable: {}", self.gpu_miner.device_name.read(), self.gpu_miner.available.load(Ordering::SeqCst)),
            ["mine", "cpu", "start"] => { self.miner.start(); "cpu started".into() }
            ["mine", "cpu", "stop"]  => { self.miner.stop(); "cpu stopped".into() }
            ["mine", "gpu", "start"] => {
                if !self.gpu_miner.available.load(Ordering::SeqCst) { return "Error: GPU unavailable".into(); }
                self.gpu_miner.start(); "gpu started".into()
            }
            ["mine", "gpu", "stop"] => { self.gpu_miner.stop(); "gpu stopped".into() }
            ["send", addr, amt] => {
                let a = amt.parse::<f64>().unwrap_or(-1.0);
                if a <= 0.0 { return "Error: invalid amount".into(); }
                let sat = (a * COIN as f64) as u64;
                match self.wallet.send(&self.chain, addr, sat, 1000) {
                    Ok(tx) => { let id = hash_to_hex(&tx.txid()); self.mempool.add(tx); format!("ok txid={}", id) }
                    Err(e) => format!("Error: {}", e),
                }
            }
            _ => format!("Error: unknown command '{}'", cmd),
        }
    }

    fn mining_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        ui.label(RichText::new("Mining").size(20.0).strong().color(t.text_strong));
        ui.add_space(18.0);

        let cpu_running = self.miner.running.load(Ordering::SeqCst);
        let gpu_running = self.gpu_miner.running.load(Ordering::SeqCst);
        let gpu_available = self.gpu_miner.available.load(Ordering::SeqCst);
        let found_cpu = self.miner.blocks_found.load(Ordering::Relaxed);
        let found_gpu = self.gpu_miner.blocks_found.load(Ordering::Relaxed);
        let gpu_dev = self.gpu_miner.device_name.read().clone();

        let border_cpu = if self.use_cpu { t.accent } else { t.border };
        egui::Frame::none()
            .fill(t.panel_alt).stroke(Stroke::new(1.0, border_cpu))
            .rounding(Rounding::same(8.0)).inner_margin(Margin::same(16.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.use_cpu, "");
                    ui.label(RichText::new("💻  CPU Miner").size(14.0).strong().color(t.text_strong));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (dot, lbl) = if cpu_running { (t.success, "Active") } else { (t.text_muted, "Idle") };
                        ui.label(RichText::new(lbl).size(11.5).color(dot));
                        ui.label(RichText::new("●").color(dot).size(10.0));
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let hr = if cpu_running { self.cpu_hashrate } else { 0.0 };
                    ui.label(RichText::new(fmt_hashrate(hr)).size(22.0).strong()
                        .color(if self.use_cpu { t.text_strong } else { t.text_muted }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{} blocks · {} cores", found_cpu, num_cpus()))
                            .size(11.0).color(t.text_muted));
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Threads").size(11.5).color(t.text_dim));
                    ui.add_enabled(self.use_cpu,
                        egui::Slider::new(&mut self.threads, 1..=num_cpus()).show_value(true));
                });
            });

        ui.add_space(12.0);

        let border_gpu = if self.use_gpu && gpu_available { t.accent }
            else if !gpu_available { t.danger } else { t.border };
        egui::Frame::none()
            .fill(t.panel_alt).stroke(Stroke::new(1.0, border_gpu))
            .rounding(Rounding::same(8.0)).inner_margin(Margin::same(16.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let prev = self.use_gpu;
                    ui.add_enabled(gpu_available, egui::Checkbox::new(&mut self.use_gpu, ""));
                    ui.label(RichText::new("🎮  GPU Miner").size(14.0).strong().color(t.text_strong));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (dot, lbl) = if !gpu_available { (t.danger, "Unavailable") }
                            else if gpu_running { (t.success, "Active") }
                            else { (t.text_muted, "Idle") };
                        ui.label(RichText::new(lbl).size(11.5).color(dot));
                        ui.label(RichText::new("●").color(dot).size(10.0));
                    });
                    if !prev && self.use_gpu && !gpu_available {
                        self.use_gpu = false;
                        self.notify("No OpenCL device found", true);
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let hr = if gpu_running { self.gpu_hashrate } else { 0.0 };
                    ui.label(RichText::new(fmt_hashrate(hr)).size(22.0).strong()
                        .color(if self.use_gpu && gpu_available { t.text_strong } else { t.text_muted }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{} blocks", found_gpu)).size(11.0).color(t.text_muted));
                    });
                });
                ui.add_space(8.0);
                ui.add(egui::Label::new(RichText::new(format!("Device: {}", gpu_dev))
                    .size(11.0).color(t.text_dim)).wrap(true));
            });

        ui.add_space(14.0);

        egui::Frame::none()
            .fill(t.panel_alt).stroke(Stroke::new(1.0, t.border))
            .rounding(Rounding::same(8.0)).inner_margin(Margin::same(14.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let total = (if cpu_running { self.cpu_hashrate } else { 0.0 })
                              + (if gpu_running { self.gpu_hashrate } else { 0.0 });
                    ui.label(RichText::new("Total").size(11.5).color(t.text_dim));
                    ui.add_space(6.0);
                    ui.label(RichText::new(fmt_hashrate(total))
                        .size(17.0).strong().color(t.text_strong).family(FontFamily::Monospace));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let any_running = cpu_running || gpu_running;
                        if !any_running {
                            let can_start = self.use_cpu || (self.use_gpu && gpu_available);
                            let resp = ui.add_enabled(can_start,
                                egui::Button::new(RichText::new("▶ Start").size(13.0).color(Color32::WHITE).strong())
                                    .fill(t.accent).stroke(Stroke::NONE)
                                    .rounding(Rounding::same(6.0)).min_size(Vec2::new(140.0, 36.0)));
                            if resp.clicked() {
                                if self.use_cpu { self.miner.start(); }
                                if self.use_gpu && gpu_available { self.gpu_miner.start(); }
                                self.notify("Started", false);
                            }
                        } else {
                            if ui.add(egui::Button::new(RichText::new("■ Stop").size(13.0).color(t.text).strong())
                                .fill(t.input_bg).stroke(Stroke::new(1.0, t.border))
                                .rounding(Rounding::same(6.0)).min_size(Vec2::new(140.0, 36.0))).clicked() {
                                self.miner.stop(); self.gpu_miner.stop();
                                self.notify("Stopped", false);
                            }
                        }
                    });
                });
            });

        ui.add_space(14.0);

        egui::Frame::none()
            .fill(t.panel_alt).stroke(Stroke::new(1.0, t.border))
            .rounding(Rounding::same(8.0)).inner_margin(Margin::same(16.0))
            .show(ui, |ui| {
                ui.label(RichText::new("Statistics").size(12.5).color(t.text_dim));
                ui.add_space(10.0);
                kv(ui, &t, "Blocks (CPU + GPU)", &format!("{}", found_cpu + found_gpu));
                kv(ui, &t, "Block height", &format!("{}", *self.chain.height.read()));
                let next = crate::core::consensus::block_reward(*self.chain.height.read()+1, *self.chain.supply.read());
                kv(ui, &t, "Current reward", &fmt_coin(next));
                kv(ui, &t, "Reward address", &short_addr(&self.wallet.address()));
                kv(ui, &t, "Difficulty", &format!("0x{:08x}", self.chain.current_bits()));
            });
    }
}

fn sidebar_item(ui: &mut egui::Ui, t: &Theme, selected: bool, icon: &str, label: &str, mut on_click: impl FnMut()) {
    let row_h = 36.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), egui::Sense::click());
    let bg = if selected { t.sidebar_selected }
        else if resp.hovered() { Color32::from_rgba_unmultiplied(255,255,255,10) }
        else { Color32::TRANSPARENT };
    let fg = if selected { Color32::WHITE } else { t.text };
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(6.0), bg);
    p.text(rect.min + Vec2::new(14.0, row_h / 2.0), egui::Align2::LEFT_CENTER, icon,
        FontId::new(14.0, FontFamily::Proportional), fg);
    p.text(rect.min + Vec2::new(40.0, row_h / 2.0), egui::Align2::LEFT_CENTER, label,
        FontId::new(13.0, FontFamily::Proportional), fg);
    if resp.clicked() { on_click(); }
    if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
}

fn kv(ui: &mut egui::Ui, t: &Theme, k: &str, v: &str) {
    let full_w = ui.available_width();
    ui.allocate_ui_with_layout(Vec2::new(full_w, 0.0),
        egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.label(RichText::new(k).size(11.5).color(t.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(v).size(12.0).color(t.text_strong).family(FontFamily::Monospace));
            });
        });
    ui.add_space(5.0);
}

fn electrum_row(ui: &mut egui::Ui, t: &Theme, e: &HistoryEntry) {
    let full_w = ui.available_width();
    egui::Frame::none().inner_margin(Margin::symmetric(4.0, 8.0)).show(ui, |ui| {
        ui.allocate_ui_with_layout(Vec2::new(full_w, 0.0),
            egui::Layout::left_to_right(egui::Align::Center), |ui| {
                match e {
                    HistoryEntry::Mining { height, block_hash, reward, timestamp } => {
                        circle_icon(ui, t.warn, "⛏");
                        ui.add_space(10.0);
                        ui.allocate_ui(Vec2::new(90.0, 0.0), |ui| {
                            ui.label(RichText::new("Mined").size(12.5).strong().color(t.warn));
                        });
                        ui.allocate_ui(Vec2::new(130.0, 0.0), |ui| {
                            ui.label(RichText::new(fmt_ts(*timestamp)).size(11.0).color(t.text_dim));
                        });
                        ui.label(RichText::new(format!("#{} {}", height, short_id(block_hash)))
                            .size(11.0).color(t.text_dim).family(FontFamily::Monospace));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new("THO").size(10.5).color(t.text_dim));
                            ui.add_space(3.0);
                            ui.label(RichText::new(format!("+ {}", fmt_coin(*reward)))
                                .size(12.0).strong().color(t.success).family(FontFamily::Monospace));
                        });
                    }
                    HistoryEntry::Tx { txid, amount, is_received, timestamp, confirmed, address } => {
                        let (icon, color, prefix, lbl) = if *is_received {
                            ("↓", t.success, "+", "Received")
                        } else { ("↑", t.danger, "−", "Sent") };
                        circle_icon(ui, color, icon);
                        ui.add_space(10.0);
                        ui.allocate_ui(Vec2::new(90.0, 0.0), |ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(lbl).size(12.5).strong().color(color));
                                if !*confirmed { ui.label(RichText::new("unconfirmed").size(9.5).color(t.warn)); }
                            });
                        });
                        ui.allocate_ui(Vec2::new(130.0, 0.0), |ui| {
                            ui.label(RichText::new(fmt_ts(*timestamp)).size(11.0).color(t.text_dim));
                        });
                        let disp = if address.is_empty() { short_id(txid) } else { short_addr(address) };
                        ui.label(RichText::new(disp).size(11.0).color(t.text_dim).family(FontFamily::Monospace));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new("THO").size(10.5).color(t.text_dim));
                            ui.add_space(3.0);
                            ui.label(RichText::new(format!("{} {}", prefix, fmt_coin(*amount)))
                                .size(12.0).strong().color(color).family(FontFamily::Monospace));
                        });
                    }
                }
            });
    });
}

fn circle_icon(ui: &mut egui::Ui, color: Color32, icon: &str) {
    let size = 26.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(size, size), egui::Sense::hover());
    let center = rect.center();
    let p = ui.painter();
    p.circle_stroke(center, size/2.0 - 1.0, Stroke::new(1.5, color));
    p.text(center, egui::Align2::CENTER_CENTER, icon,
        FontId::new(14.0, FontFamily::Proportional), color);
}
fn data_dir() -> std::path::PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(std::path::PathBuf::from))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = base.join("ThoCoin");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn pass_file_path() -> std::path::PathBuf { data_dir().join("wallet.pass") }

fn load_password_hash() -> Option<[u8; 32]> {
    let bytes = std::fs::read(pass_file_path()).ok()?;
    if bytes.len() == 32 {
        let mut h = [0u8; 32];
        h.copy_from_slice(&bytes);
        Some(h)
    } else { None }
}

fn save_password_hash(hash: &[u8; 32]) -> std::io::Result<()> {
    std::fs::write(pass_file_path(), hash)
}

fn hash_password(s: &str) -> [u8; 32] {
    sha256d(s.as_bytes())
}

fn delete_password_file() {
    let _ = std::fs::remove_file(pass_file_path());
}

fn totp_file_path() -> std::path::PathBuf { data_dir().join("wallet.totp") }

fn load_totp_secret() -> Option<String> {
    let s = std::fs::read_to_string(totp_file_path()).ok()?;
    let s = s.trim();
    if s.is_empty() { None } else { Some(s.to_string()) }
}

fn save_totp_secret(secret_b32: &str) -> std::io::Result<()> {
    std::fs::write(totp_file_path(), secret_b32)
}

fn delete_totp() {
    let _ = std::fs::remove_file(totp_file_path());
}

fn generate_totp_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut bytes);
    base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &bytes)
}

fn build_totp(secret_b32: &str) -> Option<totp_rs::TOTP> {
    let bytes = base32::decode(base32::Alphabet::Rfc4648 { padding: false }, secret_b32)?;
    totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6, 2, 30,
        bytes,
    ).ok()
}

fn verify_totp(secret_b32: &str, code: &str) -> bool {
    let code = code.trim().replace(' ', "");
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) { return false; }
    match build_totp(secret_b32) {
        Some(t) => t.check_current(&code).unwrap_or(false),
        None => false,
    }
}

fn totp_uri(secret_b32: &str) -> String {
    format!("otpauth://totp/ThoCoin:wallet?secret={}&issuer=ThoCoin&algorithm=SHA1&digits=6&period=30", secret_b32)
}
