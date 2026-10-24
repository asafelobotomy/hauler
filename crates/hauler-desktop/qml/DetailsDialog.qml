import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.hauler.desktop

Kirigami.Dialog {
    id: dialog

    required property TaskModel model
    required property var backend
    property int taskId: -1
    readonly property int row: taskId >= 0 && model.count >= 0 ? model.indexOfId(taskId) + model.changes * 0 : -1
    readonly property var task: row >= 0 ? model.get(row) : null
    readonly property bool running: task !== null && task.status === "running"
    readonly property real fraction: task && task.size > 0 ? Math.min(1, task.received / task.size) : 0
    // Two stat tiles a row in a narrow window. Not the dialog's own width:
    // its scroll bar comes and goes with the height that this changes.
    readonly property bool narrow: parent ? parent.width < Kirigami.Units.gridUnit * 30 : false

    function openFor(id) {
        taskId = id;
        map.map = null;
        graph.clear();
        backend.requestSegments(id);
        open();
    }

    parent: QQC2.Overlay.overlay
    title: task ? (task.fileName || qsTr("Download")) : ""
    preferredWidth: Kirigami.Units.gridUnit * 36
    // Leave room for the open animation, which slides the dialog down by
    // two grid units: a dialog that fills the window gets pushed back by the
    // popup margins, and its position never settles.
    maximumHeight: parent ? parent.height - Kirigami.Units.gridUnit * 5 : Infinity
    standardButtons: QQC2.Dialog.Close
    padding: Kirigami.Units.largeSpacing

    onClosed: taskId = -1

    Connections {
        target: dialog.backend

        function onSegments(id, json) {
            if (id === dialog.taskId)
                map.map = JSON.parse(json);
        }
    }

    // Once a second while running: refresh the map and add a speed sample.
    Timer {
        interval: 1000
        repeat: true
        running: dialog.visible && dialog.running
        onTriggered: {
            dialog.backend.requestSegments(dialog.taskId);
            graph.push(dialog.task.speed);
        }
    }

    component StatTile: Rectangle {
        id: tile

        property string caption
        property string value

        Layout.fillWidth: true
        Layout.preferredWidth: 1
        implicitHeight: tileColumn.implicitHeight + Kirigami.Units.largeSpacing * 2
        radius: Kirigami.Units.smallSpacing
        color: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.backgroundColor, Kirigami.Theme.textColor, 0.05)

        ColumnLayout {
            id: tileColumn

            anchors.fill: parent
            anchors.margins: Kirigami.Units.largeSpacing
            spacing: 0

            QQC2.Label {
                Layout.fillWidth: true
                text: tile.caption
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }
            Kirigami.Heading {
                Layout.fillWidth: true
                level: 3
                text: tile.value
                font.features: { "tnum": 1 }
                elide: Text.ElideRight
            }
        }
    }

    component ActionButton: QQC2.ToolButton {
        display: QQC2.AbstractButton.IconOnly
        QQC2.ToolTip.text: text
        QQC2.ToolTip.visible: hovered
        QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        TaskState {
            id: taskState

            status: dialog.task ? dialog.task.status : ""
            speed: dialog.task ? dialog.task.speed : 0
            received: dialog.task ? dialog.task.received : 0
        }

        // Icon, state and the main actions.
        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing

            FileIcon {
                size: Kirigami.Units.iconSizes.large
                source: dialog.task && dialog.task.fileName ? dialog.backend.iconForFile(dialog.task.fileName) : "folder-download"
                emblem: taskState.emblem
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                QQC2.Label {
                    Layout.fillWidth: true
                    text: taskState.label
                    color: taskState.color
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    text: !dialog.task ? ""
                        : dialog.task.status === "failed" ? dialog.task.error
                        : dialog.task.status === "completed" ? Fmt.bytes(dialog.task.size > 0 ? dialog.task.size : dialog.task.received)
                        : dialog.task.size > 0 ? qsTr("%1 of %2 · %3").arg(Fmt.bytes(dialog.task.received)).arg(Fmt.bytes(dialog.task.size)).arg(Fmt.percent(dialog.fraction))
                        : Fmt.bytes(dialog.task.received)
                    color: Kirigami.Theme.disabledTextColor
                    elide: Text.ElideRight
                    wrapMode: dialog.task && dialog.task.status === "failed" ? Text.Wrap : Text.NoWrap
                    maximumLineCount: 3
                }
            }
            ActionButton {
                visible: dialog.task !== null && ["running", "queued", "waiting"].includes(dialog.task.status)
                icon.name: "media-playback-pause"
                text: qsTr("Pause")
                onClicked: dialog.backend.pause(dialog.taskId)
            }
            ActionButton {
                visible: dialog.task !== null && (dialog.task.status === "paused" || dialog.task.status === "failed")
                icon.name: dialog.task && dialog.task.status === "failed" ? "view-refresh" : "media-playback-start"
                text: dialog.task && dialog.task.status === "failed" ? qsTr("Retry") : qsTr("Resume")
                onClicked: dialog.backend.resume(dialog.taskId)
            }
            ActionButton {
                visible: dialog.task !== null && dialog.task.status === "completed"
                icon.name: "document-open"
                text: qsTr("Open")
                onClicked: Qt.openUrlExternally(Fmt.fileUrl(dialog.task.dir + "/" + dialog.task.fileName))
            }
            ActionButton {
                icon.name: "document-open-folder"
                text: qsTr("Open Folder")
                onClicked: Qt.openUrlExternally(Fmt.fileUrl(dialog.task.dir))
            }
        }

        ProgressStrip {
            Layout.fillWidth: true
            visible: dialog.task !== null && dialog.task.size > 0 && dialog.task.status !== "completed"
            value: dialog.fraction
            fillColor: taskState.barColor
        }

        GridLayout {
            Layout.fillWidth: true
            visible: dialog.task !== null && dialog.task.status !== "completed"
            columns: dialog.narrow ? 2 : 4
            columnSpacing: Kirigami.Units.smallSpacing
            rowSpacing: Kirigami.Units.smallSpacing

            StatTile {
                caption: qsTr("Speed")
                value: dialog.running ? Fmt.speed(dialog.task.speed) : "–"
            }
            StatTile {
                caption: qsTr("Time left")
                value: dialog.running && dialog.task.size > 0 && dialog.task.speed > 0 ? Fmt.eta((dialog.task.size - dialog.task.received) / dialog.task.speed) : "–"
            }
            StatTile {
                caption: qsTr("Downloaded")
                value: dialog.task ? Fmt.bytes(dialog.task.received) : ""
            }
            StatTile {
                caption: qsTr("Connections")
                value: dialog.running ? dialog.task.connections : "–"
            }
        }

        Kirigami.Heading {
            level: 5
            visible: graph.visible
            text: qsTr("Speed")
        }
        SpeedGraph {
            id: graph

            Layout.fillWidth: true
            visible: dialog.running || samples.length > 1
        }

        Kirigami.Heading {
            level: 5
            visible: map.visible
            text: qsTr("Segments")
        }
        SegmentMap {
            id: map

            Layout.fillWidth: true
            visible: dialog.task !== null && dialog.task.size > 0
        }

        // The map's legend, and how many pieces the file is in.
        Flow {
            Layout.fillWidth: true
            visible: map.visible && map.map !== null
            spacing: Kirigami.Units.largeSpacing

            Repeater {
                // Labels only: colours in the model would rebuild the
                // delegates on every theme change.
                model: [qsTr("Downloaded"), qsTr("In progress"), qsTr("Remaining")]

                RowLayout {
                    required property string modelData
                    required property int index

                    spacing: Kirigami.Units.smallSpacing

                    Rectangle {
                        implicitWidth: Kirigami.Units.iconSizes.small * 0.75
                        implicitHeight: implicitWidth
                        radius: 2
                        color: [map.doneColor, map.activeColor, map.trackColor][parent.index]
                    }
                    QQC2.Label {
                        text: parent.modelData
                        font: Kirigami.Theme.smallFont
                        color: Kirigami.Theme.disabledTextColor
                    }
                }
            }
            QQC2.Label {
                text: !map.map ? "" : map.map.segments.length === 1 ? qsTr("1 segment") : qsTr("%1 segments").arg(map.map.segments.length)
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
            }
        }

        Kirigami.Separator {
            Layout.fillWidth: true
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            RowLayout {
                Kirigami.FormData.label: qsTr("Address:")
                Layout.fillWidth: true
                Layout.maximumWidth: Kirigami.Units.gridUnit * 22
                spacing: 0

                QQC2.Label {
                    Layout.fillWidth: true
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 18
                    text: dialog.task ? dialog.task.url : ""
                    elide: Text.ElideMiddle
                }
                ActionButton {
                    icon.name: "edit-copy"
                    text: qsTr("Copy Link")
                    onClicked: dialog.backend.copyText(dialog.task.url)
                }
            }
            QQC2.Label {
                Kirigami.FormData.label: qsTr("Saved to:")
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 18
                Layout.maximumWidth: Kirigami.Units.gridUnit * 22
                text: dialog.task ? dialog.task.dir : ""
                elide: Text.ElideMiddle
            }
            QQC2.Label {
                Kirigami.FormData.label: qsTr("Size:")
                text: !dialog.task ? "" : dialog.task.size >= 0 ? Fmt.bytes(dialog.task.size) : qsTr("Unknown")
            }
            QQC2.Label {
                Kirigami.FormData.label: qsTr("Resumable:")
                Layout.fillWidth: true
                Layout.maximumWidth: Kirigami.Units.gridUnit * 22
                text: dialog.task && dialog.task.resumable ? qsTr("Yes") : qsTr("No, the server does not support it")
                elide: Text.ElideRight
            }
            QQC2.SpinBox {
                Kirigami.FormData.label: qsTr("Speed limit:")
                from: 0
                to: 100000
                stepSize: 64
                editable: true
                value: dialog.task ? Math.round(dialog.task.limit / 1024) : 0
                textFromValue: v => v === 0 ? qsTr("None") : qsTr("%1 KiB/s").arg(v)
                valueFromText: t => parseInt(t) || 0
                onValueModified: dialog.backend.setLimit(dialog.taskId, value * 1024)
            }
            QQC2.SpinBox {
                Kirigami.FormData.label: qsTr("Priority:")
                from: -10
                to: 10
                value: dialog.task ? dialog.task.priority : 0
                onValueModified: dialog.backend.setPriority(dialog.taskId, value)
            }
        }
    }
}
