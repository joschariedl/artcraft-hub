//! ArtCraft Hub – a native installer/updater for the Craft Apps.
//!
//! No web view, no Electron: egui/eframe render the whole UI. Every install is
//! verified against the release's `SHA256SUMS.txt` and VirusTotal before a
//! single byte is extracted.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, RichText, Sense, Stroke, Vec2,
};

use artcraft_core::catalog::{self};
use artcraft_core::config::Config;
use artcraft_core::ops::{self, AppStatus};
use artcraft_core::paths::Paths;
use artcraft_core::progress::Progress;

const BG: Color32 = Color32::from_rgb(13, 17, 24);
const PANEL: Color32 = Color32::from_rgb(18, 23, 32);
const CARD: Color32 = Color32::from_rgb(24, 30, 41);
const CARD_HOVER: Color32 = Color32::from_rgb(31, 39, 53);
const BORDER: Color32 = Color32::from_rgb(42, 51, 68);
const TEXT: Color32 = Color32::from_rgb(232, 236, 244);
const MUTED: Color32 = Color32::from_rgb(144, 156, 178);
const ACCENT: Color32 = Color32::from_rgb(64, 208, 180);
const OK: Color32 = Color32::from_rgb(92, 200, 130);
const WARN: Color32 = Color32::from_rgb(240, 180, 80);
const BAD: Color32 = Color32::from_rgb(240, 105, 115);

