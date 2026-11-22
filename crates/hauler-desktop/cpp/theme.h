#pragma once

// Follow uQ's theme: the KDE colour scheme, icon theme and interface font uqd renders into
// `$XDG_STATE_HOME/uq/theme`. Applied once at startup and again whenever those files change.
// Without uQ (no such folder) the system palette stays as it is.
// The work is uQ's shared uqkdetheme (cpp/uqkdetheme.*, from uQ's native/uq-theme), which noti-fi
// uses too: Kirigami caches a colour scheme by its path, so each change is handed over as a fresh
// copy.
void hauler_follow_uq_theme();
