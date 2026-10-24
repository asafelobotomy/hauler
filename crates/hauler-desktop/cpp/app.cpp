#include "hauler-desktop/cpp/app.h"

#include <QGuiApplication>
#include <QQuickWindow>
#include <QString>

namespace {
int argc = 1;
char arg0[] = "hauler-desktop";
char *argv[] = {arg0, nullptr};
}

std::unique_ptr<QApplication> hauler_new_application(rust::Str version)
{
    auto app = std::make_unique<QApplication>(argc, argv);
    QApplication::setApplicationName(QStringLiteral("hauler"));
    QApplication::setApplicationDisplayName(QStringLiteral("Hauler"));
    QApplication::setApplicationVersion(QString::fromUtf8(version.data(), static_cast<qsizetype>(version.size())));
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
    const auto file = QString::fromUtf8(path.data(), static_cast<qsizetype>(path.size()));
    for (QWindow *w : QGuiApplication::topLevelWindows()) {
        if (auto *quick = qobject_cast<QQuickWindow *>(w); quick && quick->isVisible())
            return quick->grabWindow().save(file);
    }
    return false;
}
