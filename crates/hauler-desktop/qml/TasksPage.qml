import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kitemmodels as KItemModels
import org.hauler.desktop

Kirigami.ScrollablePage {
    id: page

    required property TaskModel model
    required property var backend
    property bool connected: false
    property var network: null
    property real totalSpeed: 0
    property int runningCount: 0

    signal addRequested
    signal settingsRequested

    function showDetails(id) {
        details.openFor(id);
    }

    function closeDetails() {
        details.close();
    }

    title: qsTr("Downloads")

    actions: [
        Kirigami.Action {
            icon.name: "list-add"
            text: qsTr("Add…")
            enabled: page.connected
            onTriggered: page.addRequested()
        },
        Kirigami.Action {
            icon.name: "media-playback-pause"
            text: qsTr("Pause All")
            enabled: page.connected
            displayHint: Kirigami.DisplayHint.KeepVisible
            onTriggered: page.backend.pauseAll()
        },
        Kirigami.Action {
            icon.name: "media-playback-start"
            text: qsTr("Resume All")
            enabled: page.connected
            onTriggered: page.backend.resumeAll()
        },
        Kirigami.Action {
            icon.name: "configure"
            text: qsTr("Settings")
            enabled: page.connected && root.settings !== null
            displayHint: Kirigami.DisplayHint.IconOnly
            onTriggered: page.settingsRequested()
        }
    ]

    header: ColumnLayout {
        spacing: 0

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            position: Kirigami.InlineMessage.Header
            type: Kirigami.MessageType.Warning
            visible: page.connected && page.network !== null && !page.network.open
            text: page.network && !page.network.online
                ? qsTr("Offline. Downloads continue when the network is back.")
                : qsTr("Metered connection. Downloads are held.")
            actions: Kirigami.Action {
                text: qsTr("Download Anyway")
                visible: page.network !== null && page.network.online
                onTriggered: page.backend.saveSettings(JSON.stringify({ pause_on_metered: false }))
            }
        }

        QQC2.TabBar {
            id: filter

            Layout.fillWidth: true
            onCurrentIndexChanged: proxy.invalidateFilter()

            QQC2.TabButton { text: qsTr("All") }
            QQC2.TabButton { text: qsTr("Active") }
            QQC2.TabButton { text: qsTr("Finished") }
        }
    }

    ListView {
        id: list

        model: KItemModels.KSortFilterProxyModel {
            id: proxy

            sourceModel: page.model
            filterRowCallback: (row, parent) => {
                const s = page.model.get(row).status;
                switch (filter.currentIndex) {
                case 1:
                    return s !== "completed";
                case 2:
                    return s === "completed" || s === "failed";
                default:
                    return true;
                }
            }
        }

        delegate: TaskDelegate {
            backend: page.backend
            onDetailsRequested: details.openFor(taskId)
        }

        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.gridUnit * 4
            visible: list.count === 0
            icon.name: page.connected ? "folder-download" : "network-disconnect"
            text: !page.connected ? qsTr("Connecting to haulerd…")
                : filter.currentIndex === 0 ? qsTr("No downloads yet")
                : qsTr("Nothing here")
            explanation: page.connected && filter.currentIndex === 0
                ? qsTr("Paste a link with Ctrl+V, drop one here, or press Ctrl+N.")
                : ""
            helpfulAction: page.connected && filter.currentIndex === 0 ? addAction : null

            Kirigami.Action {
                id: addAction

                icon.name: "list-add"
                text: qsTr("Add Download")
                onTriggered: page.addRequested()
            }
        }
    }

    footer: QQC2.ToolBar {
        RowLayout {
            anchors.fill: parent

            QQC2.Label {
                Layout.fillWidth: true
                text: !page.connected ? qsTr("Not connected to haulerd")
                    : page.runningCount === 0 ? qsTr("Idle")
                    : qsTr("%n downloading", "", page.runningCount)
                elide: Text.ElideRight
            }
            QQC2.Label {
                visible: page.runningCount > 0
                text: Fmt.speed(page.totalSpeed)
                font.features: { "tnum": 1 }
            }
            QQC2.Label {
                readonly property int limit: root.settings ? root.settings.global_limit : 0
                visible: limit > 0
                text: qsTr("limit %1").arg(Fmt.speed(limit))
                color: Kirigami.Theme.neutralTextColor
            }
        }
    }

    // Ctrl+V outside a text field queues the clipboard's links. Text fields
    // take the shortcut first, so pasting inside the add dialog still works.
    Shortcut {
        sequences: [StandardKey.Paste]
        enabled: page.connected
        onActivated: {
            clipboard.text = "";
            clipboard.paste();
            root.openAdd(clipboard.text);
        }
    }

    TextEdit {
        id: clipboard

        visible: false
    }

    DetailsDialog {
        id: details

        model: page.model
        backend: page.backend
    }
}
