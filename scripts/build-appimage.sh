#!/usr/bin/env bash
# Build a self-contained AppImage of Hauler: the desktop app, haulerd and the
# hauler CLI, with Qt, Kirigami and their QML modules bundled.
#
#   scripts/build-appimage.sh
#
# Needs linuxdeploy-x86_64.AppImage and linuxdeploy-plugin-qt-x86_64.AppImage
# from https://github.com/linuxdeploy in target/appimage-tools/ (or set TOOLS).
# The AppImage is written to target/appimage/.
set -euo pipefail
cd "$(dirname "$0")/.."

TOOLS=${TOOLS:-target/appimage-tools}
OUT=target/appimage
APPDIR=$OUT/Hauler.AppDir
QT_PLUGINS=$(qmake6 -query QT_INSTALL_PLUGINS)

for tool in linuxdeploy-x86_64.AppImage linuxdeploy-plugin-qt-x86_64.AppImage; do
    [[ -x $TOOLS/$tool ]] || { echo "missing $TOOLS/$tool (see the header of this script)" >&2; exit 1; }
done

cargo build --release -p hauler-desktop -p hauler-daemon -p hauler-cli

# Start clean: a restored build cache, or an earlier build, may hold other
# versions' output.
rm -rf "$OUT"
mkdir -p "$APPDIR/usr/bin" "$OUT/qml-extra"
install -m755 target/release/hauler-desktop target/release/haulerd target/release/hauler "$APPDIR/usr/bin/"

# The app's own launcher and icon, renamed so the AppImage carries its icon.
sed 's/^Icon=.*/Icon=hauler/' dist/hauler.desktop > "$OUT/hauler.desktop"
icon=$(find /usr/share/icons/breeze /usr/share/icons/hicolor -name 'folder-download.svg' -print -quit 2>/dev/null || true)
[[ -n $icon ]] || { echo "no folder-download.svg icon found to use as the app icon" >&2; exit 1; }
cp "$icon" "$OUT/hauler.svg"

# QML the scanner cannot see in our sources: the Controls style is chosen at
# runtime (Plasma's on KDE, Fusion elsewhere), so name both here.
cat > "$OUT/qml-extra/RuntimeStyles.qml" <<'EOF'
import QtQuick
import QtQuick.Controls.Fusion
import org.kde.desktop
Item {}
EOF

# Plugins Qt loads at runtime that linuxdeploy-plugin-qt does not deploy:
#  - Kirigami's desktop platform plugin: the system colours and icon theme.
#  - Wayland's EGL client integration: without it the window cannot get an
#    OpenGL surface on Wayland and Qt Quick aborts at startup.
extra_plugins=(
    kf6/kirigami/platform/org.kde.desktop.so
    wayland-graphics-integration-client/libqt-plugin-wayland-egl.so
)
for plugin in "${extra_plugins[@]}"; do
    install -Dm644 "$QT_PLUGINS/$plugin" "$APPDIR/usr/plugins/$plugin"
    # Point it at the bundled libraries, like the plugins linuxdeploy deploys.
    up=$(dirname "$plugin" | sed 's|[^/]*|..|g')
    patchelf --set-rpath "\$ORIGIN/$up/../lib:\$ORIGIN" "$APPDIR/usr/plugins/$plugin"
done

version=${VERSION:-$(cargo pkgid -p hauler-desktop | sed "s/.*[#@]//")-$(git rev-parse --short HEAD)}
export QMAKE=qmake6
export QML_SOURCES_PATHS="$PWD/crates/hauler-desktop/qml:$PWD/$OUT/qml-extra"
# Wayland's platform plugin is libqwayland.so since Qt 6.10 and
# libqwayland-egl.so plus libqwayland-generic.so before; take what this Qt has.
# offscreen is tiny and lets the AppImage run headless (smoke tests, CI).
wayland_plugins=$(cd "$QT_PLUGINS/platforms" && printf '%s;' libqwayland*.so)
export EXTRA_PLATFORM_PLUGINS="${wayland_plugins}libqoffscreen.so"
export EXTRA_QT_MODULES="svg"
export LINUXDEPLOY_OUTPUT_VERSION=$version
# linuxdeploy's bundled strip is older than this toolchain's binaries.
export NO_STRIP=1
# Run the tools without FUSE (containers, sandboxes, no setuid fusermount).
export APPIMAGE_EXTRACT_AND_RUN=1
export PATH="$PWD/$TOOLS:$PATH"

cd "$OUT"
linuxdeploy-x86_64.AppImage \
    --appdir Hauler.AppDir \
    --executable Hauler.AppDir/usr/bin/hauler-desktop \
    --executable Hauler.AppDir/usr/bin/haulerd \
    --executable Hauler.AppDir/usr/bin/hauler \
    $(printf -- '--deploy-deps-only Hauler.AppDir/usr/plugins/%s ' "${extra_plugins[@]}") \
    --desktop-file hauler.desktop \
    --icon-file hauler.svg \
    --plugin qt \
    --output appimage
ls -la "$PWD/Hauler-$version-x86_64.AppImage"
