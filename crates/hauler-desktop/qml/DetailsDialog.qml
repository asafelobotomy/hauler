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

    function openFor(id) {
        taskId = id;
        map.map = null;
        backend.requestSegments(id);
        open();
    }

    parent: QQC2.Overlay.overlay
    title: task ? (task.fileName || qsTr("Download")) : ""
    preferredWidth: Kirigami.Units.gridUnit * 34
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

    Timer {
        interval: 1000
        repeat: true
        running: dialog.visible && dialog.task !== null && dialog.task.status === "running"
        onTriggered: dialog.backend.requestSegments(dialog.taskId)
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        SegmentMap {
            id: map

            Layout.fillWidth: true
            visible: dialog.task !== null && dialog.task.size > 0
        }

        QQC2.Label {
            Layout.fillWidth: true
            visible: map.visible && map.map !== null
            text: map.map ? qsTr("%n segment(s)", "", map.map.segments.length) : ""
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            QQC2.Label {
                Kirigami.FormData.label: qsTr("Address:")
                Layout.fillWidth: true
                Layout.maximumWidth: Kirigami.Units.gridUnit * 22
                text: dialog.task ? dialog.task.url : ""
                elide: Text.ElideMiddle
            }
            QQC2.Label {
                Kirigami.FormData.label: qsTr("Saved to:")
                Layout.maximumWidth: Kirigami.Units.gridUnit * 22
                text: dialog.task ? dialog.task.dir : ""
                elide: Text.ElideMiddle
            }
            QQC2.Label {
                Kirigami.FormData.label: qsTr("Size:")
                text: dialog.task ? Fmt.bytes(dialog.task.size) : ""
            }
            QQC2.Label {
                Kirigami.FormData.label: qsTr("Downloaded:")
                text: dialog.task ? Fmt.bytes(dialog.task.received) : ""
            }
            QQC2.Label {
                Kirigami.FormData.label: qsTr("Resumable:")
                text: dialog.task && dialog.task.resumable ? qsTr("Yes") : qsTr("No, the server does not support it")
            }
            RowLayout {
                Kirigami.FormData.label: qsTr("Speed limit:")

                QQC2.SpinBox {
                    id: limit

                    from: 0
                    to: 100000
                    stepSize: 64
                    editable: true
                    value: dialog.task ? Math.round(dialog.task.limit / 1024) : 0
                    textFromValue: v => v === 0 ? qsTr("None") : qsTr("%1 KiB/s").arg(v)
                    valueFromText: t => parseInt(t) || 0
                    onValueModified: dialog.backend.setLimit(dialog.taskId, value * 1024)
                }
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
