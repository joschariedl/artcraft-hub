//! Persistent state: what is installed and what the last release check found.
//!
//! The filesystem is the source of truth for installations; `state.json` only
//! adds metadata (checksum, tag, install time) and caches release lookups so
//! the Omarchy widget can answer instantly.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::SUITE;
use crate::error::Result;
use crate::github::ReleasePlan;
use crate::paths::{Paths, sanitize_component};
use crate::time::now_unix;
use crate::version::ReleaseVersion;

/// One installed app version.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledApp {
    pub version: String,
    pub tag: String,
    pub sha256: String,
    pub installed_at: i64,
    pub dir: PathBuf,
    pub desktop_file: PathBuf,
}

/// A release lookup cached for `metadata_cache_minutes`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedRelease {
    pub fetched_at: i64,
    pub plan: ReleasePlan,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub installed: BTreeMap<String, InstalledApp>,
    #[serde(default)]
    pub releases: BTreeMap<String, CachedRelease>,
    #[serde(default)]
    pub checked_at: Option<i64>,
}

impl State {
    /// Loads the state file, falling back to an empty state when it is missing
    /// or unreadable. A corrupt cache must never block the app.
    pub fn load(paths: &Paths) -> Self {
        let path = paths.state_file();
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        fs::create_dir_all(&paths.state_dir)?;
        let json = serde_json::to_vec_pretty(self)?;
        write_atomic(&paths.state_file(), &json, 0o644)?;
        Ok(())
    }

    /// Replaces the installed map with what actually exists on disk, keeping
    /// metadata for versions that are still present.
    pub fn reconcile(&mut self, paths: &Paths) {
        let scanned = scan_installed(paths);
        let mut merged = BTreeMap::new();
        for (id, mut found) in scanned {
            if let Some(old) = self.installed.get(&id) {
                if old.version == found.version {
                    found.tag = old.tag.clone();
                    found.sha256 = old.sha256.clone();
                    found.installed_at = old.installed_at;
                }
            }
            merged.insert(id, found);
        }
        self.installed = merged;
        // Drop cached releases whose app disappeared from the catalogue.
        self.releases
            .retain(|id, _| SUITE.iter().any(|a| a.id == id));
    }

    pub fn cached_plan(&self, id: &str, ttl_secs: i64, now: i64) -> Option<&ReleasePlan> {
        let entry = self.releases.get(id)?;
        if now.saturating_sub(entry.fetched_at) <= ttl_secs {
            Some(&entry.plan)
        } else {
            None
        }
    }

    pub fn store_plan(&mut self, plan: &ReleasePlan, now: i64) {
        self.releases.insert(
            plan.app_id.clone(),
            CachedRelease {
                fetched_at: now,
                plan: plan.clone(),
            },
        );
    }

    pub fn installed_version(&self, id: &str) -> Option<&str> {
        self.installed.get(id).map(|a| a.version.as_str())
    }
}

/// Reads `~/.local/opt/artcraft/<id>/<version>` and picks the newest version
/// that actually contains a `bin/` directory.
pub fn scan_installed(paths: &Paths) -> BTreeMap<String, InstalledApp> {
    let mut out = BTreeMap::new();
    for app in SUITE.iter() {
        let root = paths.app_dir(app.id);
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        let mut best: Option<(ReleaseVersion, PathBuf)> = None;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) if !name.starts_with('.') => name,
                _ => continue,
            };
            if !path.join("bin").is_dir() {
                continue;
            }
            let version = ReleaseVersion::parse(name);
            let newer = best
                .as_ref()
                .is_none_or(|(current, _)| version.cmp_version(current).is_gt());
            if newer {
                best = Some((version, path));
            }
        }
        if let Some((version, dir)) = best {
            let bundle = app.bundle_id();
            out.insert(
                app.id.to_string(),
                InstalledApp {
                    version: version.raw.clone(),
                    tag: version.raw,
                    sha256: String::new(),
                    installed_at: 0,
                    desktop_file: paths.desktop_file(&bundle),
                    dir,
                },
            );
        }
    }
    out
}

/// The version directory used while installing: `<root>/<id>/.staging-<pid>`.
pub fn staging_dir(paths: &Paths, id: &str) -> PathBuf {
    paths
        .app_dir(id)
        .join(format!(".staging-{}", std::process::id()))
}

/// Directory name for a version, safe against traversal.
pub fn version_dir_name(version: &str) -> String {
    sanitize_component(version)
}

pub fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("json")
    ));
    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(mode)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Builds the `InstalledApp` record for a freshly installed version.
pub fn installed_record(paths: &Paths, plan: &ReleasePlan, dir: &Path) -> InstalledApp {
    InstalledApp {
        version: plan.version.clone(),
        tag: plan.tag.clone(),
        sha256: plan.sha256.clone(),
        installed_at: now_unix(),
        dir: dir.to_path_buf(),
        desktop_file: paths.desktop_file(&format!("ai.storyteller.{}", plan.app_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Paths::from_env();
        let home = dir.path();
        paths.config_dir = home.join("config");
        paths.state_dir = home.join("state");
        paths.cache_dir = home.join("cache");
        paths.data_dir = home.join("data");
        paths.install_root = home.join("opt");
        paths.bin_dir = home.join("bin");
        paths.applications_dir = home.join("data/applications");
        paths.desktop_dir = paths.applications_dir.clone();
        paths.icons_dir = home.join("data/icons");
        paths.mime_dir = home.join("data/mime");
        paths.metainfo_dir = home.join("data/metainfo");
        paths.ensure().unwrap();
        (dir, paths)
    }

    #[test]
    fn scans_the_newest_version_on_disk() {
        let (_dir, paths) = temp_paths();
        for version in ["0.9.0", "0.13.0", "0.10.1"] {
            let dir = paths.app_version_dir("gridcraft", version);
            fs::create_dir_all(dir.join("bin")).unwrap();
            fs::write(dir.join("bin/gridcraft"), b"bin").unwrap();
        }
        fs::create_dir_all(paths.app_dir("gridcraft").join(".staging-1/bin")).unwrap();
        let found = scan_installed(&paths);
        assert_eq!(found["gridcraft"].version, "0.13.0");
    }

    #[test]
    fn cached_plans_expire() {
        let mut state = State::default();
        let plan = ReleasePlan {
            app_id: "gridcraft".into(),
            tag: "v1.0.0".into(),
            version: "1.0.0".into(),
            asset_name: "x.tar.gz".into(),
            asset_url: "https://github.com/storytold/gridcraft/releases/download/v1.0.0/x.tar.gz".into(),
            asset_size: 1,
            sha256: "a".repeat(64),
            sums_sha256: None,
            api_sha256: None,
            release_url: "https://github.com/storytold/gridcraft/releases".into(),
            published_at: None,
            notes: String::new(),
            prerelease: false,
        };
        state.store_plan(&plan, 100);
        assert!(state.cached_plan("gridcraft", 60, 150).is_some());
        assert!(state.cached_plan("gridcraft", 60, 200).is_none());
    }
}
