// A thin progress bar drawn here, so it shows the same under every Controls style (the desktop style
// draws a ProgressBar through the widget style, and some of those leave it blank).
import QtQuick

Item {
    id: bar

    property real from: 0
    property real to: 100
    property real value: 0
    property bool indeterminate: false

    readonly property real fraction: bar.to > bar.from ? Math.max(0, Math.min(1, (bar.value - bar.from) / (bar.to - bar.from))) : 0

    implicitHeight: 6
    implicitWidth: 120

    SystemPalette {
        id: palette
    }

    Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: palette.mid
        opacity: 0.45
    }
    Item {
        anchors.fill: parent
        clip: true

        Rectangle {
            id: fill

            height: parent.height
            radius: height / 2
            color: palette.highlight
            width: bar.indeterminate ? parent.width * 0.3 : parent.width * bar.fraction
            x: 0

            Behavior on width {
                enabled: !bar.indeterminate
                NumberAnimation {
                    duration: 250
                    easing.type: Easing.OutCubic
                }
            }
            SequentialAnimation on x {
                running: bar.indeterminate && bar.visible
                loops: Animation.Infinite
                NumberAnimation {
                    from: -fill.width
                    to: bar.width
                    duration: 1400
                }
            }
        }
    }
}