fn main() -> eframe::Result<()> {
    let mut args = std::env::args().skip(1);
    if let Some(arg) = args.next() {
        match arg.as_str() {
            "-V" | "--version" => {
                println!("artcraft-hub {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-h" | "--help" => {
                println!(
                    "ArtCraft Hub {}\n\nUsage: artcraft-hub\n\nTakes no arguments; all actions happen in the UI.\n\n  -h, --help     This help\n  -V, --version  Print version",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            _ => {
                std::process::exit(2);
            }
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ArtCraft Hub")
            .with_inner_size([1120.0, 780.0])
            .with_min_inner_size([900.0, 620.0])
            .with_app_id("artcraft-hub"),
        ..Default::default()
    };
    eframe::run_native(
        "artcraft-hub",
        options,
        Box::new(|cc| Ok(Box::new(HubApp::new(cc)))),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Apps,
    Settings,
}

#[derive(Clone, PartialEq, Eq)]
enum Busy {
    Check,
    Install(String),
    InstallAll,
    Upgrade,
}

impl Busy {
    fn label(&self) -> String {
        match self {
            Busy::Check => "Checking releases …".into(),
            Busy::Install(name) => format!("Installing {name} …"),
            Busy::InstallAll => "Installing apps …".into(),
            Busy::Upgrade => "Updating apps …".into(),
        }
    }
}

enum Msg {
    Status(Vec<AppStatus>),
    Progress(Progress),
    Done { text: String, ok: bool },
}

struct SettingsDraft {
    vt_key: String,
    github_token: String,
    allow_unknown: bool,
    install_1024: bool,
    require_sums: bool,
    check_interval: u64,
}

impl SettingsDraft {
    fn from_config(cfg: &Config) -> Self {
        Self {
            vt_key: cfg.virustotal_api_key.clone(),
            github_token: cfg.github_token.clone(),
            allow_unknown: cfg.vt.allow_unknown,
            install_1024: cfg.install_1024_icons,
            require_sums: cfg.require_sha256sums,
            check_interval: cfg.check_interval_minutes,
        }
    }

    fn apply(&self, cfg: &mut Config) {
        cfg.virustotal_api_key = self.vt_key.trim().to_string();
        cfg.github_token = self.github_token.trim().to_string();
        cfg.vt.allow_unknown = self.allow_unknown;
        cfg.install_1024_icons = self.install_1024;
        cfg.require_sha256sums = self.require_sums;
        cfg.check_interval_minutes = self.check_interval.clamp(5, 24 * 60);
    }
}

struct HubApp {
    paths: Paths,
    cfg: Config,
    statuses: Vec<AppStatus>,
    selected: Option<String>,
    view: View,
    busy: Option<Busy>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    icons_started: bool,
    log: Vec<String>,
    download: Option<(u64, Option<u64>)>,
    search: String,
    show_updates: bool,
    auto_opened_updates: bool,
    update_selection: HashMap<String, bool>,
    settings: SettingsDraft,
    toast: Option<(String, bool, Instant)>,
    confirm_uninstall: Option<String>,
    show_log: bool,
}

impl HubApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let paths = Paths::from_env();
        let _ = paths.ensure();
        let cfg = Config::load(&paths).unwrap_or_default();
        let (tx, rx) = channel();

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = CARD;
        visuals.extreme_bg_color = Color32::from_rgb(10, 13, 19);
        visuals.faint_bg_color = CARD;
        visuals.override_text_color = Some(TEXT);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(30, 37, 50);
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(30, 37, 50);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(40, 49, 65);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(40, 49, 65);
        visuals.widgets.active.bg_fill = Color32::from_rgb(48, 58, 76);
        visuals.selection.bg_fill = ACCENT.linear_multiply(0.35);
        visuals.selection.stroke = Stroke::new(1.0, ACCENT);
        cc.egui_ctx.set_visuals(visuals);

        cc.egui_ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = Vec2::new(8.0, 8.0);
            style.spacing.button_padding = Vec2::new(12.0, 7.0);
            style.text_styles.insert(
                egui::TextStyle::Heading,
                FontId::new(22.0, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Body,
                FontId::new(14.0, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Button,
                FontId::new(14.0, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Small,
                FontId::new(11.5, egui::FontFamily::Proportional),
            );
        });

        Self {
            settings: SettingsDraft::from_config(&cfg),
            paths,
            cfg,
            statuses: Vec::new(),
            selected: None,
            view: View::Apps,
            busy: None,
            tx,
            rx,
            icons_started: false,
            log: Vec::new(),
            download: None,
            search: String::new(),
            show_updates: false,
            auto_opened_updates: false,
            update_selection: HashMap::new(),
            toast: None,
            confirm_uninstall: None,
            show_log: false,
        }
    }

    // -----------------------------------------------------------------------
    // Background work
    // -----------------------------------------------------------------------

    fn spawn_check(&mut self, ctx: &egui::Context, force: bool) {
        if self.busy.is_some() {
            self.toast("Please wait, a task is already running.", false);
            return;
        }
        self.busy = Some(Busy::Check);
        self.download = None;
        let tx = self.tx.clone();
        let paths = self.paths.clone();
        let cfg = self.cfg.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let report = |event: Progress| {
                let _ = tx.send(Msg::Progress(event));
                ctx.request_repaint();
            };
            let statuses = ops::check_all(&paths, &cfg, force, &report);
            let _ = tx.send(Msg::Status(statuses));
            ctx.request_repaint();
        });
    }

    fn spawn_install(&mut self, ctx: &egui::Context, id: String) {
        if self.busy.is_some() {
            return;
        }
        let name = catalog::by_id(&id)
            .map(|a| a.name.to_string())
            .unwrap_or_else(|| id.clone());
        self.busy = Some(Busy::Install(name.clone()));
        self.download = None;
        self.log.clear();

        let tx = self.tx.clone();
        let paths = self.paths.clone();
        let cfg = self.cfg.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let report = |event: Progress| {
                let _ = tx.send(Msg::Progress(event));
                ctx.request_repaint();
            };
            let (text, ok) = match ops::install(&id, &paths, &cfg, false, &report) {
                Ok(outcome) => {
                    let mut text = format!(
                        "{} {} installed successfully.",
                        name,
                        outcome.version
                    );
                    if let Some(vt) = &outcome.vt {
                        text.push_str(&format!(" VirusTotal: {}.", vt.verdict.label()));
                    } else if outcome.already_installed {
                        text = format!("{} {} is already installed.", name, outcome.version);
                    }
                    (text, true)
                }
                Err(error) => (format!("{name}: {error}"), false),
            };
            let _ = tx.send(Msg::Done { text, ok });
            ctx.request_repaint();
        });
    }

    fn spawn_upgrade(&mut self, ctx: &egui::Context, ids: Vec<String>) {
        if self.busy.is_some() || ids.is_empty() {
            return;
        }
        self.busy = Some(Busy::Upgrade);
        self.download = None;
        self.log.clear();

        let tx = self.tx.clone();
        let paths = self.paths.clone();
        let cfg = self.cfg.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let report = |event: Progress| {
                let _ = tx.send(Msg::Progress(event));
                ctx.request_repaint();
            };
            let total = ids.len();
            let mut succeeded = 0usize;
            let mut failures = Vec::new();
            for id in ids {
                let name = catalog::by_id(&id)
                    .map(|a| a.name.to_string())
                    .unwrap_or_else(|| id.clone());
                match ops::install(&id, &paths, &cfg, false, &report) {
                    Ok(_) => succeeded += 1,
                    Err(error) => failures.push(format!("{name}: {error}")),
                }
            }
            let mut text = format!("{succeeded} of {total} app(s) updated.");
            if !failures.is_empty() {
                text.push_str(" Failed: ");
                text.push_str(&failures.join(" · "));
            }
            let _ = tx.send(Msg::Done {
                text,
                ok: failures.is_empty(),
            });
            ctx.request_repaint();
        });
    }

    fn spawn_install_all(&mut self, ctx: &egui::Context) {
        if self.busy.is_some() {
            return;
        }
        let ids: Vec<String> = self
            .statuses
            .iter()
            .filter(|s| s.installed_version.is_none() && s.plan.is_some())
            .map(|s| s.id.clone())
            .collect();
        if ids.is_empty() {
            return;
        }
        self.busy = Some(Busy::InstallAll);
        self.download = None;
        self.log.clear();

        let tx = self.tx.clone();
        let paths = self.paths.clone();
        let cfg = self.cfg.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let report = |event: Progress| {
                let _ = tx.send(Msg::Progress(event));
                ctx.request_repaint();
            };
            let total = ids.len();
            let mut succeeded = 0usize;
            let mut failures = Vec::new();
            for id in ids {
                let name = catalog::by_id(&id)
                    .map(|a| a.name.to_string())
                    .unwrap_or_else(|| id.clone());
                match ops::install(&id, &paths, &cfg, false, &report) {
                    Ok(_) => succeeded += 1,
                    Err(error) => failures.push(format!("{name}: {error}")),
                }
            }
            let mut text = format!("{succeeded} of {total} app(s) installed.");
            if !failures.is_empty() {
                text.push_str(" Failed: ");
                text.push_str(&failures.join(" · "));
            }
            let _ = tx.send(Msg::Done {
                text,
                ok: failures.is_empty(),
            });
            ctx.request_repaint();
        });
    }

    fn spawn_vt_recheck(&mut self, ctx: &egui::Context, id: String) {
        if self.busy.is_some() {
            return;
        }
        let name = catalog::by_id(&id)
            .map(|a| a.name.to_string())
            .unwrap_or_else(|| id.clone());
        self.busy = Some(Busy::Check);
        self.download = None;
        self.log.clear();

        let tx = self.tx.clone();
        let paths = self.paths.clone();
        let cfg = self.cfg.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let report = |event: Progress| {
                let _ = tx.send(Msg::Progress(event));
                ctx.request_repaint();
            };
            match ops::vt_recheck(&id, &paths, &cfg, &report) {
                Ok(vt) => {
                    let text = format!(
                        "VirusTotal re-check for {name}: {} ({} of {} engines).",
                        vt.verdict.label(),
                        vt.stats.bad(),
                        vt.stats.total()
                    );
                    let _ = tx.send(Msg::Done { text, ok: vt.verdict != artcraft_core::vt::Verdict::Blocked });
                }
                Err(error) => {
                    let _ = tx.send(Msg::Done {
                        text: format!("{name}: {error}"),
                        ok: false,
                    });
                }
            }
            ctx.request_repaint();
        });
    }

    // -----------------------------------------------------------------------
    // Message handling
    // -----------------------------------------------------------------------

    fn drain(&mut self, ctx: &egui::Context) {
        let mut needs_check = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Status(statuses) => {
                    self.busy = None;
                    self.download = None;
                    let errors = statuses.iter().filter(|s| s.error.is_some()).count();
                    self.statuses = statuses;
                    if self.selected.is_none() {
                        self.selected = self.statuses.first().map(|s| s.id.clone());
                    }
                    for status in &self.statuses {
                        if status.update_available {
                            self.update_selection
                                .entry(status.id.clone())
                                .or_insert(true);
                        }
                    }
                    if errors > 0 {
                        self.push_log(format!(
                            "{} app(s) could not be checked.",
                            errors
                        ));
                    } else {
                        self.push_log("Release check complete.".to_string());
                    }
                    if !self.auto_opened_updates
                        && self.statuses.iter().any(|s| s.update_available)
                    {
                        self.show_updates = true;
                        self.auto_opened_updates = true;
                    }
                }
                Msg::Progress(Progress::Message(text)) => self.push_log(text),
                Msg::Progress(Progress::Download { received, total }) => {
                    self.download = Some((received, total));
                }
                Msg::Done { text, ok } => {
                    self.busy = None;
                    self.download = None;
                    self.push_log(text.clone());
                    self.toast(text, ok);
                    needs_check = true;
                }
                // (icons are loaded synchronously from embedded data)
            }
        }
        if needs_check {
            self.spawn_check(ctx, false);
        }
    }

    fn push_log(&mut self, text: String) {
        if text.trim().is_empty() {
            return;
        }
        self.log.push(text);
        if self.log.len() > 400 {
            self.log.drain(..self.log.len() - 400);
        }
    }

    fn toast(&mut self, text: impl Into<String>, ok: bool) {
        self.toast = Some((text.into(), ok, Instant::now()));
    }

    // -----------------------------------------------------------------------
    // UI
    // -----------------------------------------------------------------------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("top")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(18, 12))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("ArtCraft")
                            .size(24.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new("HUB")
                            .size(20.0)
                            .strong()
                            .color(ACCENT),
                    );
                    ui.add_space(24.0);
                    if ui
                        .selectable_label(self.view == View::Apps, "Apps")
                        .clicked()
                    {
                        self.view = View::Apps;
                    }
                    if ui
                        .selectable_label(self.view == View::Settings, "Settings")
                        .clicked()
                    {
                        self.view = View::Settings;
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(busy) = &self.busy {
                            ui.add(egui::Spinner::new().size(16.0).color(ACCENT));
                            ui.label(RichText::new(busy.label()).color(MUTED));
                        } else {
                            let updates = self
                                .statuses
                                .iter()
                                .filter(|s| s.update_available)
                                .count();
                            if updates > 0
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(format!("{updates} Update(s)"))
                                                .color(Color32::BLACK)
                                                .strong(),
                                        )
                                        .fill(ACCENT)
                                        .corner_radius(CornerRadius::same(8)),
                                    )
                                    .clicked()
                            {
                                self.show_updates = true;
                            }
                            let not_installed = self
                                .statuses
                                .iter()
                                .filter(|s| s.installed_version.is_none() && s.plan.is_some())
                                .count();
                            if not_installed > 0
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(format!("Install all ({not_installed})"))
                                                .color(Color32::BLACK)
                                                .strong(),
                                        )
                                        .fill(ACCENT)
                                        .corner_radius(CornerRadius::same(8)),
                                    )
                                    .clicked()
                            {
                                self.spawn_install_all(&ctx);
                            }
                            if ui.button("Refresh").clicked() {
                                self.spawn_check(&ctx, true);
                            }
                        }
                    });
                });
            });
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("bottom")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(18, 8))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    if self.busy.is_some() {
                        if let Some((received, total)) = self.download {
                            let fraction = total
                                .filter(|t| *t > 0)
                                .map(|t| received as f32 / t as f32)
                                .unwrap_or(0.0);
                            ui.add(
                                egui::ProgressBar::new(fraction)
                                    .desired_width(220.0)
                                    .corner_radius(CornerRadius::same(4))
                                    .fill(ACCENT),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{:.1} / {:.1} MB",
                                    received as f64 / 1_048_576.0,
                                    total.unwrap_or(0) as f64 / 1_048_576.0
                                ))
                                .small()
                                .color(MUTED),
                            );
                        } else {
                            ui.label(RichText::new("Working …").color(MUTED));
                        }
                    } else if let Some(last) = self.log.last() {
                        ui.label(
                            RichText::new(truncate(last, 110))
                                .small()
                                .color(MUTED),
                        );
                    } else {
                        ui.label(RichText::new("Ready.").small().color(MUTED));
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                                .small()
                                .color(MUTED),
                        );
                        if ui
                            .selectable_label(
                                self.show_log,
                                RichText::new("Log").small().color(TEXT),
                            )
                            .clicked()
                        {
                            self.show_log = !self.show_log;
                        }
                        let vt = if self.cfg.vt_key().is_some() {
                            RichText::new("VirusTotal active").small().color(OK)
                        } else {
                            RichText::new("VirusTotal key missing").small().color(BAD)
                        };
                        ui.label(vt);
                    });
                });
            });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("apps")
            .exact_size(330.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(12, 12))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search apps …")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(4.0);

                let search = self.search.trim().to_lowercase();
                let mut clicked = None;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for status in &self.statuses {
                            if !search.is_empty()
                                && !status.name.to_lowercase().contains(&search)
                                && !status.id.contains(&search)
                            {
                                continue;
                            }
                            let selected = self.selected.as_deref() == Some(&status.id);
                            let frame = egui::Frame::new()
                                .fill(if selected { CARD_HOVER } else { CARD })
                                .corner_radius(CornerRadius::same(10))
                                .inner_margin(Margin::symmetric(10, 8))
                                .stroke(Stroke::new(
                                    1.0,
                                    if selected { ACCENT } else { BORDER },
                                ));
                            let row = frame
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        self.icon(ui, status, 38.0);
                                        ui.add_space(4.0);
                                        ui.vertical(|ui| {
                                            ui.label(
                                                RichText::new(&status.name)
                                                    .strong()
                                                    .color(TEXT),
                                            );
                                            let (text, color) = side_status(status);
                                            ui.label(
                                                RichText::new(truncate(&text, 30))
                                                    .small()
                                                    .color(color),
                                            );
                                        });
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if status.update_available {
                                                    ui.label(
                                                        RichText::new("●")
                                                            .color(WARN)
                                                            .strong(),
                                                    );
                                                } else if status.installed_version.is_some()
                                                {
                                                    status_glyph(ui, true);
                                                }
                                            },
                                        );
                                    });
                                })
                                .response;
                            if row.interact(Sense::click()).clicked() {
                                clicked = Some(status.id.clone());
                            }
                            ui.add_space(6.0);
                        }
                    });
                if let Some(id) = clicked {
                    self.selected = Some(id);
                    self.view = View::Apps;
                }
            });
    }

    fn icon(&self, ui: &mut egui::Ui, status: &AppStatus, size: f32) {
        // Load from a process-global cache so textures survive any reset.
        static ICON_CACHE: std::sync::Mutex<Option<std::collections::HashMap<String, egui::TextureHandle>>> =
            std::sync::Mutex::new(None);
        let texture = {
            let mut cache = ICON_CACHE.lock().unwrap();
            if cache.is_none() {
                *cache = Some(std::collections::HashMap::new());
            }
            let cache = cache.as_mut().unwrap();
            if !cache.contains_key(&status.id) {
                // Prefer the embedded, padding-free icons (always available).
                let img = if let Some(bytes) = embedded_icon(&status.id) {
                    image::load_from_memory(bytes).ok().map(|img| to_color_image(&img))
                } else {
                    let bundle = catalog::by_id(&status.id).map(|a| a.bundle_id()).unwrap_or_default();
                    local_icon(&self.paths, &status.id, &bundle).and_then(|p| load_image(&p))
                };
                if let Some(image) = img {
                    let tex = ui.ctx().load_texture(
                        format!("icon-{}", status.id),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    cache.insert(status.id.clone(), tex);
                }
            }
            cache.get(&status.id).cloned()
        };
        if let Some(texture) = texture {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            let (response, painter) =
                ui.allocate_painter(Vec2::splat(size), Sense::hover());
            let rect = response.rect;
            let color = Color32::from_rgb(
                status.color[0],
                status.color[1],
                status.color[2],
            );
            painter.circle_filled(rect.center(), size / 2.0, color.linear_multiply(0.85));
            let initial = status.name.chars().next().unwrap_or('?');
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                initial,
                FontId::proportional(size * 0.42),
                Color32::WHITE,
            );
        }
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(24, 18)),
            )
            .show(ui, |ui| {
                if self.view == View::Settings {
                    self.settings_view(ui);
                    return;
                }
                if self.cfg.vt_key().is_none() {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(58, 30, 34))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 12))
                        .stroke(Stroke::new(1.0, BAD.linear_multiply(0.6)))
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new("⚠").color(BAD).strong());
                                ui.label(
                                    RichText::new(
                                        "No VirusTotal API key configured. Installs are locked until a key is set.",
                                    )
                                    .color(TEXT),
                                );
                                if ui.button("Set up now").clicked() {
                                    self.view = View::Settings;
                                }
                            });
                        });
                    ui.add_space(10.0);
                }

                let Some(status) = self
                    .statuses
                    .iter()
                    .find(|s| Some(&s.id) == self.selected.as_ref())
                    .cloned()
                else {
                    ui.centered_and_justified(|ui| {
                        if self.busy.is_some() {
                            ui.add(egui::Spinner::new().size(28.0).color(ACCENT));
                        } else {
                            ui.label(RichText::new("No app selected.").color(MUTED));
                        }
                    });
                    return;
                };

                let app = catalog::by_id(&status.id);
                ui.horizontal(|ui| {
                    self.icon(ui, &status, 92.0);
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(&status.name)
                                .size(30.0)
                                .strong()
                                .color(TEXT),
                        );
                        ui.label(
                            RichText::new(&status.tagline)
                                .size(15.0)
                                .color(MUTED),
                        );
                        if let Some(app) = app {
                            ui.hyperlink_to(
                                RichText::new("getartcraft.com").small().color(ACCENT),
                                app.website(),
                            );
                        }
                    });
                });

                ui.add_space(14.0);
                self.action_row(ui, &status, &ctx);
                ui.add_space(14.0);

                self.info_card(ui, &status);
                ui.add_space(12.0);
                self.security_card(ui, &status);
                ui.add_space(12.0);

                egui::Frame::new()
                    .fill(CARD)
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 12))
                    .stroke(Stroke::new(1.0, BORDER))
                    .show(ui, |ui| {
                        ui.label(RichText::new("Release notes").strong().color(TEXT));
                        ui.add_space(4.0);
                        let notes = status
                            .plan
                            .as_ref()
                            .map(|p| p.notes.trim())
                            .filter(|n| !n.is_empty())
                            .unwrap_or("No release notes available.");
                        egui::ScrollArea::vertical()
                            .id_salt("notes")
                            .max_height(200.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(truncate(notes, 6000))
                                        .monospace()
                                        .size(12.0)
                                        .color(MUTED),
                                );
                            });
                    });
            });
    }

    fn action_row(&mut self, ui: &mut egui::Ui, status: &AppStatus, ctx: &egui::Context) {
        let busy = self.busy.is_some();
        ui.horizontal_wrapped(|ui| {
            let install_label = if status.installed_version.is_none() {
                "Install"
            } else if status.update_available {
                "Update"
            } else {
                "Reinstall"
            };
            let enabled = !busy && status.plan.is_some() && self.cfg.vt_key().is_some();
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(
                        RichText::new(install_label).strong().color(Color32::BLACK),
                    )
                    .fill(ACCENT)
                    .corner_radius(CornerRadius::same(8))
                    .min_size(Vec2::new(140.0, 34.0)),
                )
                .clicked()
            {
                self.spawn_install(ctx, status.id.clone());
            }

            if status.installed_version.is_some()
                && ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(RichText::new("Open").color(TEXT))
                            .corner_radius(CornerRadius::same(8))
                            .min_size(Vec2::new(90.0, 34.0)),
                    )
                    .clicked()
            {
                match ops::open_app(&status.id, &self.paths) {
                    Ok(()) => self.toast(format!("Opened {}.", status.name), true),
                    Err(error) => self.toast(format!("Could not open {}: {error}", status.name), false),
                }
            }

            if status.installed_version.is_some()
                && ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(RichText::new("Remove").color(BAD))
                            .corner_radius(CornerRadius::same(8))
                            .min_size(Vec2::new(120.0, 34.0)),
                    )
                    .clicked()
            {
                self.confirm_uninstall = Some(status.id.clone());
            }

            if status.plan.is_some() && self.cfg.vt_key().is_none() {
                ui.label(
                    RichText::new("VirusTotal key required")
                        .small()
                        .color(WARN),
                );
            }
        });
        if let Some(error) = &status.error {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Note: {error}")).small().color(WARN));
        }
    }

    fn info_card(&self, ui: &mut egui::Ui, status: &AppStatus) {
        egui::Frame::new()
            .fill(CARD)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 12))
            .stroke(Stroke::new(1.0, BORDER))
            .show(ui, |ui| {
                egui::Grid::new("info")
                    .num_columns(2)
                    .spacing([24.0, 7.0])
                    .show(ui, |ui| {
                        row(ui, "Installed", status.installed_version.as_deref().unwrap_or("–"), TEXT);
                        row(
                            ui,
                            "Latest version",
                            status.latest_version.as_deref().unwrap_or("–"),
                            if status.update_available { WARN } else { TEXT },
                        );
                        row(
                            ui,
                            "Release",
                            status.latest_tag.as_deref().unwrap_or("–"),
                            MUTED,
                        );
                        let sha = status
                            .latest_sha256
                            .as_deref()
                            .map(|s| format!("{}…", &s[..16.min(s.len())]))
                            .unwrap_or_else(|| "–".into());
                        ui.label(RichText::new("SHA-256").color(MUTED));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(sha).monospace().color(TEXT));
                            if let Some(sha) = &status.latest_sha256 {
                                if ui.small_button("copy").clicked() {
                                    ui.ctx().copy_text(sha.clone());
                                }
                            }
                        });
                        ui.end_row();
                    });
            });
    }

    fn security_card(&mut self, ui: &mut egui::Ui, status: &AppStatus) {
        egui::Frame::new()
            .fill(CARD)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 12))
            .stroke(Stroke::new(1.0, BORDER))
            .show(ui, |ui| {
                ui.label(RichText::new("Security check").strong().color(TEXT));
                ui.add_space(2.0);
                check_line(
                    ui,
                    true,
                    "SHA-256 against the release SHA256SUMS.txt (GitHub digest as cross-check)",
                );
                check_line(
                    ui,
                    self.cfg.vt_key().is_some(),
                    if self.cfg.vt_key().is_some() {
                        "VirusTotal: file report and URL scan, content-hash verification"
                    } else {
                        "VirusTotal: no API key configured – installs locked"
                    },
                );
                check_line(
                    ui,
                    true,
                    "No root; installs only into ~/.local",
                );
                if self.cfg.vt_key().is_some() {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        if let Some(sha) = &status.latest_sha256 {
                            let url = format!("https://www.virustotal.com/gui/file/{sha}");
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("VirusTotal permalink").small().color(ACCENT),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                open_url(&url);
                            }
                        }
                        let busy = self.busy.is_some();
                        let ctx = ui.ctx().clone();
                        let id = status.id.clone();
                        if ui
                            .add_enabled(!busy, egui::Button::new("Re-check VirusTotal").small())
                            .clicked()
                        {
                            self.spawn_vt_recheck(&ctx, id);
                        }
                    });
                }
            });
    }

    fn settings_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.label(
            RichText::new(
                "Configuration is stored in ~/.config/artcraft-hub/config.toml with mode 0600.",
            )
            .small()
            .color(MUTED),
        );
        ui.add_space(12.0);

        egui::Frame::new()
            .fill(CARD)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 12))
            .stroke(Stroke::new(1.0, BORDER))
            .show(ui, |ui| {
                ui.label(RichText::new("VirusTotal").strong().color(TEXT));
                ui.label(
                    RichText::new(
                        "Public API: 4 requests per minute, 500 per day. The key is only used for checks; files are never uploaded.",
                    )
                    .small()
                    .color(MUTED),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.vt_key)
                            .password(true)
                            .hint_text("VirusTotal API key")
                            .desired_width(420.0),
                    );
                    if ui.button("Create key").clicked() {
                        open_url("https://www.virustotal.com/gui/my-apikey");
                    }
                });
                if self.cfg.virustotal_api_key.trim().is_empty()
                    && self.cfg.vt_key().is_some()
                {
                    ui.label(
                        RichText::new(
                            "Currently VIRUSTOTAL_API_KEY from the environment is used.",
                        )
                        .small()
                        .color(MUTED),
                    );
                }
            });
        ui.add_space(10.0);

        egui::Frame::new()
            .fill(CARD)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 12))
            .stroke(Stroke::new(1.0, BORDER))
            .show(ui, |ui| {
                ui.label(RichText::new("GitHub").strong().color(TEXT));
                ui.label(
                    RichText::new(
                        "Optional. A token raises the API limit from 60 to 5,000 requests per hour; without one the release feed is used as fallback.",
                    )
                    .small()
                    .color(MUTED),
                );
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.github_token)
                        .password(true)
                        .hint_text("GitHub token (optional)")
                        .desired_width(420.0),
                );
            });
        ui.add_space(10.0);

        egui::Frame::new()
            .fill(CARD)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 12))
            .stroke(Stroke::new(1.0, BORDER))
            .show(ui, |ui| {
                ui.label(RichText::new("Behaviour").strong().color(TEXT));
                ui.checkbox(
                    &mut self.settings.require_sums,
                    "Require SHA256SUMS.txt",
                );
                ui.checkbox(
                    &mut self.settings.install_1024,
                    "Install 1024 px icon from the repository",
                );
                ui.horizontal(|ui| {
                    ui.label("Check interval:");
                    ui.add(
                        egui::DragValue::new(&mut self.settings.check_interval)
                            .range(5..=1440)
                            .suffix(" min"),
                    );
                });
            });
        ui.add_space(10.0);

        egui::Frame::new()
            .fill(Color32::from_rgb(46, 35, 20))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 12))
            .stroke(Stroke::new(1.0, WARN.linear_multiply(0.5)))
            .show(ui, |ui| {
                ui.label(RichText::new("Advanced").strong().color(WARN));
                ui.checkbox(
                    &mut self.settings.allow_unknown,
                    "Allow unclear VirusTotal results",
                );
                ui.label(
                    RichText::new(
                        "Not recommended: if VirusTotal cannot confirm the content, the app is installed anyway.",
                    )
                    .small()
                    .color(MUTED),
                );
            });

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("Save").strong().color(Color32::BLACK))
                        .fill(ACCENT)
                        .corner_radius(CornerRadius::same(8))
                        .min_size(Vec2::new(130.0, 34.0)),
                )
                .clicked()
            {
                self.settings.apply(&mut self.cfg);
                match self.cfg.save(&self.paths) {
                    Ok(()) => {
                        self.toast("Settings saved.", true);
                        self.spawn_check(ui.ctx(), false);
                    }
                    Err(error) => self.toast(format!("Saving failed: {error}"), false),
                }
            }
            if ui.button("Open config folder").clicked() {
                open_url(&format!("file://{}", self.paths.config_dir.display()));
            }
            if ui.button("Reset").clicked() {
                self.settings = SettingsDraft::from_config(&self.cfg);
            }
        });
    }

    fn update_window(&mut self, ctx: &egui::Context) {
        if !self.show_updates {
            return;
        }
        let updates: Vec<AppStatus> = self
            .statuses
            .iter()
            .filter(|s| s.update_available)
            .cloned()
            .collect();
        if updates.is_empty() {
            self.show_updates = false;
            return;
        }
        let mut open = true;
        egui::Window::new("Updates available")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(520.0)
            .open(&mut open)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(18))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} app(s) can be updated.",
                        updates.len()
                    ))
                    .color(TEXT),
                );
                ui.label(
                    RichText::new("Every update is re-checked via SHA-256 and VirusTotal.")
                        .small()
                        .color(MUTED),
                );
                ui.add_space(8.0);
                for status in &updates {
                    let mut checked = self
                        .update_selection
                        .get(&status.id)
                        .copied()
                        .unwrap_or(true);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut checked, "");
                        self.icon(ui, status, 28.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&status.name).strong());
                            ui.label(
                                RichText::new(ops::version_label(
                                    status.installed_version.as_deref(),
                                    status.latest_version.as_deref(),
                                ))
                                .small()
                                .color(MUTED),
                            );
                        });
                    });
                    self.update_selection.insert(status.id.clone(), checked);
                    ui.add_space(2.0);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Select all").clicked() {
                        for status in &updates {
                            self.update_selection.insert(status.id.clone(), true);
                        }
                    }
                    if ui.button("None").clicked() {
                        for status in &updates {
                            self.update_selection.insert(status.id.clone(), false);
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let selected: Vec<String> = updates
                            .iter()
                            .filter(|s| {
                                self.update_selection
                                    .get(&s.id)
                                    .copied()
                                    .unwrap_or(true)
                            })
                            .map(|s| s.id.clone())
                            .collect();
                        let enabled = !selected.is_empty() && self.busy.is_none();
                        if ui
                            .add_enabled(
                                enabled,
                                egui::Button::new(
                                    RichText::new(format!(
                                        "Update now ({})",
                                        selected.len()
                                    ))
                                    .strong()
                                    .color(Color32::BLACK),
                                )
                                .fill(ACCENT)
                                .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            self.show_updates = false;
                            self.spawn_upgrade(ctx, selected);
                        }
                        if ui.button("Later").clicked() {
                            self.show_updates = false;
                        }
                    });
                });
            });
        if !open {
            self.show_updates = false;
        }
    }

    fn uninstall_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_uninstall.clone() else {
            return;
        };
        let name = catalog::by_id(&id)
            .map(|a| a.name.to_string())
            .unwrap_or_else(|| id.clone());
        let mut open = true;
        egui::Window::new("Remove app")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{name} will be removed from ~/.local/opt/artcraft. App settings are kept."
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Remove").color(Color32::WHITE))
                                .fill(BAD.linear_multiply(0.8))
                                .corner_radius(CornerRadius::same(8)),
                        )
                        .clicked()
                    {
                        match ops::uninstall(&id, &self.paths) {
                            Ok(removed) => {
                                self.toast(
                                    if removed {
                                        format!("{name} was removed.")
                                    } else {
                                        format!("{name} was not installed.")
                                    },
                                    true,
                                );
                                self.spawn_check(ctx, false);
                            }
                            Err(error) => {
                                self.toast(format!("Removal failed: {error}"), false)
                            }
                        }
                        self.confirm_uninstall = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_uninstall = None;
                    }
                });
            });
        if !open {
            self.confirm_uninstall = None;
        }
    }

    fn log_window(&mut self, ctx: &egui::Context) {
        if !self.show_log {
            return;
        }
        let mut open = true;
        egui::Window::new("Log")
            .open(&mut open)
            .default_width(640.0)
            .default_height(360.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(14))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in &self.log {
                            ui.label(
                                RichText::new(line)
                                    .monospace()
                                    .size(12.0)
                                    .color(MUTED),
                            );
                        }
                    });
            });
        self.show_log = open;
    }

    fn toast_layer(&mut self, ctx: &egui::Context) {
        let Some((text, ok, at)) = self.toast.clone() else {
            return;
        };
        if at.elapsed() > Duration::from_secs(8) {
            self.toast = None;
            return;
        }
        egui::Area::new(egui::Id::unique("artcraft-toast"))
            .anchor(Align2::CENTER_BOTTOM, [0.0, -54.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(if ok {
                        Color32::from_rgb(24, 52, 38)
                    } else {
                        Color32::from_rgb(58, 30, 34)
                    })
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(16, 10))
                    .stroke(Stroke::new(
                        1.0,
                        if ok { OK.linear_multiply(0.6) } else { BAD.linear_multiply(0.6) },
                    ))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(truncate(&text, 160))
                                .color(if ok { OK } else { BAD }),
                        );
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(400));
    }
}

