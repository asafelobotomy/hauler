import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.hauler.desktop

QQC2.ItemDelegate {
    id: item

    required property var backend
    required property int index
    required property int taskId
    required property string url
    required property string dir
    required property string fileName
    required property string status
    required property real size
    required property real received
    required property real speed
    required property int connections
    required property bool resumable
    required property string error

    readonly property string displayName: fileName || url
    readonly property real fraction: size > 0 ? Math.min(1, received / size) : 0
    readonly property string path: dir + "/" + fileName

    signal detailsRequested(int taskId)

    width: ListView.view.width
    onClicked: detailsRequested(taskId)

    function detailText() {
        const amount = size > 0 ? qsTr("%1 of %2").arg(Fmt.bytes(received)).arg(Fmt.bytes(size)) : Fmt.bytes(received);
        switch (status) {
        case "running": {
            const parts = [amount, Fmt.speed(speed)];
            if (size > 0 && speed > 0)
                parts.push(qsTr("%1 left").arg(Fmt.duration((size - received) / speed)));
            parts.push(qsTr("%n connection(s)", "", connections));
            return parts.join(" · ");
        }
        case "queued":
            return qsTr("Queued") + (received > 0 ? " · " + amount : "");
        case "paused":
            return qsTr("Paused") + (received > 0 ? " · " + amount : "") + (resumable ? "" : " · " + qsTr("will restart"));
        case "waiting":
            return qsTr("Waiting for the network") + " · " + amount;
        case "completed":
            return Fmt.bytes(size > 0 ? size : received);
        case "failed":
            return qsTr("Failed: %1").arg(error);
        }
        return "";
    }

    contentItem: RowLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Icon {
            Layout.preferredWidth: Kirigami.Units.iconSizes.medium
            Layout.preferredHeight: Kirigami.Units.iconSizes.medium
            source: item.status === "failed" ? "dialog-error"
                : item.status === "completed" ? "document-save"
                : "folder-download"
            color: item.status === "failed" ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            QQC2.Label {
                Layout.fillWidth: true
                text: item.displayName
                elide: Text.ElideMiddle
                font.weight: Font.DemiBold
            }

            QQC2.ProgressBar {
                Layout.fillWidth: true
                visible: item.status === "running" || (item.status !== "completed" && item.received > 0)
                from: 0
                to: 1
                value: item.fraction
                indeterminate: item.status === "running" && item.size <= 0
            }

            QQC2.Label {
                Layout.fillWidth: true
                text: item.detailText()
                elide: Text.ElideRight
                font: Kirigami.Theme.smallFont
                color: item.status === "failed" ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.disabledTextColor
            }
        }

        QQC2.ToolButton {
            visible: item.status === "running" || item.status === "queued" || item.status === "waiting"
            icon.name: "media-playback-pause"
            text: qsTr("Pause")
            display: QQC2.AbstractButton.IconOnly
            QQC2.ToolTip.text: text
            QQC2.ToolTip.visible: hovered
            onClicked: item.backend.pause(item.taskId)
        }
        QQC2.ToolButton {
            visible: item.status === "paused" || item.status === "failed"
            icon.name: item.status === "failed" ? "view-refresh" : "media-playback-start"
            text: item.status === "failed" ? qsTr("Retry") : qsTr("Resume")
            display: QQC2.AbstractButton.IconOnly
            QQC2.ToolTip.text: text
            QQC2.ToolTip.visible: hovered
            onClicked: item.backend.resume(item.taskId)
        }
        QQC2.ToolButton {
            visible: item.status === "completed"
            icon.name: "document-open"
            text: qsTr("Open")
            display: QQC2.AbstractButton.IconOnly
            QQC2.ToolTip.text: text
            QQC2.ToolTip.visible: hovered
            onClicked: Qt.openUrlExternally(Fmt.fileUrl(item.path))
        }
        QQC2.ToolButton {
            visible: item.status === "completed"
            icon.name: "document-open-folder"
            text: qsTr("Open Folder")
            display: QQC2.AbstractButton.IconOnly
            QQC2.ToolTip.text: text
            QQC2.ToolTip.visible: hovered
            onClicked: Qt.openUrlExternally(Fmt.fileUrl(item.dir))
        }
        QQC2.ToolButton {
            icon.name: "edit-delete-remove"
            text: qsTr("Remove")
            display: QQC2.AbstractButton.IconOnly
            QQC2.ToolTip.text: text
            QQC2.ToolTip.visible: hovered
            onClicked: confirmRemove.open()
        }
    }

    Kirigami.PromptDialog {
        id: confirmRemove

        parent: QQC2.Overlay.overlay
        title: qsTr("Remove Download?")
        subtitle: item.displayName
        standardButtons: QQC2.Dialog.Cancel
        customFooterActions: Kirigami.Action {
            icon.name: "edit-delete-remove"
            text: qsTr("Remove")
            onTriggered: {
                item.backend.remove(item.taskId, deleteFiles.checked);
                confirmRemove.close();
            }
        }

        QQC2.CheckBox {
            id: deleteFiles

            text: item.status === "completed" ? qsTr("Also delete the downloaded file") : qsTr("Also delete the partial file")
            checked: item.status !== "completed"
        }
    }
}
