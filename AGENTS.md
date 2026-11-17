# hauler

Fast, resumable multi-connection downloader for Linux: engine library, `hauler` CLI, `haulerd` daemon and a Kirigami desktop app.

## Commands
- Build: `cargo build --workspace` (the desktop crate needs Qt 6, Kirigami, KItemModels, qqc2-desktop-style and cmake).
- Test: `cargo test --workspace --locked`.
- Format: `cargo fmt --all --check` (`rustfmt.toml`: max_width 120).
- Lint: `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- File length: `scripts/check-loc.sh` (CI fails if any `.rs`, `.qml`, `.cpp`, `.h` or `.sh` file under `crates/` or `scripts/` exceeds 400 lines; split into modules).
- Install for your user: `scripts/install-user.sh` (release build into `~/.local/bin`, systemd user units, app menu entry).
- Benchmark: `scripts/bench.sh <size-mib> <kib-per-sec> <slow-every> <connections>` (vs aria2c, local throttled test server).
- Desktop screenshots, headless: `QT_QPA_PLATFORM=offscreen QT_QPA_PLATFORMTHEME=kde HAULER_DESKTOP_DEMO=/tmp/shot target/debug/hauler-desktop`.
- Settings page tests: `node --test 'tests/uq/*.test.mjs'`, `QML_XHR_ALLOW_FILE_READ=1 QT_QPA_PLATFORM=offscreen qmltestrunner -import tests/uq/stub -input tests/uq`, and `cargo test -p hauler-cli --test uq_page`; after changing what a command prints, `UPDATE_FIXTURES=1` on the last one rewrites `tests/uq/fixtures/` (`tests/uq/README.md`).
- Debug logs: `HAULER_LOG=debug`.

- Theme check, headless: copy a folder from uQ's `crates/uq-core/tests/golden/<theme>` to `$XDG_STATE_HOME/uq/theme`, run `hauler-desktop` under Xvfb; swapping the folder (rename) must restyle the open window.

## Layout
- `crates/hauler-engine`: download library (probe, segments, resume state, speed limits).
- `crates/hauler-ipc`: JSON-RPC types and async client for the daemon socket.
- `crates/hauler-daemon`: `haulerd` (queue, SQLite store, RPC server, D-Bus; `src/bus.rs` serves `org.hauler.Hauler1` and uQ's launcher provider on the session bus).
- `crates/hauler-cli`: `hauler`. `crates/hauler-desktop`: Kirigami UI via cxx-qt (`qml/`, `cpp/`, `src/`, `build.rs`).
- `crates/hauler-testserver`: throttled HTTP server with Range/ETag for tests and benchmarks.
- `uq/`: uQ desktop integration (bar widget, launcher provider, Downloads page in Settings, D-Bus service file), installed by the PKGBUILD; check it with `uqctl integrations check uq/` from uQ.
- `dist/`: arch PKGBUILD, systemd units, desktop file. `scripts/`: build-appimage, build-static, install-user, bench, check-loc.

## Conventions
- Rust edition 2024, MSRV 1.88; dual MIT/Apache-2.0. Work on `main`; CI runs on push to main and PRs.
- Releases: push a tag `vX.Y.Z`. The Packages workflow fails unless the tag equals the `hauler-cli` crate version, then builds AppImage (Debian 13), Arch package and static musl tarball and publishes a GitHub release with `SHA256SUMS`. Workspace version is in `Cargo.toml`.
- Removing a download must only delete files that download created (`.part`, `.hauler`, the finished file); tests cover this.

## Gotchas
- CI tests run in an `archlinux` container because of Qt 6/Kirigami.
- The static build needs `musl-tools`, cmake and `rustup target add x86_64-unknown-linux-musl`; AppImage build needs linuxdeploy in `target/appimage-tools/`.
