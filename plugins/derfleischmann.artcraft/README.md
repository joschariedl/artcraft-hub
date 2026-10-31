# ArtCraft Hub – Omarchy-Bar-Plugin

Kleine Bar-Anzeige für die 13 ArtCraft-Apps. Ein Pinsel-Symbol oben rechts
wird farbig, sobald ein Update bereitsteht; ein Klick öffnet die Übersicht.

## Was es macht

- liest `artcraftctl status --json` (Standard: Cache, damit die Bar nicht bei
  jedem Timer-Tick das Netzwerk belastet)
- zeigt installierte Version → neue Version je App
- startet `artcraftctl upgrade …` in einem schwebenden Terminal. Dort laufen
  SHA-256-Abgleich und VirusTotal-Prüfung sichtbar mit; ohne gültigen
  VirusTotal-Key wird nichts installiert.
- warnt, wenn kein VirusTotal-Key hinterlegt ist, und bietet den Sprung in
  den Hub an

Die Abfrage läuft alle 30 Minuten sowie beim Öffnen des Popups. Der Knopf
„Neu prüfen“ (oder die Taste `r`) erzwingt eine frische GitHub-Abfrage.

## Abhängigkeiten

- `artcraftctl` in `~/.local/bin` (wird von `scripts/install.sh` installiert)
- `artcraft-hub` in `~/.local/bin` für den Knopf „Hub“

## Installation

```bash
./scripts/install.sh
```

Das Skript kopiert das Plugin nach
`~/.config/omarchy/plugins/derfleischmann.artcraft` und aktiviert es in der
rechten Bar-Sektion. Manuell:

```bash
omarchy plugin validate ~/.config/omarchy/plugins/derfleischmann.artcraft
omarchy-shell shell rescanPlugins
omarchy plugin enable derfleischmann.artcraft right
```

## Bedienung

| Aktion | Wirkung |
|---|---|
| Linksklick | Popup öffnen/schließen |
| Mittelklick | Popup öffnen |
| `r` im Popup | Neu prüfen |
| `o` im Popup | Hub öffnen |
| Enter im Popup | Alle Updates starten (bzw. neu prüfen) |
| Esc | Schließen |

IPC:

```bash
omarchy-shell derfleischmann.artcraft toggle
omarchy-shell derfleischmann.artcraft refresh
```
