//! High-level operations shared by the GUI, the CLI and the Omarchy widget.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::{self, AppDef};
use crate::config::Config;
use crate::download;
use crate::error::{Error, Result};
use crate::github::{self, ReleasePlan};
use crate::install;
use crate::paths::Paths;
use crate::progress::{self, Progress, ProgressFn};
use crate::state::{self, State};
use crate::time::now_unix;
use crate::version::ReleaseVersion;
use crate::vt::{Verdict, VtClient, VtResult};

/// One row in the hub / one entry of `artcraftctl status`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppStatus {
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub color: [u8; 3],
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
    pub latest_tag: Option<String>,
    pub latest_sha256: Option<String>,
    /// An update from an installed version is available.
    pub update_available: bool,
    /// The app is not installed at all.
    pub not_installed: bool,
    pub error: Option<String>,
    /// Full plan for display/installation; not part of the JSON status.
    #[serde(skip)]
    pub plan: Option<ReleasePlan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallOutcome {
    pub app_id: String,
    pub version: String,
    pub already_installed: bool,
    pub downloaded: bool,
    pub vt: Option<VtResult>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpgradeReport {
    pub updated: Vec<String>,
    pub failed: Vec<FailedApp>,
    pub skipped: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FailedApp {
    pub id: String,
    pub name: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DoctorItem {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// Checks every app. Uses cached metadata while it is fresh; `force` always
/// asks the network.
pub fn check_all(
    paths: &Paths,
    cfg: &Config,
    force: bool,
    progress: ProgressFn<'_>,
) -> Vec<AppStatus> {
    let now = now_unix();
    let mut state = State::load(paths);
    state.reconcile(paths);
    let ttl = cfg.metadata_ttl_secs();
    let mut out = Vec::with_capacity(catalog::SUITE.len());

    for app in catalog::SUITE.iter() {
        let installed = state.installed.get(app.id).cloned();
        let mut error = None;

        let plan = if !force {
            state.cached_plan(app.id, ttl, now).cloned()
        } else {
            None
        };
        let plan = match plan {
            Some(plan) => Some(plan),
            None => {
                progress::message(progress, format!("Checking {} …", app.name));
                match github::resolve(app, cfg) {
                    Ok(plan) => {
                        state.store_plan(&plan, now);
                        Some(plan)
                    }
                    Err(err) => {
                        state.releases.remove(app.id);
                        error = Some(err.to_string());
                        None
                    }
                }
            }
        };
        if let Some(plan) = &plan {
            // Keep the download cache bounded to the latest known version.
            prune_download_cache(paths, app, &plan.asset_name);
        }

        out.push(status_for(app, installed.as_ref(), plan, error));
    }

    state.checked_at = Some(now);
    if let Err(error) = state.save(paths) {
        progress::message(progress, format!("Note: status not saved ({error})"));
    }
    out
}

fn status_for(
    app: &AppDef,
    installed: Option<&state::InstalledApp>,
    plan: Option<ReleasePlan>,
    error: Option<String>,
) -> AppStatus {
    let update_available = match (installed, plan.as_ref()) {
        (Some(installed), Some(plan)) => plan.version().is_newer_than(&installed.version),
        _ => false,
    };
    AppStatus {
        id: app.id.to_string(),
        name: app.name.to_string(),
        tagline: app.tagline.to_string(),
        color: app.color,
        installed_version: installed.map(|i| i.version.clone()),
        latest_version: plan.as_ref().map(|p| p.version.clone()),
        latest_tag: plan.as_ref().map(|p| p.tag.clone()),
        latest_sha256: plan.as_ref().map(|p| p.sha256.clone()),
        update_available,
        not_installed: installed.is_none(),
        error,
        plan,
    }
}

/// Installs or reinstalls one app. With `force = false` an existing latest
/// version is left alone.
pub fn install(
    app_id: &str,
    paths: &Paths,
    cfg: &Config,
    force: bool,
    progress: ProgressFn<'_>,
) -> Result<InstallOutcome> {
    let app = catalog::by_id(app_id)
        .ok_or_else(|| Error::Other(format!("Unknowne App: {app_id}")))?;

    if cfg.vt_key().is_none() {
        return Err(Error::VirusTotalNotConfigured);
    }

    let mut state = State::load(paths);
    state.reconcile(paths);

    let now = now_unix();
    let plan = match state.cached_plan(app.id, cfg.metadata_ttl_secs(), now) {
        Some(plan) if !force => plan.clone(),
        _ => {
            progress::message(progress, format!("Resolving latest version of {} …", app.name));
            let plan = github::resolve(app, cfg)?;
            state.store_plan(&plan, now);
            let _ = state.save(paths);
            plan
        }
    };

    if !force {
        if let Some(installed) = state.installed.get(app.id) {
            if installed.version == plan.version {
                return Ok(InstallOutcome {
                    app_id: app.id.to_string(),
                    version: plan.version,
                    already_installed: true,
                    downloaded: false,
                    vt: None,
                });
            }
        }
    }

    let outcome = install_with_plan(app, &plan, paths, cfg, &mut state, progress)?;
    state.installed.insert(
        app.id.to_string(),
        state::installed_record(paths, &plan, &paths.app_version_dir(app.id, &plan.version)),
    );
    state.store_plan(&plan, now_unix());
    state.save(paths)?;
    Ok(outcome)
}

/// Installs a plan that has already been resolved (used by bulk upgrades).
fn install_with_plan(
    app: &AppDef,
    plan: &ReleasePlan,
    paths: &Paths,
    cfg: &Config,
    state: &mut State,
    progress: ProgressFn<'_>,
) -> Result<InstallOutcome> {
    progress::message(
        progress,
        format!("{} {} is being checked …", app.name, plan.version),
    );

    // 1. VirusTotal, before anything is downloaded.
    let mut vt = VtClient::new(paths, cfg)?;
    let report = |text: &str| progress(Progress::Message(text.to_string()));
    let verdict = vt.check(&plan.sha256, &plan.asset_url, &report)?;
    if verdict.verdict == Verdict::Blocked {
        return Err(Error::VirusTotalBlocked(verdict.detail.clone()));
    }
    if verdict.verdict == Verdict::Unknown {
        progress::message(
            progress,
            format!(
                "Warning: VirusTotal is unclear ({}), continuing because allow_unknown is set.",
                verdict.detail
            ),
        );
    } else {
        progress::message(
            progress,
            format!(
                "VirusTotal: clean (0 of {} engines).",
                verdict.stats.total()
            ),
        );
    }

    // 2. Download into the cache, hashing while writing.
    paths.ensure()?;
    let archive = paths.download_dir().join(&plan.asset_name);
    let mut downloaded = false;
    if archive.is_file() && download::has_checksum(&archive, &plan.sha256) {
        progress::message(progress, "Archive is already in the cache and verified.");
    } else {
        if archive.exists() {
            let _ = std::fs::remove_file(&archive);
        }
        progress::message(
            progress,
            format!(
                "Downloading {} ({} MB) …",
                plan.asset_name,
                plan.asset_size / 1024 / 1024
            ),
        );
        let agent = crate::net::agent(std::time::Duration::from_secs(600));
        download::fetch(
            &agent,
            &plan.asset_url,
            &archive,
            &plan.sha256,
            &|event| progress(event),
        )?;
        downloaded = true;
        progress::message(progress, "SHA-256 matches SHA256SUMS.txt.");
    }

    // 2b. Optional GPG signature check.
    install::verify_gpg_signature(plan, &archive, progress)?;

    // 3. Extract and register.
    let installed = install::install_archive(app, plan, &archive, paths, cfg, progress)?;
    state.installed.insert(app.id.to_string(), installed);
    // Keep the download cache bounded: only the just-installed archive survives.
    prune_download_cache(paths, app, &plan.asset_name);
    progress::message(progress, format!("{} {} is installed.", app.name, plan.version));

    Ok(InstallOutcome {
        app_id: app.id.to_string(),
        version: plan.version.clone(),
        already_installed: false,
        downloaded,
        vt: Some(verdict),
    })
}

/// Installs every available update, skipping failures.
pub fn upgrade_all(paths: &Paths, cfg: &Config, progress: ProgressFn<'_>) -> UpgradeReport {
    let statuses = check_all(paths, cfg, true, progress);
    let mut report = UpgradeReport::default();
    let mut state = State::load(paths);
    state.reconcile(paths);

    for status in statuses {
        let Some(app) = catalog::by_id(&status.id) else {
            continue;
        };
        if let Some(error) = &status.error {
            report.failed.push(FailedApp {
                id: status.id.clone(),
                name: status.name.clone(),
                error: error.clone(),
            });
            continue;
        }
        if !status.update_available {
            if status.installed_version.is_some() {
                report.skipped.push(status.id.clone());
            }
            continue;
        }
        let Some(plan) = status.plan else {
            continue;
        };
        match install_with_plan(app, &plan, paths, cfg, &mut state, progress) {
            Ok(_) => {
                state.store_plan(&plan, now_unix());
                report.updated.push(status.id.clone());
            }
            Err(error) => report.failed.push(FailedApp {
                id: status.id.clone(),
                name: status.name.clone(),
                error: error.to_string(),
            }),
        }
    }

    if let Err(error) = state.save(paths) {
        progress::message(progress, format!("Note: status not saved ({error})"));
    }
    report
}

/// Removes an app and updates the state file.
pub fn uninstall(app_id: &str, paths: &Paths) -> Result<bool> {
    let app = catalog::by_id(app_id)
        .ok_or_else(|| Error::Other(format!("Unknowne App: {app_id}")))?;
    let removed = install::uninstall(app, paths)?;
    // Drop any cached archives of this app as well.
    prune_download_cache(paths, app, "");
    let mut state = State::load(paths);
    state.reconcile(paths);
    state.installed.remove(app.id);
    state.save(paths)?;
    Ok(removed)
}

/// Sanity checks for the CLI's `doctor` command.
pub fn doctor(paths: &Paths, cfg: &Config) -> Vec<DoctorItem> {
    let mut items = Vec::new();

    let mut writable = true;
    if let Err(error) = paths.ensure() {
        writable = false;
        items.push(DoctorItem {
            name: "Verzeichnisse".into(),
            ok: false,
            detail: error.to_string(),
        });
    }
    if writable {
        let probe = paths.cache_dir.join(".doctor-probe");
        let ok = std::fs::write(&probe, b"ok").is_ok();
        let _ = std::fs::remove_file(&probe);
        items.push(DoctorItem {
            name: "Schreibrechte".into(),
            ok,
            detail: paths.cache_dir.display().to_string(),
        });
    }

    items.push(DoctorItem {
        name: "VirusTotal-API-Key".into(),
        ok: cfg.vt_key().is_some(),
        detail: if cfg.vt_key().is_some() {
            "configured".into()
        } else {
            format!("missing – in {} eintragen", paths.config_file().display())
        },
    });

    items.push(DoctorItem {
        name: "GitHub token (optional)".into(),
        ok: cfg.github_token().is_some(),
        detail: if cfg.github_token().is_some() {
            "configured (5.000 Anfragen/Stunde)".into()
        } else {
            "nicht configured (60 Anfragen/Stunde, Feed-Fallback aktiv)".into()
        },
    });

    let path_has_bin = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|p| p == paths.bin_dir))
        .unwrap_or(false);
    items.push(DoctorItem {
        name: "~/.local/bin im PATH".into(),
        ok: path_has_bin,
        detail: if path_has_bin {
            "ja".into()
        } else {
            format!("no – add {}", paths.bin_dir.display())
        },
    });

    items.push(DoctorItem {
        name: "artcraftctl".into(),
        ok: find_in_path("artcraftctl").is_some(),
        detail: find_in_path("artcraftctl")
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not in PATH (needed for the Omarchy plugin)".into()),
    });

    items.push(DoctorItem {
        name: "Omarchy-Plugin".into(),
        ok: paths.omarchy_plugin_dir().join("manifest.json").is_file(),
        detail: paths.omarchy_plugin_dir().display().to_string(),
    });

    for tool in ["update-desktop-database", "gtk-update-icon-cache"] {
        let found = find_in_path(tool);
        items.push(DoctorItem {
            name: tool.into(),
            ok: found.is_some(),
            detail: found
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "optional, nicht gefunden".into()),
        });
    }

    items
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Re-runs the VirusTotal check for an app, ignoring any cached verdict.
pub fn vt_recheck(app_id: &str, paths: &Paths, cfg: &Config, progress: ProgressFn<'_>) -> Result<VtResult> {
    let app = catalog::by_id(app_id)
        .ok_or_else(|| Error::Other(format!("Unknown app: {app_id}")))?;
    let mut state = State::load(paths);
    state.reconcile(paths);
    let now = now_unix();
    let plan = state
        .cached_plan(app.id, cfg.metadata_ttl_secs(), now)
        .cloned()
        .or_else(|| github::resolve(app, cfg).ok())
        .ok_or_else(|| Error::Other(format!("No release plan for {}", app.name)))?;
    let mut vt = VtClient::new(paths, cfg)?;
    vt.invalidate(&plan.sha256);
    let report = |text: &str| progress(Progress::Message(text.to_string()));
    vt.check(&plan.sha256, &plan.asset_url, &report)
}

/// Launches an installed app by running its binary.
pub fn open_app(app_id: &str, paths: &Paths) -> Result<()> {
    let app = catalog::by_id(app_id)
        .ok_or_else(|| Error::Other(format!("Unknown app: {app_id}")))?;
    let dir = installed_dir(paths, app_id)
        .ok_or_else(|| Error::Other(format!("{} is not installed.", app.name)))?;
    // Find the main binary inside the version dir.
    let bin_dir = dir.join("bin");
    let binary = install::find_main_binary(&bin_dir, app)
        .map(|name| bin_dir.join(name))
        .unwrap_or_else(|| paths.bin_dir.join(app.id));
    if !binary.exists() {
        return Err(Error::Other(format!("Binary not found: {}", binary.display())));
    }
    std::process::Command::new(&binary)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error::Other(format!("Could not launch {}: {e}", app.name)))?;
    Ok(())
}

/// Convenience for the GUI: the version dir of an installed app.
pub fn installed_dir(paths: &Paths, id: &str) -> Option<PathBuf> {
    let state = State::load(paths);
    state.installed.get(id).map(|i| i.dir.clone()).or_else(|| {
        let mut state = state;
        state.reconcile(paths);
        state.installed.get(id).map(|i| i.dir.clone())
    })
}

/// Short label such as `0.13.0 → 0.14.0`, used by the CLI and the widget.
pub fn version_label(installed: Option<&str>, latest: Option<&str>) -> String {
    match (installed, latest) {
        (Some(i), Some(l)) => {
            let iv = ReleaseVersion::parse(i);
            let lv = ReleaseVersion::parse(l);
            if lv.is_newer_than(i) {
                format!("{iv} → {lv}")
            } else {
                format!("{iv}")
            }
        }
        (Some(i), None) => i.to_string(),
        (None, Some(l)) => format!("not installed → {l}"),
        (None, None) => "–".to_string(),
    }
}

/// Checks whether a path is inside the hub's install root.
pub fn is_managed(paths: &Paths, path: &Path) -> bool {
    paths.is_managed_path(path)
}

/// Removes cached download archives of `app` that are no longer the current
/// version, so `~/.cache/artcraft-hub/downloads/` cannot grow without bound.
pub fn prune_download_cache(paths: &Paths, app: &AppDef, keep: &str) {
    let dir = paths.download_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let mut stems: Vec<&str> = vec![app.id];
    stems.extend(app.stems.iter().copied());
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name == keep || !name.ends_with(".tar.gz") {
            continue;
        }
        if !stems.iter().any(|stem| name.starts_with(&format!("{stem}-"))) {
            continue;
        }
        let _ = fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_labels_are_readable() {
        assert_eq!(version_label(Some("0.13.0"), Some("0.14.0")), "0.13.0 → 0.14.0");
        assert_eq!(version_label(Some("0.14.0"), Some("0.13.0")), "0.14.0");
        assert_eq!(version_label(None, Some("1.0.0")), "not installed → 1.0.0");
    }
}
