# AGENTS.md – ArtCraft Hub

This file documents the project for AI agents (and humans) working on the codebase. It describes
the architecture, data flow, key decisions, file layout and conventions so changes stay consistent.

## Project overview

ArtCraft Hub is a native installer/updater for the 13 ArtCraft apps
(https://getartcraft.com/apps). It targets Arch Linux / Omarchy under Wayland. It downloads the
official `linux-x86_64.tar.gz` releases from GitHub, verifies them (SHA-256 + VirusTotal + optional
GPG), installs them without root into `~/.local`, creates `.desktop` entries with high-resolution
icons, and provides an Omarchy bar widget for update notifications and upgrades.

- Language: all user-facing text is **English**.
- License: MIT.
- Author: Joscha Riedl (online handle: derfleischmann).
- Repository: https://github.com/joschariedl/artcraft-hub (private).

## Workspace layout

```
artcraft-hub/
├── Cargo.toml                  # workspace root
├── crates/
│   ├── artcraft-core/          # shared library (no UI deps)
│   ├── artcraft-hub/           # GUI (eframe/egui)
│   └── artcraftctl/            # CLI
├── plugins/
│   └── derfleischmann.artcraft/ # Omarchy bar plugin (QML)
├── assets/
│   ├── artcraft-hub.svg        # hub icon
│   └── icons/                  # embedded app icons (PNG, 256x256, padding-free)
├── scripts/
│   ├── install.sh
│   └── uninstall.sh
├── README.md
├── AGENTS.md
└── LICENSE
```

## Crates

### artcraft-core (`crates/artcraft-core`)

Shared library. No UI dependencies. Used by both GUI and CLI.

Key modules:
- `catalog.rs` – `AppDef`, `SUITE: [AppDef; 13]`, `by_id()`, `release_arch()`, `icon_paths()`.
  - 13 apps in website order. pdfcraft has stem `printcraft`.
  - Bugfix: `icon_paths()` iterates `once(id).chain(stems)` so every app has its 1024 px icon path.
- `config.rs` – `Config` (TOML, 0600). Fields: github_token, virustotal_api_key, metadata_cache_minutes,
  check_interval_minutes, require_sha256sums, install_1024_icons, vt (VtConfig), language (default "en").
  - `vt_key()`, `github_token()` resolve env fallbacks. `metadata_ttl_secs()`.
- `paths.rs` – `Paths::from_env()`. install_root `~/.local/opt/artcraft`, bin `~/.local/bin`,
  applications `~/.local/share/applications`, icons `~/.local/share/icons`, state `~/.local/state/artcraft-hub`,
  cache `~/.cache/artcraft-hub`. `omarchy_plugin_dir()`.
- `github.rs` – release discovery. `latest()` (GitHub API → feed fallback), `resolve()`, `plan_from_release()`,
  `plan_from_parts()` (checksum rules), `select_asset()`, `fetch_sums()`, `parse_sha256sums()`,
  `icon_urls()`, `valid_*` validators. Feed URL: `https://api.storyteller.ai/v1/craft_apps/release_info`.
- `vt.rs` – `VtClient`, `VtResult`, `Verdict`, `Stats`, `VtCache`. URL-scan flow: file_report → submit_url →
  poll → url_report → hash comparison. `invalidate()`, `new()`, `check()`. Rate limiting (16s), 7-day cache.
  - `VtResult::permalink()` links to `/gui/file/<sha256>` for file reports and `/gui/url/<base64url>` for
    URL-scan verdicts. URL-scan results store the scanned `url`; older cache entries are backfilled on the
    next check and otherwise fall back to the release plan's asset URL in the GUI.
- `download.rs` – `fetch()` (streaming download with SHA-256 check), `fetch_bytes()`, `has_checksum()`.
- `install.rs` – `extract_archive()`, `checked_relative()`, `install_archive()`, `find_main_binary()`,
  `write_desktop_file()`, `build_desktop_file()`, `quote_exec()`, `copy_icons()`, `copy_extras()`,
  `install_highres_icon()`, `prune_old_versions()`, `uninstall()`, `verify_gpg_signature()`.
  - TryExec is written **unquoted** (GLib treats quoted paths as unavailable).
  - `copy_icons` copies only PNGs (SVGs are size-gated: <= 50 KB).
- `ops.rs` – high-level ops shared by GUI and CLI: `check_all()`, `install()`, `install_with_plan()`,
  `upgrade_all()`, `uninstall()`, `doctor()`, `vt_recheck()`, `open_app()`, `prune_download_cache()`,
  `version_label()`, `installed_dir()`.
- `state.rs` – `State` (installed map, cached releases, checked_at). `reconcile()` scans disk.
  `scan_installed()` picks newest version dir with a `bin/` directory.
- `version.rs` – `ReleaseVersion` parsing/comparison.
- `progress.rs` – `Progress` enum (Message, Download).
- `net.rs` – `ureq` wrapper (agent, get, post_form). `http_status_as_error(false)`.
- `error.rs` – `Error` enum (thiserror). All messages in English.
- `time.rs` – `now_unix()`, `parse_rfc3339()`.

### artcraft-hub (`crates/artcraft-hub`)

Native GUI. eframe 0.37 + egui 0.37, glow backend, Wayland + X11 + clipboard.

- `main.rs` – `HubApp` (eframe::App). Views: Apps, Settings. Panels: top, bottom, left sidebar, central.
  - `spawn_check`, `spawn_install`, `spawn_install_all`, `spawn_upgrade`, `spawn_vt_recheck`.
  - `icons` loaded from embedded PNGs (include_bytes!) via `embedded_icon()`.
  - `icon()` renders via `painter().image()` into a fixed-size rect.
  - `to_color_image()` thumbnails to 256×256.
  - `local_icon()` prefers icon cache, then hicolor sizes.
  - Footer shows version number (`v{CARGO_PKG_VERSION}`).
  - `--version`/`--help` handled in main().
- Dependencies: eframe (default_fonts, glow, wayland, x11, clipboard), egui, image (png, webp), serde, serde_json.

### artcraftctl (`crates/artcraftctl`)

CLI. clap derive. Commands: list, status, check, install, upgrade, uninstall, doctor, config.
- `status --json` outputs schema v1 JSON used by the Omarchy plugin.
- `reporter()` prints progress to stderr.

## Omarchy plugin (`plugins/derfleischmann.artcraft`)

QML bar widget. Files: `manifest.json`, `BarWidget.qml`, `ArtcraftPanel.qml`, `README.md`.

- `BarWidget.qml`: BarIconButton with paintbrush glyph `\uf1fc`. Properties: opened, updateCount, vtMissing.
  IPC handler: refresh, open, close, show, hide, toggle.
- `ArtcraftPanel.qml`: KeyboardPanel. Reads `artcraftctl status --json` via Process + StdioCollector.
  - `refresh(force)` → `status` (cached) or `check` (network).
  - `upgradeAll()` / `upgradeOne(id)` → runs `artcraftctl upgrade ... --yes` in a background Process,
    captures stdout/stderr into `upgradeLog`, shows spinner + scrollable log while upgrading.
  - `openApp(id)` → launches `~/.local/bin/<id>`.
  - `openHub()` → launches `artcraft-hub`.
  - Keyboard: `r` refresh, `o` open hub, `Enter` update all, click app name → open app.
  - Timer: 30 min polling (triggeredOnStart).
  - Footer: shortcuts hint.
- Installed to `~/.config/omarchy/plugins/derfleischmann.artcraft/`.
- Enabled in `~/.config/omarchy/shell.json` under `bar.layout.right`.

## Data flow

1. **Status check**: `artcraftctl status --json` → `ops::check_all()` → for each app: load cached plan
   (TTL 30 min) or `github::resolve()`. Reconciles installed state from disk. Outputs JSON schema v1.
2. **Plugin polling**: Timer (30 min) → `refresh(false)` → `status --json`. If cache expired, network query.
3. **Install**: `ops::install()` → VT check (VtClient) → download (SHA-256 verified) → GPG check →
   `install::install_archive()` → extract, desktop file, symlinks, icons, prune old versions.
4. **Upgrade**: `ops::upgrade_all()` → `check_all(force=true)` → for each update_available, `install_with_plan()`.
5. **State**: `~/.local/state/artcraft-hub/state.json` (installed map + cached releases). Filesystem is
   source of truth; state adds metadata. `reconcile()` syncs.

## Security model

- Download only from `https://github.com/<repo>/releases/download/<tag>/<name>`.
- SHA-256 checked against `SHA256SUMS.txt`; `SHA256SUMS.txt` checked against GitHub asset digest.
- VirusTotal: file report → URL scan → content hash comparison. Fail-closed. No file upload.
- Optional GPG: if `.sig`/`.asc` asset exists, verify with gpg; if gpg missing, skip with note.
- No root; installs only in `~/.local`.

## Key decisions

- **egui 0.37 API**: `App::ui(&mut self, ui, frame)` not `update`. Panels use `egui::Panel::{top,bottom,left}`
  with `.show(ui, …)` and `CentralPanel::show(ui, …)`. Styles via `Context::all_styles_mut`.
- **ureq 3.5** (blocking) instead of Tokio. HTTP in background threads.
- **No Electron/webview**: native Rust.
- **Icons**: embedded WebP→PNG (256×256, padding-free) via `include_bytes!`. OS menu uses 1024 px PNGs.
  SVGs installed only if < 50 KB (clean vectors: gridcraft, wordcraft).
- **TryExec unquoted** in desktop files (GLib requires a path, not a quoted command).
- **Config 0600**, state 0644.
- **Cache pruning**: download cache keeps only the latest version per app.

## Conventions

- All user-facing text in English.
- Error messages via `thiserror` in `error.rs`, all English.
- Progress messages go through `progress::message()` / `Progress::Message`.
- CLI prints progress to stderr.
- Rust edition 2021, rust-version 1.85. Clippy clean (0 warnings).
- Tests: 30+ in artcraft-core. Run `cargo test --workspace`.

## Build & install

```bash
cargo build --workspace --release
./scripts/install.sh              # builds + installs hub, ctl, plugin
./scripts/install.sh --debug       # debug build
./scripts/install.sh --no-plugin  # without Omarchy plugin
```

Uninstall: `./scripts/uninstall.sh [--apps] [--purge]`.

## Common pitfalls

- `gh` must be authenticated for pushing (`gh auth login`).
- Omarchy plugin requires `omarchy-shell` in PATH.
- FigCraft (and others) show "no release" only on GitHub rate limit (60 req/h without token).
- The icon cache (`~/.cache/artcraft-hub/icons/`) is populated from embedded data; no network needed.
- `update-desktop-database` and `gtk-update-icon-cache` should be installed for menu integration.
- Wayland: GUI uses eframe/glow; set `LIBGL_ALWAYS_SOFTWARE=1` if GL drivers missing.

## Recent changes (summary)

- Initial workspace: artcraft-core, artcraft-hub (GUI), artcraftctl (CLI).
- Bugfix: `icon_paths()` now includes all apps' icons (was only pdfcraft).
- GUI adapted to egui 0.37 API.
- Omarchy plugin created and installed.
- README.md (English), LICENSE (MIT), AGENTS.md.
- Security: SHA-256 + VirusTotal URL scan + optional GPG.
- Cache pruning, VT re-check button, install-all button, open-app button.
- Icons: embedded WebP→PNG, consistent rendering, size-gated SVGs.
- Footer: version number, vertically centered.
- All text translated to English.
- Git repo initialized and pushed to https://github.com/joschariedl/artcraft-hub (private).
- Cross-process `flock` lock (`lock.rs`), transactional installs with rollback, version pinning/skip.
- Network robustness: retries/backoff, conditional requests, resumable downloads, parallel checks.
- Stronger trust anchors: signed manifest, GitHub attestations, configurable GPG keyring.
- Omarchy theme integration, GUI filters and keyboard shortcuts, plugin schema v2.
