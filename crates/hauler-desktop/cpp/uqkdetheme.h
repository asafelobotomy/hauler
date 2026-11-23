// uq-theme 1.1.0
//
// For Qt 6 apps that draw with the desktop's own style (Kirigami, qqc2-desktop-style, Fusion,
// Qt Widgets) instead of a QML Theme singleton: follow uQ's theme through the KDE colour scheme,
// icon theme and interface font uqd renders into $XDG_STATE_HOME/uq/theme. Applied once and again
// whenever those files change, with no polling. Without uQ (no such folder) nothing changes.
// Vendor uqkdetheme.h and uqkdetheme.cpp unchanged (see uQ's native/uq-theme/README.md); they
// need Qt Core and Qt Gui only, not KF6.
#pragma once

#include <QPalette>
#include <QString>

namespace uqtheme {

/// Starts following uQ's theme for this application (call once, after the QGuiApplication or
/// QApplication exists). `name` names the app's temporary colour-scheme copies
/// (`<name>-uq-<pid>-<n>.colors` in the temp folder): Kirigami caches a scheme by its path, so
/// each change is handed over as a fresh copy, and the copies are removed on quit.
void followKdeTheme(const QString &name);

/// What KColorScheme::createApplicationPalette gives from a KDE colour scheme file, without
/// linking KF6: the roles Fusion, qqc2-desktop-style and QML draw with. Roles the file doesn't
/// set keep `base`'s colours.
QPalette paletteFromColors(const QString &colorsPath, const QPalette &base);

} // namespace uqtheme
