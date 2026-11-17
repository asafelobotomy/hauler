#include "hauler-desktop/cpp/theme.h"

#include <QApplication>
#include <QColor>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QFileSystemWatcher>
#include <QFont>
#include <QIcon>
#include <QJsonDocument>
#include <QJsonObject>
#include <QObject>
#include <QPalette>
#include <QSettings>
#include <QStandardPaths>
#include <QString>
#include <QTimer>

namespace {

QString themeDir()
{
    QString state = qEnvironmentVariable("XDG_STATE_HOME");
    if (state.isEmpty() || !QDir::isAbsolutePath(state))
        state = QDir::homePath() + QStringLiteral("/.local/state");
    return state + QStringLiteral("/uq/theme");
}

// "r,g,b" (a KDE colour scheme value; QSettings reads the commas as a list).
QColor readColor(QSettings &s, const QString &group, const QString &key, const QColor &fallback)
{
    const QStringList v = s.value(group + QLatin1Char('/') + key).toStringList();
    if (v.size() < 3)
        return fallback;
    const QColor c(v[0].trimmed().toInt(), v[1].trimmed().toInt(), v[2].trimmed().toInt());
    return c.isValid() ? c : fallback;
}

// What KColorScheme::createApplicationPalette gives, from the same file, without linking KF6:
// the roles Fusion, qqc2-desktop-style and our own QML draw with.
QPalette paletteFrom(const QString &path)
{
    QSettings s(path, QSettings::IniFormat);
    QPalette p = QApplication::palette();
    const QColor none;

    const auto set = [&](QPalette::ColorRole role, const QColor &c) { p.setColor(role, c); };
    const auto disabled = [&](QPalette::ColorRole role, const QColor &c) { p.setColor(QPalette::Disabled, role, c); };

    const QString win = QStringLiteral("Colors:Window"), view = QStringLiteral("Colors:View"),
                  btn = QStringLiteral("Colors:Button"), sel = QStringLiteral("Colors:Selection"),
                  tip = QStringLiteral("Colors:Tooltip");

    const QColor window = readColor(s, win, QStringLiteral("BackgroundNormal"), p.color(QPalette::Window));
    const QColor windowText = readColor(s, win, QStringLiteral("ForegroundNormal"), p.color(QPalette::WindowText));
    const QColor base = readColor(s, view, QStringLiteral("BackgroundNormal"), p.color(QPalette::Base));
    const QColor text = readColor(s, view, QStringLiteral("ForegroundNormal"), p.color(QPalette::Text));
    const QColor button = readColor(s, btn, QStringLiteral("BackgroundNormal"), p.color(QPalette::Button));
    const QColor buttonText = readColor(s, btn, QStringLiteral("ForegroundNormal"), p.color(QPalette::ButtonText));
    const QColor highlight = readColor(s, sel, QStringLiteral("BackgroundNormal"), p.color(QPalette::Highlight));
    const QColor highlightText = readColor(s, sel, QStringLiteral("ForegroundNormal"), p.color(QPalette::HighlightedText));

    set(QPalette::Window, window);
    set(QPalette::WindowText, windowText);
    set(QPalette::Base, base);
    set(QPalette::AlternateBase, readColor(s, view, QStringLiteral("BackgroundAlternate"), base));
    set(QPalette::Text, text);
    set(QPalette::Button, button);
    set(QPalette::ButtonText, buttonText);
    set(QPalette::Highlight, highlight);
    set(QPalette::HighlightedText, highlightText);
    set(QPalette::ToolTipBase, readColor(s, tip, QStringLiteral("BackgroundNormal"), window));
    set(QPalette::ToolTipText, readColor(s, tip, QStringLiteral("ForegroundNormal"), windowText));
    set(QPalette::Link, readColor(s, view, QStringLiteral("ForegroundLink"), p.color(QPalette::Link)));
    set(QPalette::LinkVisited, readColor(s, view, QStringLiteral("ForegroundVisited"), p.color(QPalette::LinkVisited)));
    set(QPalette::BrightText, readColor(s, view, QStringLiteral("ForegroundNegative"), p.color(QPalette::BrightText)));
    const QColor muted = readColor(s, view, QStringLiteral("ForegroundInactive"), none);
    if (muted.isValid()) {
        set(QPalette::PlaceholderText, muted);
        disabled(QPalette::WindowText, muted);
        disabled(QPalette::Text, muted);
        disabled(QPalette::ButtonText, muted);
    }

    // The 3D shades Fusion uses for frames and separators, derived from the button colour like Qt does.
    set(QPalette::Light, button.lighter(150));
    set(QPalette::Midlight, button.lighter(125));
    set(QPalette::Mid, button.darker(150));
    set(QPalette::Dark, button.darker(200));
    set(QPalette::Shadow, QColor(Qt::black));
    return p;
}

bool iconThemeInstalled(const QString &theme)
{
    for (const QString &dir : QIcon::themeSearchPaths()) {
        if (QDir(dir).exists(theme + QStringLiteral("/index.theme")))
            return true;
    }
    return false;
}

void apply()
{
    const QString dir = themeDir();
    const QString colors = dir + QStringLiteral("/kde/uq.colors");
    if (!QFile::exists(colors))
        return;

    // Kirigami's colours come from KColorScheme, which reads the scheme named by this property
    // (else kdeglobals: Breeze Light outside Plasma). Set it before the palette change, which is
    // what makes Kirigami fetch the colours again.
    // KDE caches a scheme file by its path and does not notice it changed, so every apply hands
    // it a copy under a new name.
    static int serial = 0;
    static QString copy;
    const QString fresh = QStandardPaths::writableLocation(QStandardPaths::TempLocation)
        + QStringLiteral("/hauler-uq-%1-%2.colors").arg(QCoreApplication::applicationPid()).arg(++serial);
    const bool copied = QFile::copy(colors, fresh);
    if (!copy.isEmpty())
        QFile::remove(copy);
    copy = copied ? fresh : QString();
    qApp->setProperty("KDE_COLOR_SCHEME_PATH", copied ? fresh : colors);
    QApplication::setPalette(paletteFrom(colors));

    QFile f(dir + QStringLiteral("/tokens.json"));
    if (!f.open(QIODevice::ReadOnly) || f.size() > (1 << 20))
        return;
    const QJsonObject root = QJsonDocument::fromJson(f.readAll()).object();

    const QString icons = root.value(QStringLiteral("icons")).toObject().value(QStringLiteral("theme")).toString();
    if (!icons.isEmpty() && iconThemeInstalled(icons) && QIcon::themeName() != icons)
        QIcon::setThemeName(icons);

    const QJsonObject fonts = root.value(QStringLiteral("fonts")).toObject();
    const QString family = fonts.value(QStringLiteral("ui")).toString();
    const int size = fonts.value(QStringLiteral("size")).toInt();
    QFont font = QApplication::font();
    bool changed = false;
    if (!family.isEmpty() && font.family() != family) {
        font.setFamily(family);
        changed = true;
    }
    if (size >= 1 && size <= 72 && font.pointSize() != size) {
        font.setPointSize(size);
        changed = true;
    }
    if (changed)
        QApplication::setFont(font);
}

// uqd replaces these files (and sometimes the folders) when a theme is applied, which drops
// them from the watcher: watch again after every change, and the parent folder too, so a theme
// swapped in by a rename, or installed later, is still picked up.
void watchAll(QFileSystemWatcher &w)
{
    const QString dir = themeDir();
    QStringList want = {dir, dir + QStringLiteral("/kde"), dir + QStringLiteral("/kde/uq.colors"),
                        dir + QStringLiteral("/tokens.json")};
    // The folder holding `theme` sees it swapped in by a rename; the nearest existing parent
    // sees it appear.
    QDir parent(dir);
    parent.cdUp();
    while (!parent.exists() && parent.cdUp()) {
    }
    want << parent.absolutePath();
    const QStringList have = w.files() + w.directories();
    for (const QString &p : want) {
        if (QFileInfo::exists(p) && !have.contains(p))
            w.addPath(p);
    }
}

} // namespace

void hauler_follow_uq_theme()
{
    apply();

    auto *watcher = new QFileSystemWatcher(qApp);
    auto *debounce = new QTimer(watcher);
    debounce->setSingleShot(true);
    debounce->setInterval(200);
    QObject::connect(debounce, &QTimer::timeout, watcher, [watcher] {
        watchAll(*watcher);
        apply();
    });
    const auto poke = [debounce] { debounce->start(); };
    QObject::connect(watcher, &QFileSystemWatcher::fileChanged, debounce, poke);
    QObject::connect(watcher, &QFileSystemWatcher::directoryChanged, debounce, poke);
    watchAll(*watcher);
    QObject::connect(qApp, &QCoreApplication::aboutToQuit, watcher, [] {
        for (const QFileInfo &f : QDir(QStandardPaths::writableLocation(QStandardPaths::TempLocation))
                 .entryInfoList({QStringLiteral("hauler-uq-%1-*.colors").arg(QCoreApplication::applicationPid())}))
            QFile::remove(f.absoluteFilePath());
    });
}
