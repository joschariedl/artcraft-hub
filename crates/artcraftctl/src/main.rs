//! `artcraftctl` – command line interface used by the Omarchy bar widget and
//! for automation.

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::{Parser, Subcommand};
use serde::Serialize;

use artcraft_core::config::Config;
use artcraft_core::ops::{self, AppStatus, UpgradeReport};
use artcraft_core::paths::Paths;
use artcraft_core::progress::Progress;
use artcraft_core::state::State;
use artcraft_core::time::now_unix;

#[derive(Parser)]
#[command(
    name = "artcraftctl",
    version,
    about = "Install and update ArtCraft apps",
    long_about = "Installed die 13 ArtCraft-Apps aus ihren GitHub-Releases nach ~/.local/opt/artcraft. \
                  Every install is verified via SHA-256 and VirusTotal."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// All apps und ihren Zustand auflisten
    List {
        /// Netzwerkabfrage erzwingen
        #[arg(long)]
        refresh: bool,
        /// Ausgabe als JSON
        #[arg(long)]
        json: bool,
    },
    /// Short status for the Omarchy bar (from cache by default)
    Status {
        /// Netzwerkabfrage erzwingen
        #[arg(long)]
        refresh: bool,
        /// Ausgabe als JSON
        #[arg(long)]
        json: bool,
    },
    /// Nach Updates suchen
    Check {
        #[arg(long)]
        json: bool,
    },
    /// Install an app
    Install {
        app: String,
        /// Install even if the latest version is already present
        #[arg(long)]
        force: bool,
        /// Without confirmation
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Update one app or all apps
    Upgrade {
        /// Only this app
        app: Option<String>,
        /// All apps
        #[arg(long)]
        all: bool,
        /// Without confirmation
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Eine Remove app
    Uninstall {
        app: String,
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Check environment, keys and tools
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Show or change configuration
    Config {
        #[arg(long)]
        show: bool,
        #[arg(long)]
        set_vt_key: Option<String>,
        #[arg(long)]
        set_github_token: Option<String>,
        /// Allow unclear VirusTotal results (nicht empfohlen)
        #[arg(long)]
        allow_unknown: Option<bool>,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Fehler: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let paths = Paths::from_env();
    let mut cfg = Config::load(&paths)?;

    match cli.command {
        Command::List { refresh, json } => {
            let statuses = check(&paths, &cfg, refresh);
            if json {
                print_json(&StatusJson::from(&statuses, &cfg, &paths))?;
            } else {
                print_table(&statuses);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Status { refresh, json } => {
            let statuses = check(&paths, &cfg, refresh);
            if json {
                print_json(&StatusJson::from(&statuses, &cfg, &paths))?;
            } else {
                print_table(&statuses);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Check { json } => {
            let statuses = check(&paths, &cfg, true);
            if json {
                print_json(&StatusJson::from(&statuses, &cfg, &paths))?;
            } else {
                print_table(&statuses);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Install { app, force, yes } => {
            let app = resolve_app(&app)?;
            if !yes && !confirm(&format!("Install {}?", app.name))? {
                eprintln!("Aborted.");
                return Ok(ExitCode::SUCCESS);
            }
            if cfg.vt_key().is_none() {
                eprintln!(
                    "{}",
                    artcraft_core::Error::VirusTotalNotConfigured
                );
                eprintln!(
                    "Configure a key with: artcraftctl config --set-vt-key <KEY>"
                );
                return Ok(ExitCode::from(2));
            }
            let progress = reporter();
            match ops::install(app.id, &paths, &cfg, force, &progress) {
                Ok(outcome) => {
                    if outcome.already_installed {
                        println!(
                            "{} {} is already installed (--force to reinstall).",
                            app.name, outcome.version
                        );
                    } else {
                        println!("{} {} was installed.", app.name, outcome.version);
                    }
                    if let Some(vt) = outcome.vt {
                        println!(
                            "VirusTotal: {} – {}",
                            vt.verdict.label(),
                            vt.permalink()
                        );
                    }
                    Ok(ExitCode::SUCCESS)
                }
                Err(error) => Err(Box::new(error)),
            }
        }
        Command::Upgrade { app, all, yes } => {
            if !all && app.is_none() {
                // `upgrade` without arguments means everything.
            }
            if let Some(id) = app {
                let app = resolve_app(&id)?;
                if !yes && !confirm(&format!("Update {}?", app.name))? {
                    eprintln!("Aborted.");
                    return Ok(ExitCode::SUCCESS);
                }
                let progress = reporter();
                match ops::install(app.id, &paths, &cfg, true, &progress) {
                    Ok(outcome) => {
                        println!("{} {} was installed.", app.name, outcome.version);
                        Ok(ExitCode::SUCCESS)
                    }
                    Err(error) => Err(Box::new(error)),
                }
            } else {
                if !yes {
                    let statuses = check(&paths, &cfg, true);
                    let updates: Vec<&AppStatus> =
                        statuses.iter().filter(|s| s.update_available).collect();
                    if updates.is_empty() {
                        println!("No updates available.");
                        return Ok(ExitCode::SUCCESS);
                    }
                    for status in &updates {
                        println!(
                            "  {} {}",
                            status.name,
                            ops::version_label(
                                status.installed_version.as_deref(),
                                status.latest_version.as_deref()
                            )
                        );
                    }
                    if !confirm(&format!("Update {} app(s)?", updates.len()))? {
                        eprintln!("Aborted.");
                        return Ok(ExitCode::SUCCESS);
                    }
                }
                let progress = reporter();
                let report = ops::upgrade_all(&paths, &cfg, &progress);
                print_report(&report);
                if report.failed.is_empty() {
                    Ok(ExitCode::SUCCESS)
                } else {
                    Ok(ExitCode::FAILURE)
                }
            }
        }
        Command::Uninstall { app, yes } => {
            let app = resolve_app(&app)?;
            if !yes && !confirm(&format!("Really remove {}?", app.name))? {
                eprintln!("Aborted.");
                return Ok(ExitCode::SUCCESS);
            }
            if ops::uninstall(app.id, &paths)? {
                println!("{} was removed.", app.name);
            } else {
                println!("{} was not installed.", app.name);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Doctor { json } => {
            let items = ops::doctor(&paths, &cfg);
            if json {
                print_json(&items)?;
            } else {
                for item in items {
                    println!(
                        "{} {:<32} {}",
                        if item.ok { "✓" } else { "✗" },
                        item.name,
                        item.detail
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Config {
            show,
            set_vt_key,
            set_github_token,
            allow_unknown,
        } => {
            let mut changed = false;
            if let Some(key) = set_vt_key {
                cfg.virustotal_api_key = key.trim().to_string();
                changed = true;
            }
            if let Some(token) = set_github_token {
                cfg.github_token = token.trim().to_string();
                changed = true;
            }
            if let Some(allow) = allow_unknown {
                cfg.vt.allow_unknown = allow;
                changed = true;
            }
            if changed {
                cfg.save(&paths)?;
                println!("Configuration saved: {}", paths.config_file().display());
            }
            if show || !changed {
                println!("Configuration: {}", paths.config_file().display());
                println!(
                    "  VirusTotal key: {}",
                    if cfg.vt_key().is_some() {
                        "configured"
                    } else {
                        "missing"
                    }
                );
                println!(
                    "  GitHub token:   {}",
                    if cfg.github_token().is_some() {
                        "configured"
                    } else {
                        "missing (optional)"
                    }
                );
                println!(
                    "  Allow unclear VT results: {}",
                    if cfg.vt.allow_unknown { "ja" } else { "no" }
                );
                println!("  Check interval:  {} minutes", cfg.check_interval_minutes);
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn resolve_app(id: &str) -> Result<&'static artcraft_core::catalog::AppDef, String> {
    artcraft_core::catalog::by_id(id)
        .ok_or_else(|| format!("Unknown app: {id}. Valid apps: {}", app_ids()))
}

fn app_ids() -> String {
    artcraft_core::catalog::SUITE
        .iter()
        .map(|a| a.id)
        .collect::<Vec<_>>()
        .join(", ")
}

fn check(paths: &Paths, cfg: &Config, force: bool) -> Vec<AppStatus> {
    let progress = reporter();
    ops::check_all(paths, cfg, force, &progress)
}

fn reporter() -> impl Fn(Progress) + Send + Sync {
    let last_percent = AtomicU64::new(u64::MAX);
    move |event| match event {
        Progress::Message(message) => {
            eprintln!("{message}");
            last_percent.store(u64::MAX, Ordering::Relaxed);
        }
        Progress::Download { received, total } => {
            if let Some(total) = total.filter(|t| *t > 0) {
                let percent = received.saturating_mul(100) / total;
                if percent != last_percent.load(Ordering::Relaxed) {
                    last_percent.store(percent, Ordering::Relaxed);
                    eprint!("\rDownload: {percent:>3} %");
                    let _ = std::io::stderr().flush();
                    if percent >= 100 {
                        eprintln!();
                    }
                }
            }
        }
    }
}

fn confirm(question: &str) -> Result<bool, Box<dyn std::error::Error>> {
    if !std::io::stdin().is_terminal() {
        eprintln!("No terminal for the prompt; please use --yes.");
        return Ok(false);
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let answer = line.trim().to_lowercase();
    Ok(answer == "y" || answer == "yes")
}

fn print_json<T: Serialize>(value: &T) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[derive(Serialize)]
struct StatusJson<'a> {
    schema: u32,
    checked_at: Option<i64>,
    now: i64,
    vt_configured: bool,
    update_count: usize,
    installed_count: usize,
    apps: Vec<AppJson<'a>>,
}

#[derive(Serialize)]
struct AppJson<'a> {
    id: &'a str,
    name: &'a str,
    installed_version: Option<&'a str>,
    latest_version: Option<&'a str>,
    update_available: bool,
    not_installed: bool,
    error: Option<&'a str>,
}

impl<'a> StatusJson<'a> {
    fn from(statuses: &'a [AppStatus], cfg: &Config, paths: &Paths) -> Self {
        Self {
            schema: 1,
            checked_at: State::load(paths).checked_at,
            now: now_unix(),
            vt_configured: cfg.vt_key().is_some(),
            update_count: statuses.iter().filter(|s| s.update_available).count(),
            installed_count: statuses
                .iter()
                .filter(|s| s.installed_version.is_some())
                .count(),
            apps: statuses
                .iter()
                .map(|s| AppJson {
                    id: &s.id,
                    name: &s.name,
                    installed_version: s.installed_version.as_deref(),
                    latest_version: s.latest_version.as_deref(),
                    update_available: s.update_available,
                    not_installed: s.not_installed,
                    error: s.error.as_deref(),
                })
                .collect(),
        }
    }
}

fn print_table(statuses: &[AppStatus]) {
    println!(
        "{:<12} {:<14} {:<14} Status",
        "App", "Installed", "Latest"
    );
    for status in statuses {
        let state = if let Some(error) = &status.error {
            format!("Fehler: {error}")
        } else if status.update_available {
            "Update available".to_string()
        } else if status.not_installed {
            "not installed".to_string()
        } else {
            "up to date".to_string()
        };
        println!(
            "{:<12} {:<14} {:<14} {}",
            status.id,
            status.installed_version.as_deref().unwrap_or("–"),
            status.latest_version.as_deref().unwrap_or("–"),
            truncate(&state, 80)
        );
    }
    let updates = statuses.iter().filter(|s| s.update_available).count();
    if updates > 0 {
        println!("\n{updates} update(s) available: artcraftctl upgrade");
    }
}

fn print_report(report: &UpgradeReport) {
    if report.updated.is_empty() && report.failed.is_empty() {
        println!("Nothing to do.");
        return;
    }
    for id in &report.updated {
        println!("✓ {id} updated");
    }
    for failure in &report.failed {
        println!("✗ {}: {}", failure.name, failure.error);
    }
    if !report.skipped.is_empty() {
        println!("{} app(s) were already up to date.", report.skipped.len());
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
}