impl eframe::App for HubApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain(&ctx);
        if !self.icons_started {
            self.icons_started = true;
            self.spawn_check(&ctx, false);
        }

        self.top_bar(ui);
        self.bottom_bar(ui);
        self.sidebar(ui);
        self.central(ui);

        self.update_window(&ctx);
        self.uninstall_window(&ctx);
        self.log_window(&ctx);
        self.toast_layer(&ctx);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn row(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    ui.label(RichText::new(label).color(MUTED));
    ui.label(RichText::new(value).color(color));
    ui.end_row();
}

fn check_line(ui: &mut egui::Ui, ok: bool, text: &str) {
    ui.horizontal_wrapped(|ui| {
        status_glyph(ui, ok);
        ui.label(RichText::new(text).small().color(MUTED));
    });
}

fn status_glyph(ui: &mut egui::Ui, ok: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let color = if ok { OK } else { BAD };
    let c = rect.center();
    if ok {
        painter.line_segment([c + Vec2::new(-5.0, 1.0), c + Vec2::new(-1.5, 5.0)], Stroke::new(2.0, color));
        painter.line_segment([c + Vec2::new(-1.5, 5.0), c + Vec2::new(5.0, -4.0)], Stroke::new(2.0, color));
    } else {
        painter.line_segment([c + Vec2::new(-4.0, -4.0), c + Vec2::new(4.0, 4.0)], Stroke::new(2.0, color));
        painter.line_segment([c + Vec2::new(-4.0, 4.0), c + Vec2::new(4.0, -4.0)], Stroke::new(2.0, color));
    }
}

