#!/usr/bin/env bash
# Build fully static x86_64 binaries of the CLI and daemon (no Qt, no
# glibc requirement) and pack them with the systemd units:
#
#   scripts/build-static.sh        # needs: rustup target x86_64-unknown-linux-musl, musl-gcc, cmake
#
# Writes target/static/hauler-<version>-x86_64-linux-musl.tar.gz.
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET=x86_64-unknown-linux-musl
VERSION=${VERSION:-$(cargo pkgid -p hauler-cli | sed 's/.*[#@]//')}
NAME=hauler-$VERSION-x86_64-linux-musl
OUT=target/static
STAGE=$OUT/$NAME

export CC_x86_64_unknown_linux_musl=${CC_x86_64_unknown_linux_musl:-musl-gcc}
cargo build --release --locked --target "$TARGET" -p hauler-cli -p hauler-daemon

rm -rf "$STAGE"
mkdir -p "$STAGE/systemd"
install -m755 "target/$TARGET/release/hauler" "target/$TARGET/release/haulerd" "$STAGE/"
install -m644 dist/systemd/hauler.socket "$STAGE/systemd/"
install -m644 dist/systemd/hauler.service "$STAGE/systemd/hauler.service.in"
install -m644 COPYING LICENSE-MIT LICENSE-APACHE README.md "$STAGE/"
cat > "$STAGE/INSTALL.txt" <<'EOF'
hauler and haulerd are static binaries with no runtime dependencies.

Install for your user:

    install -Dm755 hauler haulerd -t ~/.local/bin
    install -Dm644 systemd/hauler.socket ~/.config/systemd/user/hauler.socket
    sed "s|@BINDIR@|$HOME/.local/bin|" systemd/hauler.service.in > ~/.config/systemd/user/hauler.service
    systemctl --user daemon-reload
    systemctl --user enable --now hauler.socket

Then: hauler add <url>, hauler watch. Or run `hauler get <url>` without the daemon.
EOF

# Static means static: fail the build if anything is dynamically linked.
for bin in "$STAGE/hauler" "$STAGE/haulerd"; do
    if file "$bin" | grep -q "dynamically linked"; then
        echo "$bin is dynamically linked" >&2
        exit 1
    fi
done

tar -C "$OUT" -czf "$OUT/$NAME.tar.gz" "$NAME"
ls -la "$OUT/$NAME.tar.gz"
