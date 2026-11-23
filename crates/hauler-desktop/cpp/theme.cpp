#include "hauler-desktop/cpp/theme.h"

#include "uqkdetheme.h"

#include <QString>

void hauler_follow_uq_theme()
{
    uqtheme::followKdeTheme(QStringLiteral("hauler"));
}
