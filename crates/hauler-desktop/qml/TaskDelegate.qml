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
    // Narrow rows fold the secondary buttons into a menu and say less.
    readonly property bool compact: width < Kirigami.Units.gridUnit * 26
    readonly property bool wide: width >= Kirigami.Units.gridUnit * 36

    signal detailsRequested(int taskId)

    width: ListView.view.width
    onClicked: detailsRequested(taskId)

    TaskState {
        id: taskState

        status: item.status
        speed: item.speed
        received: item.received
    }

    function amountText() {
        if (size <= 0)
            return Fmt.bytes(received);
        return compact ? Fmt.bytes(received) : qsTr("%1 of %2").arg(Fmt.bytes(received)).arg(Fmt.bytes(size));
    }

    // The grey part of the status line, after the coloured state word.
    function detailText() {
        const parts = [];
        switch (status) {
        case "running":
            if (taskState.stalled) {
                if (received > 0)
                    parts.push(amountText());
                break;
            }
            if (!compact || size <= 0)
                parts.push(amountText());
            parts.push(Fmt.speed(speed));
            if (size > 0)
                parts.push(qsTr("%1 left").arg(Fmt.eta((size - received) / speed)));
            if (wide)
                parts.push(connections === 1 ? qsTr("1 connection") : qsTr("%1 connections").arg(connections));
            break;
        case "queued":
        case "paused":
        case "waiting":
            if (received > 0)
                parts.push(amountText());
            if (status === "paused" && !resumable)
                parts.push(qsTr("will restart"));
            break;
        case "completed":
            parts.push(Fmt.bytes(size > 0 ? size : received));
            if (!compact)
                parts.push(Fmt.host(url));
            break;
        case "failed":
            parts.push(error);
            break;
        }
        return parts.join(" · ");
    }

    // The one button every row shows: what you most likely want next.
    readonly property var primary: status === "completed" ? openAction
        : status === "paused" || status === "failed" ? resumeAction
        : pauseAction

    Kirigami.Action {
        id: pauseAction

        icon.name: "media-playback-pause"
        text: qsTr("Pause")
        visible: item.status === "running" || item.status === "queued" || item.status === "waiting"
        onTriggered: item.backend.pause(item.taskId)
    }
    Kirigami.Action {
        id: resumeAction

        icon.name: item.status === "failed" ? "view-refresh" : "media-playback-start"
        text: item.status === "failed" ? qsTr("Retry") : qsTr("Resume")
        visible: item.status === "paused" || item.status === "failed"
        onTriggered: item.backend.resume(item.taskId)
    }
    Kirigami.Action {
        id: openAction

        icon.name: "document-open"
        text: qsTr("Open")
        visible: item.status === "completed"
        onTriggered: Qt.openUrlExternally(Fmt.fileUrl(item.path))
    }
    Kirigami.Action {
        id: openFolderAction

        icon.name: "document-open-folder"
        text: qsTr("Open Folder")
        onTriggered: Qt.openUrlExternally(Fmt.fileUrl(item.dir))
    }
    Kirigami.Action {
        id: removeAction

        icon.name: "edit-delete"
        text: qsTr("Remove…")
        onTriggered: confirmRemove.open()
    }

    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: menu.popup()
    }
    TapHandler {
        acceptedDevices: PointerDevice.TouchScreen
        onLongPressed: menu.popup()
    }

    contentItem: RowLayout {
        spacing: Kirigami.Units.largeSpacing

        FileIcon {
            Layout.alignment: Qt.AlignVCenter
            size: Kirigami.Units.iconSizes.medium
            source: item.fileName ? item.backend.iconForFile(item.fileName) : "folder-download"
            emblem: taskState.emblem
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                QQC2.Label {
                    Layout.fillWidth: true
                    text: item.displayName
                    elide: Text.ElideMiddle
                    font.weight: Font.DemiBold
                }
                QQC2.Label {
                    visible: item.size > 0 && item.status !== "completed" && item.status !== "failed"
                    text: Fmt.percent(item.fraction)
                    font.features: { "tnum": 1 }
                    color: Kirigami.Theme.disabledTextColor
                }
            }

            ProgressStrip {
                Layout.fillWidth: true
                visible: item.size > 0 && item.status !== "completed" && (item.status === "running" || item.received > 0)
                value: item.fraction
                fillColor: taskState.barColor
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                QQC2.Label {
                    text: taskState.label
                    font: Kirigami.Theme.smallFont
                    color: taskState.color
                }
                QQC2.Label {
                    id: details

                    Layout.fillWidth: true
                    readonly property string detail: item.detailText()

                    visible: detail !== ""
                    text: "· " + detail
                    elide: Text.ElideRight
                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                    font.features: { "tnum": 1 }
                    color: item.status === "failed" ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.disabledTextColor
                    opacity: item.status === "failed" ? 0.8 : 1

                    QQC2.ToolTip.text: item.error
                    QQC2.ToolTip.visible: item.status === "failed" && truncated && hoverHandler.hovered
                    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay

                    HoverHandler {
                        id: hoverHandler
                    }
                }
                Item {
                    visible: !details.visible
                    Layout.fillWidth: true
                }
            }
        }

        RowLayout {
            Layout.alignment: Qt.AlignVCenter
            spacing: 0

            QQC2.ToolButton {
                action: item.primary
                display: QQC2.AbstractButton.IconOnly
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
            }
            QQC2.ToolButton {
                visible: !item.compact && item.status === "completed"
                action: openFolderAction
                display: QQC2.AbstractButton.IconOnly
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
            }
            QQC2.ToolButton {
                visible: !item.compact
                icon.name: "edit-delete"
                text: qsTr("Remove")
                display: QQC2.AbstractButton.IconOnly
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                onClicked: confirmRemove.open()
            }
            QQC2.ToolButton {
                id: overflow

                visible: item.compact
                icon.name: "overflow-menu"
                text: qsTr("More")
                display: QQC2.AbstractButton.IconOnly
                down: menu.visible
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered && !menu.visible
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                onPressed: menu.popup(overflow, 0, overflow.height)
            }
        }
    }

    QQC2.Menu {
        id: menu

        QQC2.MenuItem {
            action: pauseAction
            visible: pauseAction.visible
            height: visible ? implicitHeight : 0
        }
        QQC2.MenuItem {
            action: resumeAction
            visible: resumeAction.visible
            height: visible ? implicitHeight : 0
        }
        QQC2.MenuItem {
            action: openAction
            visible: openAction.visible
            height: visible ? implicitHeight : 0
        }
        QQC2.MenuItem {
            action: openFolderAction
        }
        QQC2.MenuItem {
            icon.name: "documentinfo"
            text: qsTr("Details")
            onTriggered: item.detailsRequested(item.taskId)
        }
        QQC2.MenuItem {
            icon.name: "edit-copy"
            text: qsTr("Copy Link")
            onTriggered: item.backend.copyText(item.url)
        }
        QQC2.MenuSeparator {}
        QQC2.MenuItem {
            action: removeAction
        }
    }

    Kirigami.PromptDialog {
        id: confirmRemove

        parent: QQC2.Overlay.overlay
        title: qsTr("Remove Download?")
        subtitle: item.displayName
        standardButtons: QQC2.Dialog.Cancel
        customFooterActions: Kirigami.Action {
            icon.name: "edit-delete"
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
