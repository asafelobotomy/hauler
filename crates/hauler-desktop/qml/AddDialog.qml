import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Dialog {
    id: dialog

    required property var backend
    property var settings: null

    readonly property var urls: urlField.text.split(/\s+/).filter(u => /^https?:\/\/\S+$/i.test(u))

    function openWith(text) {
        urlField.text = (text || "").trim();
        nameField.text = "";
        dirField.text = settings ? settings.download_dir : "";
        connections.value = settings ? settings.connections : 8;
        limit.value = 0;
        priority.value = 0;
        paused.checked = false;
        open();
        urlField.forceActiveFocus();
    }

    // Opened before the daemon's settings arrived (links on the command line):
    // fill the defaults in once they do.
    onSettingsChanged: {
        if (visible && settings && dirField.text === "") {
            dirField.text = settings.download_dir;
            connections.value = settings.connections;
        }
    }

    function submit() {
        for (const url of urls) {
            backend.add(JSON.stringify({
                url: url,
                dir: dirField.text || null,
                filename: urls.length === 1 && nameField.text ? nameField.text : null,
                connections: connections.value,
                limit: limit.value > 0 ? limit.value * 1024 : null,
                priority: priority.value,
                paused: paused.checked
            }));
        }
        close();
    }

    parent: QQC2.Overlay.overlay
    title: qsTr("Add Download")
    preferredWidth: Kirigami.Units.gridUnit * 30
    padding: Kirigami.Units.largeSpacing
    standardButtons: QQC2.Dialog.Cancel
    customFooterActions: Kirigami.Action {
        icon.name: "download"
        text: dialog.urls.length > 1 ? qsTr("Download %1 Files").arg(dialog.urls.length) : qsTr("Download")
        enabled: dialog.urls.length > 0
        onTriggered: dialog.submit()
    }

    Kirigami.FormLayout {
        QQC2.TextArea {
            id: urlField

            Kirigami.FormData.label: qsTr("Links:")
            Layout.fillWidth: true
            Layout.minimumWidth: Kirigami.Units.gridUnit * 18
            Layout.preferredHeight: Kirigami.Units.gridUnit * 4
            placeholderText: qsTr("https://… (one per line)")
            wrapMode: TextEdit.WrapAnywhere
        }
        QQC2.Label {
            visible: urlField.text.length > 0 && dialog.urls.length === 0
            text: qsTr("No http or https links found")
            color: Kirigami.Theme.negativeTextColor
        }
        RowLayout {
            Kirigami.FormData.label: qsTr("Save to:")
            Layout.fillWidth: true

            QQC2.TextField {
                id: dirField

                Layout.fillWidth: true
            }
            QQC2.Button {
                icon.name: "document-open-folder"
                text: qsTr("Browse…")
                onClicked: folderDialog.open()
            }
        }
        QQC2.TextField {
            id: nameField

            Kirigami.FormData.label: qsTr("File name:")
            Layout.fillWidth: true
            enabled: dialog.urls.length <= 1
            placeholderText: qsTr("From the server")
        }
        QQC2.SpinBox {
            id: connections

            Kirigami.FormData.label: qsTr("Connections:")
            from: 1
            to: 32
        }
        QQC2.SpinBox {
            id: limit

            Kirigami.FormData.label: qsTr("Speed limit:")
            from: 0
            to: 100000
            stepSize: 64
            editable: true
            textFromValue: v => v === 0 ? qsTr("None") : qsTr("%1 KiB/s").arg(v)
            valueFromText: t => parseInt(t) || 0
        }
        QQC2.SpinBox {
            id: priority

            Kirigami.FormData.label: qsTr("Priority:")
            from: -10
            to: 10
        }
        QQC2.CheckBox {
            id: paused

            text: qsTr("Add paused")
        }
    }

    FolderDialog {
        id: folderDialog

        currentFolder: dirField.text ? "file://" + dirField.text : ""
        onAccepted: dirField.text = decodeURIComponent(selectedFolder.toString().replace(/^file:\/\//, ""))
    }
}
