import QtQuick
import org.kde.kirigami as Kirigami

// A slim, static progress bar. Unlike QQC2.ProgressBar it never animates,
// so a screen of stalled downloads costs nothing to draw.
Rectangle {
    property real value: 0
    property color fillColor: Kirigami.Theme.highlightColor

    implicitHeight: Math.round(Kirigami.Units.smallSpacing * 1.5)
    radius: height / 2
    color: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.backgroundColor, Kirigami.Theme.textColor, 0.12)

    Rectangle {
        width: parent.value > 0 ? Math.max(parent.height, parent.width * Math.min(1, parent.value)) : 0
        height: parent.height
        radius: parent.radius
        color: parent.fillColor
    }
}
