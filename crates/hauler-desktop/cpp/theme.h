#pragma once

// Follow uQ's theme: the KDE colour scheme, icon theme and interface font uqd renders into
// `$XDG_STATE_HOME/uq/theme`. Applied once at startup and again whenever those files change.
// Without uQ (no such folder) the system palette stays as it is.
void hauler_follow_uq_theme();
