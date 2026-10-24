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

    Kirigami.FormLayout {
        QQC2.SpinBox {
            id: maxActive

            Kirigami.FormData.label: qsTr("Downloads at once:")
            from: 1
            to: 50
            value: page.settings.max_active
        }
        QQC2.SpinBox {
            id: connections

            Kirigami.FormData.label: qsTr("Connections per download:")
            from: 1
            to: 32
            value: page.settings.connections
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
        RowLayout {
            Kirigami.FormData.label: qsTr("Download folder:")
            Layout.fillWidth: true

            QQC2.TextField {
                id: dirField

                Layout.fillWidth: true
                Layout.minimumWidth: Kirigami.Units.gridUnit * 14
                text: page.settings.download_dir
            }
            QQC2.Button {
                icon.name: "document-open-folder"
                text: qsTr("Browse…")
                onClicked: folderDialog.open()
            }
        }
        QQC2.Switch {
            id: metered

            Kirigami.FormData.label: qsTr("Metered connections:")
            text: qsTr("Pause downloads")
            checked: page.settings.pause_on_metered
        }
    }

    FolderDialog {
        id: folderDialog

        currentFolder: "file://" + dirField.text
        onAccepted: dirField.text = decodeURIComponent(selectedFolder.toString().replace(/^file:\/\//, ""))
    }
}
