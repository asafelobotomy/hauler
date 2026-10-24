#pragma once

#include <QtWidgets/QApplication>
#include <memory>

#include "rust/cxx.h"

// A QApplication rather than a QGuiApplication: Plasma's tray integration
// builds widget menus, which abort without one.
std::unique_ptr<QApplication> hauler_new_application(rust::Str version);
int hauler_exec(QApplication &app);
// Render the first Qt Quick window to a PNG. Development aid for screenshots.
bool hauler_grab_window(rust::Str path);
