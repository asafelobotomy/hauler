#include "hauler-desktop/cpp/app.h"

#include <QClipboard>
#include <QGuiApplication>
#include <QIcon>
#include <QMimeDatabase>
#include <QQuickWindow>
#include <QString>

namespace {
QString toQString(rust::Str s)
{
    return QString::fromUtf8(s.data(), static_cast<qsizetype>(s.size()));
}

int argc = 1;
char arg0[] = "hauler-desktop";
char *argv[] = {arg0, nullptr};
}

std::unique_ptr<QApplication> hauler_new_application(rust::Str version)
{
    auto app = std::make_unique<QApplication>(argc, argv);
    QApplication::setApplicationName(QStringLiteral("hauler"));
    QApplication::setApplicationDisplayName(QStringLiteral("Hauler"));
    QApplication::setApplicationVersion(toQString(version));
    // The Wayland app id; matches hauler.desktop for the icon and taskbar.
    QApplication::setDesktopFileName(QStringLiteral("hauler"));
    // Closing the window hides it to the tray; only Quit ends the app.
    QApplication::setQuitOnLastWindowClosed(false);
    return app;
}

int hauler_exec(QApplication &)
{
    return QApplication::exec();
}

bool hauler_grab_window(rust::Str path)
{
    const auto file = toQString(path);
    for (QWindow *w : QGuiApplication::topLevelWindows()) {
        if (auto *quick = qobject_cast<QQuickWindow *>(w); quick && quick->isVisible())
            return quick->grabWindow().save(file);
    }
    return false;
}

rust::String hauler_icon_for_file(rust::Str name)
{
    const QMimeType mime = QMimeDatabase().mimeTypeForFile(toQString(name), QMimeDatabase::MatchExtension);
    // Icon themes often lack the specific icon (application-x-iso9660-image)
    // but have the generic one (media-optical).
    QString icon = mime.iconName();
    if (!QIcon::hasThemeIcon(icon))
        icon = mime.genericIconName();
    return rust::String(icon.toStdString());
}

void hauler_copy_text(rust::Str text)
{
    QGuiApplication::clipboard()->setText(toQString(text));
}
