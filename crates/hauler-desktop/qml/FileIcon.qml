import QtQuick
import org.kde.kirigami as Kirigami

// A file's MIME type icon with a small state emblem in the corner.
Item {
    id: fileIcon

    property string source: "unknown"
    property string emblem: ""
    property int size: Kirigami.Units.iconSizes.medium

    implicitWidth: size
    implicitHeight: size

    Kirigami.Icon {
        anchors.fill: parent
        source: fileIcon.source
        fallback: "unknown"
    }

    Rectangle {
        readonly property int iconSize: fileIcon.size >= Kirigami.Units.iconSizes.large ? Kirigami.Units.iconSizes.smallMedium : Kirigami.Units.iconSizes.small

        visible: fileIcon.emblem !== ""
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: -Math.round(Kirigami.Units.smallSpacing / 2)
        width: iconSize + 2
        height: width
        radius: width / 2
        color: Kirigami.Theme.backgroundColor

        Kirigami.Icon {
            anchors.centerIn: parent
            width: parent.iconSize
            height: width
            source: fileIcon.emblem
        }
    }
}
