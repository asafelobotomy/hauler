import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page

    required property var backend
    required property var settings

    title: qsTr("Settings")

    actions: [
        Kirigami.Action {
            icon.name: "dialog-ok-apply"
            text: qsTr("Apply")
            onTriggered: {
                page.backend.saveSettings(JSON.stringify({
                    max_active: maxActive.value,
                    connections: connections.value,
                    global_limit: limit.value * 1024,
                    download_dir: dirField.text,
                    pause_on_metered: metered.checked
                }));
                applicationWindow().pageStack.pop();
            }
        }
    ]

    // Grey help text under a setting.
    component Hint: QQC2.Label {
        Layout.fillWidth: true
        Layout.maximumWidth: Kirigami.Units.gridUnit * 20
        font: Kirigami.Theme.smallFont
        color: Kirigami.Theme.disabledTextColor
        wrapMode: Text.Wrap
    }

    Kirigami.FormLayout {
        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: qsTr("Downloads")
        }
        RowLayout {
            Kirigami.FormData.label: qsTr("Save to:")
            Layout.fillWidth: true

            QQC2.TextField {
                id: dirField

                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 14
                text: page.settings.download_dir
            }
            QQC2.Button {
                icon.name: "document-open-folder"
                text: qsTr("Browse…")
                display: page.width < Kirigami.Units.gridUnit * 26 ? QQC2.AbstractButton.IconOnly : QQC2.AbstractButton.TextBesideIcon
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered && display === QQC2.AbstractButton.IconOnly
                onClicked: folderDialog.open()
            }
        }
        QQC2.SpinBox {
            id: maxActive

            Kirigami.FormData.label: qsTr("Downloads at once:")
            from: 1
            to: 50
            value: page.settings.max_active
        }
        Hint {
            text: qsTr("Others wait in the queue, highest priority first.")
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: qsTr("Speed")
        }
        QQC2.SpinBox {
            id: connections

            Kirigami.FormData.label: qsTr("Connections per download:")
            from: 1
            to: 32
            value: page.settings.connections
        }
        Hint {
            text: qsTr("More connections are often faster. If a server turns some away, the rest carry on.")
        }
        QQC2.SpinBox {
            id: limit

            Kirigami.FormData.label: qsTr("Total speed limit:")
            from: 0
            to: 1000000
            stepSize: 256
            editable: true
            value: Math.round(page.settings.global_limit / 1024)
            textFromValue: v => v === 0 ? qsTr("None") : qsTr("%1 KiB/s").arg(v)
            valueFromText: t => parseInt(t) || 0
        }
        Hint {
            text: qsTr("Shared by all downloads. Each download can have its own limit as well.")
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: qsTr("Network")
        }
        QQC2.Switch {
            id: metered

            Kirigami.FormData.label: qsTr("Metered connections:")
            text: qsTr("Pause downloads")
            checked: page.settings.pause_on_metered
        }
        Hint {
            text: qsTr("Downloads continue by themselves once you are on an unmetered network again.")
        }
    }

    FolderDialog {
        id: folderDialog

        currentFolder: "file://" + dirField.text
        onAccepted: dirField.text = decodeURIComponent(selectedFolder.toString().replace(/^file:\/\//, ""))
    }
}
