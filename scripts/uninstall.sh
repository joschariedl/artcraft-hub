#!/usr/bin/env bash
# Removes ArtCraft Hub, artcraftctl and the Omarchy bar plugin.
#
# Installed Craft Apps stay where they are unless --apps is given.
set -euo pipefail

readonly PLUGIN_ID="derfleischmann.artcraft"
readonly CONFIG_DIR="${HOME}/.config/artcraft-hub"
readonly STATE_DIR="${HOME}/.local/state/artcraft-hub"
readonly CACHE_DIR="${HOME}/.cache/artcraft-hub"

purge=0
remove_apps=0

usage() {
  cat <<'EOF'
Nutzung: scripts/uninstall.sh [Optionen]

  --apps    Zusätzlich alle installierten ArtCraft-Apps entfernen
  --purge   Zusätzlich Konfiguration, Status und Cache löschen
            (enthält den gespeicherten VirusTotal-Key)
  -h, --help
EOF
}

for arg in "$@"; do
  case "$arg" in
    --apps) remove_apps=1 ;;
    --purge) purge=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unbekannte Option: $arg" >&2; usage >&2; exit 2 ;;
  esac
done

ctl="${HOME}/.local/bin/artcraftctl"

if (( remove_apps )); then
  if [[ -x "$ctl" ]]; then
    echo "==> Entferne installierte ArtCraft-Apps"
    for app in photocraft vectorcraft filmcraft lightcraft pdfcraft effectcraft \
               designcraft figcraft wordcraft soundcraft deckcraft cadcraft gridcraft; do
      "$ctl" uninstall "$app" --yes || true
    done
  else
    echo "artcraftctl fehlt – Apps werden nicht entfernt." >&2
  fi
fi

echo "==> Entferne Omarchy-Plugin"
if command -v omarchy >/dev/null 2>&1; then
  omarchy plugin remove "$PLUGIN_ID" --yes >/dev/null 2>&1 || true
fi
rm -rf "${HOME}/.config/omarchy/plugins/${PLUGIN_ID}"
omarchy-shell shell rescanPlugins >/dev/null 2>&1 || true

echo "==> Entferne Programme, Icon und Desktop-Datei"
rm -f "${HOME}/.local/bin/artcraft-hub" "${HOME}/.local/bin/artcraftctl"
rm -f "${HOME}/.local/share/applications/artcraft-hub.desktop"
rm -f "${HOME}/.local/share/icons/hicolor/scalable/apps/artcraft-hub.svg"
update-desktop-database "${HOME}/.local/share/applications" >/dev/null 2>&1 || true
gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true

if (( purge )); then
  echo "==> Lösche Konfiguration, Status und Cache"
  rm -rf "$CONFIG_DIR" "$STATE_DIR" "$CACHE_DIR"
fi

printf '\nFertig.\n'
(( remove_apps )) || echo "Hinweis: installierte Apps unter ~/.local/opt/artcraft bleiben erhalten."
