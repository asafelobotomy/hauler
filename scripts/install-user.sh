#!/usr/bin/env bash
# Build hauler and install it for the current user:
# binaries in ~/.local/bin, systemd user units in ~/.config/systemd/user.
set -euo pipefail
cd "$(dirname "$0")/.."

BINDIR=${BINDIR:-$HOME/.local/bin}
UNITDIR=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user

cargo build --release -p hauler-cli -p hauler-daemon
install -Dm755 target/release/hauler "$BINDIR/hauler"
install -Dm755 target/release/haulerd "$BINDIR/haulerd"
install -Dm644 dist/systemd/hauler.socket "$UNITDIR/hauler.socket"
sed "s|@BINDIR@|$BINDIR|" dist/systemd/hauler.service > "$UNITDIR/hauler.service"

systemctl --user daemon-reload
# Restart picks up a new binary if the daemon was already running.
systemctl --user enable --now hauler.socket
if systemctl --user is-active --quiet hauler.service; then
    systemctl --user restart hauler.service
fi
echo "installed to $BINDIR; try: hauler add <url> && hauler watch"
