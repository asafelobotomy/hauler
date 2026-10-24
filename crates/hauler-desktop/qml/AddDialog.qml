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
    readonly property int defaultConnections: settings ? settings.connections : 8

    function openWith(text) {
        urlField.text = (text || "").trim();
        nameField.text = "";
        dirField.text = settings ? settings.download_dir : "";
        connections.value = defaultConnections;
        limit.value = 0;
        priority.value = 0;
        paused.checked = false;
        advanced.expanded = false;
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

    ColumnLayout {
        spacing: Kirigami.Units.smallSpacing

        QQC2.ScrollView {
            Layout.fillWidth: true
            Layout.minimumWidth: Kirigami.Units.gridUnit * 14
            Layout.preferredHeight: Kirigami.Units.gridUnit * 5

            QQC2.TextArea {
                id: urlField

                placeholderText: qsTr("Paste or drop links here, one per line")
                wrapMode: TextEdit.WrapAnywhere
            }
        }

        // What the text box holds, as the dialog reads it.
        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            readonly property bool empty: urlField.text.trim() === ""
            readonly property bool bad: !empty && dialog.urls.length === 0

            Kirigami.Icon {
                Layout.preferredWidth: Kirigami.Units.iconSizes.small
                Layout.preferredHeight: Kirigami.Units.iconSizes.small
                source: parent.empty ? "link" : parent.bad ? "dialog-warning" : "emblem-success"
                opacity: parent.empty ? 0.6 : 1
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: parent.empty ? qsTr("Only http and https links are downloaded")
                    : parent.bad ? qsTr("No http or https links found")
                    : dialog.urls.length === 1 ? qsTr("1 link found") : qsTr("%1 links found").arg(dialog.urls.length)
                font: Kirigami.Theme.smallFont
                color: parent.empty ? Kirigami.Theme.disabledTextColor
                    : parent.bad ? Kirigami.Theme.negativeTextColor
                    : Kirigami.Theme.positiveTextColor
                elide: Text.ElideRight
            }
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            RowLayout {
                Kirigami.FormData.label: qsTr("Save to:")
                Layout.fillWidth: true

                QQC2.TextField {
                    id: dirField

                    Layout.fillWidth: true
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 10
                }
                QQC2.Button {
                    icon.name: "document-open-folder"
                    text: qsTr("Browse…")
                    display: dialog.width < Kirigami.Units.gridUnit * 24 ? QQC2.AbstractButton.IconOnly : QQC2.AbstractButton.TextBesideIcon
                    QQC2.ToolTip.text: text
                    QQC2.ToolTip.visible: hovered && display === QQC2.AbstractButton.IconOnly
                    onClicked: folderDialog.open()
                }
            }
            QQC2.TextField {
                id: nameField

                Kirigami.FormData.label: qsTr("File name:")
                Layout.fillWidth: true
                enabled: dialog.urls.length <= 1
                placeholderText: dialog.urls.length > 1 ? qsTr("From the server, for each link") : qsTr("From the server")
            }
            QQC2.CheckBox {
                id: paused

                text: qsTr("Add paused")
            }

            QQC2.Button {
                id: advanced

                property bool expanded: false

                flat: true
                icon.name: expanded ? "arrow-down" : "arrow-right"
                text: expanded ? qsTr("Fewer options") : qsTr("More options")
                onClicked: expanded = !expanded
            }

            QQC2.SpinBox {
                id: connections

                visible: advanced.expanded
                Kirigami.FormData.label: qsTr("Connections:")
                from: 1
                to: 32
            }
            QQC2.SpinBox {
                id: limit

                visible: advanced.expanded
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

                visible: advanced.expanded
                Kirigami.FormData.label: qsTr("Priority:")
                from: -10
                to: 10
                textFromValue: v => v > 0 ? "+" + v : String(v)
                valueFromText: t => parseInt(t) || 0
            }
        }
    }

    FolderDialog {
        id: folderDialog

        currentFolder: dirField.text ? "file://" + dirField.text : ""
        onAccepted: dirField.text = decodeURIComponent(selectedFolder.toString().replace(/^file:\/\//, ""))
    }
}