fn side_status(status: &AppStatus) -> (String, Color32) {
    if status.update_available {
        (
            format!(
                "Update {} -> {}",
                status.installed_version.as_deref().unwrap_or("?"),
                status.latest_version.as_deref().unwrap_or("?")
            ),
            WARN,
        )
    } else if let Some(version) = &status.installed_version {
        (format!("Installed - {version}"), OK)
    } else if let Some(error) = &status.error {
        if error.contains("No release") {
            ("No release yet".into(), MUTED)
        } else {
            ("Check failed".into(), MUTED)
        }
    } else if let Some(latest) = &status.latest_version {
        (format!("Not installed - {latest}"), MUTED)
    } else {
        ("Unknown".into(), MUTED)
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars()
        .take(max.saturating_sub(1))
        .collect::<String>()
        + "…"
}

fn open_url(url: &str) {
    if url.starts_with("https://") || url.starts_with("file://") {
        let _ = std::process::Command::new("xdg-open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

fn load_image(path: &PathBuf) -> Option<egui::ColorImage> {
    let bytes = std::fs::read(path).ok()?;
    let image = image::load_from_memory(&bytes).ok()?;
    Some(to_color_image(&image))
}

fn to_color_image(image: &image::DynamicImage) -> egui::ColorImage {
    let image = image.thumbnail(256, 256).to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw())
}

/// Returns the icon bytes embedded in the binary (padding-free WebP, converted
/// to PNG at build time). These are always available, even offline.
fn embedded_icon(id: &str) -> Option<&'static [u8]> {
    match id {
        "photocraft" => Some(include_bytes!("../../../assets/icons/photocraft.png")),
        "vectorcraft" => Some(include_bytes!("../../../assets/icons/vectorcraft.png")),
        "filmcraft" => Some(include_bytes!("../../../assets/icons/filmcraft.png")),
        "lightcraft" => Some(include_bytes!("../../../assets/icons/lightcraft.png")),
        "pdfcraft" => Some(include_bytes!("../../../assets/icons/pdfcraft.png")),
        "effectcraft" => Some(include_bytes!("../../../assets/icons/effectcraft.png")),
        "designcraft" => Some(include_bytes!("../../../assets/icons/designcraft.png")),
        "figcraft" => Some(include_bytes!("../../../assets/icons/figcraft.png")),
        "wordcraft" => Some(include_bytes!("../../../assets/icons/wordcraft.png")),
        "soundcraft" => Some(include_bytes!("../../../assets/icons/soundcraft.png")),
        "deckcraft" => Some(include_bytes!("../../../assets/icons/deckcraft.png")),
        "cadcraft" => Some(include_bytes!("../../../assets/icons/cadcraft.png")),
        "gridcraft" => Some(include_bytes!("../../../assets/icons/gridcraft.png")),
        _ => None,
    }
}

/// Finds an icon that is already on disk: installed hicolor theme, the app's
/// version directory, or the hub's own cache.
fn local_icon(paths: &Paths, id: &str, bundle: &str) -> Option<PathBuf> {
    // Prefer the padding-free cached WebP (saved as PNG) over the hicolor PNGs.
    let cached = paths.icon_cache_dir().join(format!("{id}.png"));
    if cached.is_file() {
        return Some(cached);
    }
    let sizes = ["1024x1024", "512x512", "256x256", "128x128", "64x64"];
    for size in sizes {
        let candidate = paths
            .icons_dir
            .join("hicolor")
            .join(size)
            .join("apps")
            .join(format!("{bundle}.png"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let root = paths.app_dir(id);
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    versions.sort();
    versions.reverse();
    for version in versions {
        for size in sizes {
            let candidate = version
                .join("share/icons/hicolor")
                .join(size)
                .join("apps")
                .join(format!("{bundle}.png"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
