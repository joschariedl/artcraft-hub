# ArtCraft Hub

Ein nativer Installer und Updater für die 13 Apps von [ArtCraft](https://getartcraft.com/apps) –
gebaut für Arch Linux und [Omarchy](https://omarchy.org) unter Wayland.

ArtCraft Hub lädt die offiziellen `linux-x86_64.tar.gz`-Pakete direkt aus den GitHub-Releases,
prüft sie gegen die `SHA256SUMS.txt` des Releases **und** gegen VirusTotal, installiert sie ohne
Root-Rechte nach `~/.local` und legt einen Menüeintrag mit hochauflösendem Icon an.

Dazu gibt es:

- **`artcraft-hub`** – eine native GUI (Rust + egui/eframe, kein Electron, kein Webview)
- **`artcraftctl`** – eine CLI für Status, Installation, Updates und Diagnose
- **`derfleischmann.artcraft`** – ein Omarchy-Bar-Plugin mit Update-Badge und Popup

Alle Texte sind auf Deutsch.

---

## Inhalt

- [Die Apps](#die-apps)
- [Sicherheitsmodell](#sicherheitsmodell)
- [Voraussetzungen](#voraussetzungen)
- [Installation](#installation)
- [VirusTotal-Key einrichten](#virustotal-key-einrichten)
- [Benutzung](#benutzung)
  - [GUI](#gui)
  - [CLI](#cli)
  - [Omarchy-Bar-Plugin](#omarchy-bar-plugin)
- [Datenpfade](#datenpfade)
- [Konfiguration](#konfiguration)
- [Funktionsweise im Detail](#funktionsweise-im-detail)
- [Troubleshooting](#troubleshooting)
- [Entwicklung](#entwicklung)
- [Lizenz](#lizenz)

---

## Die Apps

| App | Bereich | Repository |
| --- | --- | --- |
| PhotoCraft | Bildbearbeitung | `storytold/photocraft` |
| VectorCraft | Vektorillustration | `storytold/vectorcraft` |
| FilmCraft | Videoschnitt, Farbe und Ton | `storytold/filmcraft` |
| LightCraft | Fotobibliothek und RAW-Entwicklung | `storytold/lightcraft` |
| PdfCraft | PDF lesen, ordnen und schützen | `storytold/pdfcraft` |
| EffectCraft | Motion Graphics und VFX | `storytold/effectcraft` |
| DesignCraft | Seitenlayout und Publishing | `storytold/designcraft` |
| FigCraft | Interface-Design und Prototyping | `storytold/figcraft` |
| WordCraft | Textverarbeitung und Dokumentdesign | `storytold/wordcraft` |
| SoundCraft | Audio aufnehmen, schneiden und mischen | `storytold/soundcraft` |
| DeckCraft | Präsentationen und Slideshows | `storytold/deckcraft` |
| CADCraft | CAD und technisches Zeichnen | `storytold/cadcraft` |
| GridCraft | Tabellen und Datenanalyse | `storytold/gridcraft` |

Alle 13 Apps haben öffentliche Releases. FigCraft erscheint nur dann als „kein Release“,
wenn GitHub das API-Limit (60 Anfragen/Stunde ohne Token) erreicht hat – dann fehlt der
Feed-Fallback für diese App. Ein GitHub-Token (Einstellungen) behebt das.

---

## Sicherheitsmodell

Jede Installation und jedes Update durchläuft drei Stufen. Schlägt eine davon fehl, wird **nicht**
installiert.

1. **Herkunft** – Download ausschließlich über `https://github.com/…/releases/download/…`.
   Asset-Namen müssen dem Muster `<id>-<version>-linux-<arch>.tar.gz` entsprechen.
2. **Integrität (SHA-256)** – Der Hash der heruntergeladenen Datei wird gegen die
   `SHA256SUMS.txt` des Releases geprüft. Die `SHA256SUMS.txt` selbst wird zusätzlich gegen den
   von der GitHub-API gemeldeten Asset-Digest (`sha256:…`) verifiziert. Widersprechen sich die
   beiden Quellen, bricht die Installation mit einem harten Fehler ab.
3. **Schadsoftware (VirusTotal)** – Die Download-URL wird bei VirusTotal zur Prüfung eingereicht
   (URL-Scan). Ist der Inhalt dort bereits als Datei bekannt, wird dieser Eintrag verwendet.
   Installiert wird nur, wenn VirusTotal den **Inhalts-Hash** der URL-Antwort mit dem erwarteten
   SHA-256 bestätigt und **keine** Engine die Datei als `malicious` oder `suspicious` meldet.

### Warum URL-Scan statt Datei-Upload?

Die Release-Pakete sind 40–130 MB groß, die öffentliche VirusTotal-API erlaubt aber nur Uploads bis
32 MB. Der Hub lädt daher nichts zu VirusTotal hoch, sondern übergibt die GitHub-Download-URL.
VirusTotal lädt die Datei selbst und berechnet den SHA-256 des Inhalts. Diesen Hash vergleicht der
Hub mit dem erwarteten Hash – damit ist ausgeschlossen, dass ein beliebiger Redirect oder eine
abweichende Datei „durchgewinkt“ wird.

### Fail-closed

- Ohne VirusTotal-Key sind Installation und Updates **gesperrt**. Der Hub erklärt das in der GUI und
  im Popup des Bar-Plugins.
- Unbekannte oder noch nicht ausgewertete Ergebnisse gelten als Ablehnung. Das Verhalten lässt sich
  nur bewusst über `vt.allow_unknown = true` (bzw. `artcraftctl config --allow-unknown true`)
  ändern – davon ist abzuraten.
- Die VirusTotal-Ergebnisse werden pro SHA-256 für 7 Tage gecacht. Das schont das Kontingent
  (Public API: 4 Anfragen/Minute, 500/Tag) und macht wiederholte Prüfungen schnell.

### Was der Hub *nicht* tut

- Keine Root-Rechte, keine Schreibzugriffe außerhalb des Home-Verzeichnisses.
- Kein Auto-Update im Hintergrund: Installationen starten immer auf ausdrückliche Aktion.
- Keine Telemetrie. Netzwerkzugriffe gehen nur an GitHub, `api.storyteller.ai` (Metadaten-Fallback)
  und – bei hinterlegtem Key – an `www.virustotal.com`.

---

## Voraussetzungen

- Arch Linux oder Omarchy (getestet mit Omarchy 4.0.6, Hyprland 0.56, Wayland)
- Rust 1.85 oder neuer (`rustup` empfohlen) – nur zum Bauen
- `git`, `python3` (für das Installationsskript), optional `desktop-file-utils` und
  `gtk-update-icon-cache`
- Ein kostenloser [VirusTotal-API-Key](https://www.virustotal.com/gui/my-apikey)

Für Wayland bringt die GUI alles Nötige mit; es wird kein X11 und kein Webview benötigt.

---

## Installation

```bash
git clone https://github.com/derfleischmann/artcraft-hub.git
cd artcraft-hub
./scripts/install.sh
```

Das Skript baut den Workspace im Release-Modus und installiert:

| Datei | Ziel |
| --- | --- |
| `artcraft-hub` | `~/.local/bin/artcraft-hub` |
| `artcraftctl` | `~/.local/bin/artcraftctl` |
| Hub-Icon | `~/.local/share/icons/hicolor/scalable/apps/artcraft-hub.svg` |
| Menüeintrag | `~/.local/share/applications/artcraft-hub.desktop` |
| Bar-Plugin | `~/.config/omarchy/plugins/derfleischmann.artcraft/` |

Anschließend wird das Plugin validiert und – falls `omarchy` verfügbar ist – in der rechten
Bar-Sektion aktiviert. Zum Schluss läuft `artcraftctl doctor`.

Optionen:

```bash
./scripts/install.sh --debug        # Debug-Build
./scripts/install.sh --no-plugin    # ohne Omarchy-Plugin
./scripts/install.sh --no-enable    # Plugin kopieren, aber nicht in die Bar aufnehmen
```

Deinstallieren:

```bash
./scripts/uninstall.sh            # nur Hub, CLI, Icon, Desktop-Datei und Plugin
./scripts/uninstall.sh --apps     # zusätzlich alle installierten ArtCraft-Apps
./scripts/uninstall.sh --purge    # zusätzlich Konfiguration, Status und Caches
```

---

## VirusTotal-Key einrichten

```bash
artcraftctl config --set-vt-key <DEIN_KEY>
```

Der Key landet in `~/.config/artcraft-hub/config.toml` mit Dateirechten `0600`. Alternativ kann er
über die Umgebungsvariable `VIRUSTOTAL_API_KEY` gesetzt werden; die GUI bietet dafür ein Feld unter
**Einstellungen**.

Prüfen:

```bash
artcraftctl doctor
```

Der Doctor zeigt an, ob der Key gefunden wird, ob `~/.local/bin` im `PATH` liegt und ob die
Hilfswerkzeuge vorhanden sind.

---

## Benutzung

### GUI

```bash
artcraft-hub
```

Oder über das Anwendungsmenü („ArtCraft Hub“). Die Oberfläche zeigt alle 13 Apps mit Icon,
installierter Version und der neuesten verfügbaren Version.

- **Installieren** – lädt, prüft (SHA-256 + VirusTotal) und installiert die aktuelle Version.
- **Aktualisieren** – erscheint, sobald eine neuere Version verfügbar ist; einzeln oder alle
  zusammen.
- **Deinstallieren** – entfernt Programm, Symlinks, Desktop-Datei und Icons der App.
- **Neu prüfen** – erzwingt eine Netzwerkabfrage statt des 30-Minuten-Caches.
- **Einstellungen** – VirusTotal-Key, GitHub-Token, Prüfintervall, Icon-Optionen und Sprache.

Während eines Downloads zeigt die GUI Fortschritt und die jeweils laufende Prüfstufe an. Fehler
werden im Klartext erklärt (fehlender Key, Hash-Konflikt, VirusTotal-Fund, kein Release …).

### CLI

```bash
artcraftctl list                 # Tabelle aller Apps und Versionen
artcraftctl status               # kompakter Status (Cache, ~0 s)
artcraftctl status --refresh     # Status mit Netzwerkabfrage
artcraftctl check                # gezielt nach Updates suchen
artcraftctl install gridcraft    # eine App installieren
artcraftctl upgrade --all        # alle Apps aktualisieren
artcraftctl upgrade photocraft   # eine App aktualisieren
artcraftctl uninstall deckcraft  # eine App entfernen
artcraftctl doctor               # Umgebung und Schlüssel prüfen
artcraftctl config --show        # aktuelle Konfiguration anzeigen
```

Nützliche Flags: `--json` für maschinenlesbare Ausgabe (`list`, `status`, `check`, `doctor`),
`-y/--yes` zum Überspringen der Rückfrage, `--force` zum erneuten Installieren der neuesten
Version, `--refresh` zum Ignorieren des Metadaten-Caches.

Beispiel:

```console
$ artcraftctl status
App          Installiert    Neueste        Status
photocraft   0.16.0         0.17.0         Update verfügbar
gridcraft    0.13.0         0.13.0         aktuell
vectorcraft  –              0.18.0         nicht installiert
```

### Omarchy-Bar-Plugin

Das Plugin sitzt in der rechten Bar-Sektion (Pinsel-Symbol). Es zeigt einen Punkt und eine
Hervorhebung, sobald Updates verfügbar sind; der Tooltip nennt die Anzahl.

| Aktion | Wirkung |
| --- | --- |
| Linksklick | Popup öffnen/schließen |
| Mittelklick | Popup direkt öffnen |
| `r` im Popup | Neu prüfen |
| `o` im Popup | ArtCraft Hub starten |
| `Enter` im Popup | Alle Updates installieren |

Das Popup listet installierte Apps mit Update, nicht installierte Apps und – falls nötig – eine
deutliche Warnung, wenn der VirusTotal-Key fehlt. Der Upgrade-Lauf startet in einem schwebenden
Terminal, damit Fortschritt und Prüfergebnisse sichtbar bleiben.

Manuell steuern:

```bash
omarchy plugin list                       # Status
omarchy plugin enable derfleischmann.artcraft right
omarchy plugin disable derfleischmann.artcraft
omarchy-shell derfleischmann.artcraft open     # Popup per IPC öffnen
omarchy-shell derfleischmann.artcraft close
```

Das Plugin liegt zusätzlich im Repository unter `plugins/derfleischmann.artcraft/`.

---

## Datenpfade

| Inhalt | Pfad |
| --- | --- |
| Konfiguration (0600) | `~/.config/artcraft-hub/config.toml` |
| Status / installierte Versionen | `~/.local/state/artcraft-hub/state.json` |
| VirusTotal-Cache | `~/.local/state/artcraft-hub/virustotal-cache.json` |
| Downloads, Icons | `~/.cache/artcraft-hub/` |
| Installierte Apps | `~/.local/opt/artcraft/<id>/<version>/` |
| Startbefehle | `~/.local/bin/<id>` (Symlink auf die aktive Version) |
| Menüeinträge der Apps | `~/.local/share/applications/ai.storyteller.<id>.desktop` |
| Icons der Apps | `~/.local/share/icons/hicolor/…/apps/ai.storyteller.<id>.png` |

Mehrere Versionen einer App können parallel unter `~/.local/opt/artcraft/<id>/` liegen; der
Symlink in `~/.local/bin` zeigt auf die aktive. Eine Deinstallation entfernt alle Versionen der App.

---

## Konfiguration

`~/.config/artcraft-hub/config.toml` (wird beim ersten Speichern mit `0600` angelegt):

| Schlüssel | Standard | Bedeutung |
| --- | --- | --- |
| `github_token` | `""` | Optional; hebt das GitHub-Limit von 60 auf 5000 Anfragen/Stunde |
| `virustotal_api_key` | `""` | Pflicht für Installationen (oder `VIRUSTOTAL_API_KEY`) |
| `metadata_cache_minutes` | `30` | Gültigkeit der Release-Metadaten |
| `check_interval_minutes` | `60` | Empfohlenes Prüfintervall für Hub und Plugin |
| `require_sha256sums` | `true` | Releases ohne `SHA256SUMS.txt` ablehnen |
| `install_1024_icons` | `true` | 1024-px-PNG aus dem App-Repository zusätzlich installieren |
| `language` | `de` | UI-Sprache |
| `vt.min_interval_seconds` | `16` | Mindestabstand zwischen VirusTotal-Anfragen |
| `vt.cache_days` | `7` | Gültigkeit eines VirusTotal-Ergebnisses |
| `vt.poll_seconds` | `20` | Wartezeit zwischen Statusabfragen eines URL-Scans |
| `vt.max_polls` | `15` | Maximale Anzahl Statusabfragen |
| `vt.allow_unknown` | `false` | Unklare Ergebnisse zulassen (**nicht empfohlen**) |

Ändern lassen sich Key, GitHub-Token und `allow_unknown` über `artcraftctl config`; alles andere
per Editor oder in der GUI.

---

## Funktionsweise im Detail

**Release-Erkennung.** Primär über die GitHub-API (`/releases/latest`). Als Fallback – etwa bei
aufgebrauchtem API-Kontingent – fragt der Hub `https://api.storyteller.ai/v1/craft_apps/release_info`
einmal für alle Apps ab. Download und `SHA256SUMS.txt` kommen immer direkt von GitHub.

**Installation.** Das `tar.gz` wird im Cache entpackt, jeder Pfad wird auf Traversal geprüft
(keine absoluten Pfade, kein `..`), und der Inhalt wird nach `~/.local/opt/artcraft/<id>/<version>/`
verschoben. Danach werden Symlinks in `~/.local/bin`, die Desktop-Datei (mit korrektem
`StartupWMClass` und Icon-Namen) und die Icons in `hicolor` angelegt. Abschließend laufen
`update-desktop-database` und `gtk-update-icon-cache`, damit die Apps sofort im Omarchy-Menü
erscheinen.

**Icons.** Bevorzugt wird `assets/app-icon/<id>-1024.png` aus dem jeweiligen Repository
(1024×1024, RGBA). Fehlt es, greift der Hub auf die Icons im Tarball zurück. Die GUI cached die
PNGs unter `~/.cache/artcraft-hub/icons/`.

**Status.** `artcraftctl status` liest den lokalen Status und die gecachten Metadaten, daher ist
der Aufruf für das Bar-Plugin praktisch kostenlos. `--refresh` bzw. `check` erzwingen eine
Netzwerkabfrage.

---

## Troubleshooting

**„VirusTotal-Key fehlt“ / Installationen gesperrt.**
Key hinterlegen: `artcraftctl config --set-vt-key <KEY>`, danach `artcraftctl doctor`.

**„VirusTotal-Kontingent erschöpft“ oder HTTP 429.**
Die Public API erlaubt 4 Anfragen/Minute und 500/Tag. Der Hub drosselt selbst auf eine Anfrage alle
16 Sekunden und cacht Ergebnisse 7 Tage. Bei häufigem Testen hilft Warten oder ein eigener Key.

**„Hash stimmt nicht überein“ / Hash-Konflikt.**
Die Datei weicht von `SHA256SUMS.txt` oder vom GitHub-Asset-Digest ab. Das kann ein abgebrochener
Download sein – erneut versuchen. Bleibt es dabei, nicht installieren und das Release im
App-Repository melden.

**„Kein Release“ für FigCraft (oder andere Apps).**
Meist ein erreichtes GitHub-API-Limit (60 Anfragen/Stunde ohne Token). Der Release-Feed
enthält FigCraft nicht, daher gibt es keinen Fallback. Lösung: `artcraftctl config --set-github-token <TOKEN>`
oder warten. Mit Token (5.000 Anfragen/Stunde) tritt das nicht mehr auf.

**GitHub-API-Limit (HTTP 403).**
Ohne Token erlaubt GitHub 60 Anfragen/Stunde. Der Feed-Fallback greift automatisch; optional
`artcraftctl config --set-github-token <TOKEN>` setzen.

**App erscheint nicht im Menü.**
`update-desktop-database ~/.local/share/applications` und
`gtk-update-icon-cache -f ~/.local/share/icons/hicolor` ausführen. Prüfen, ob `~/.local/bin` im
`PATH` liegt und die Desktop-Datei unter `~/.local/share/applications/` existiert.

**Plugin zeigt „artcraftctl lieferte keine gültige Antwort“.**
`~/.local/bin/artcraftctl` fehlt oder ist nicht ausführbar. `./scripts/install.sh` erneut ausführen
und mit `artcraftctl doctor` prüfen.

**Plugin nicht in der Bar.**
`omarchy plugin list` prüfen; aktivieren mit
`omarchy plugin enable derfleischmann.artcraft right`. Die Position lässt sich anschließend in
`~/.config/omarchy/shell.json` unter `bar.layout.right` verschieben.

**GUI startet nicht unter Wayland.**
`WAYLAND_DISPLAY` prüfen (`echo $WAYLAND_DISPLAY`). Die GUI nutzt eframe/glow; auf Systemen ohne
passende GL-Treiber hilft `LIBGL_ALWAYS_SOFTWARE=1 artcraft-hub`.

---

## Entwicklung

```bash
cargo build --workspace              # Debug
cargo build --workspace --release    # Release
cargo test --workspace               # Tests (30+ in artcraft-core)
cargo clippy --workspace --all-targets
```

Aufbau:

```
crates/artcraft-core/   Katalog, GitHub, VirusTotal, Download, Installation, Status
crates/artcraft-hub/    GUI (eframe/egui)
crates/artcraftctl/     CLI
plugins/                Omarchy-Bar-Plugin (QML)
assets/                 Hub-Icon
scripts/                install.sh, uninstall.sh
```

Die Kernlogik ist frei von UI-Abhängigkeiten und wird über `ops::{check_all, install,
upgrade_all, uninstall, doctor}` von GUI und CLI gemeinsam genutzt.

---

## Lizenz

MIT – siehe Repository-Metadaten. ArtCraft selbst ist ein Produkt von Storytold; dieses Projekt
steht in keiner offiziellen Verbindung dazu und installiert ausschließlich die offiziellen
Release-Pakete.
