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
// The themed icon name for a file's MIME type, guessed from its name.
rust::String hauler_icon_for_file(rust::Str name);
void hauler_copy_text(rust::Str text);
