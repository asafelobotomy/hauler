#!/usr/bin/env bash
# Build hauler and install it for the current user: binaries in ~/.local/bin,
# systemd user units in ~/.config/systemd/user, and the desktop launcher.
set -euo pipefail
cd "$(dirname "$0")/.."

BINDIR=${BINDIR:-$HOME/.local/bin}
UNITDIR=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
APPDIR=${XDG_DATA_HOME:-$HOME/.local/share}/applications

cargo build --release -p hauler-cli -p hauler-daemon -p hauler-desktop
install -Dm755 target/release/hauler "$BINDIR/hauler"
install -Dm755 target/release/haulerd "$BINDIR/haulerd"
install -Dm755 target/release/hauler-desktop "$BINDIR/hauler-desktop"
mkdir -p "$APPDIR"
sed "s|^Exec=hauler-desktop|Exec=$BINDIR/hauler-desktop|" dist/hauler.desktop > "$APPDIR/hauler.desktop"
install -Dm644 dist/systemd/hauler.socket "$UNITDIR/hauler.socket"
sed "s|@BINDIR@|$BINDIR|" dist/systemd/hauler.service > "$UNITDIR/hauler.service"

systemctl --user daemon-reload
# Restart picks up a new binary if the daemon was already running.
systemctl --user enable --now hauler.socket
if systemctl --user is-active --quiet hauler.service; then
    systemctl --user restart hauler.service
fi
update-desktop-database "$APPDIR" 2>/dev/null || true
echo "installed to $BINDIR; open Hauler from the app menu, or try: hauler add <url>"
