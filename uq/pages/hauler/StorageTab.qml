// The Storage tab: where downloads go, how full that disk is, how much space to keep free, and unfinished
// leftovers that nothing is working on.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.uq.settings
import "logic.js" as Logic

SettingsTab {
    id: tab

    property var host
    readonly property var storage: tab.host ? tab.host.storage : null
    readonly property bool loaded: tab.storage !== null && tab.storage.ok
    readonly property var reserve: Logic.reserveChoices(tab.storage !== null && tab.storage.ok ? tab.storage.minFree : undefined)
    readonly property int rev: tab.host ? tab.host.configRev : 0

    title: "Storage"
    iconName: "drive-harddisk"

    // After a save (accepted or refused) the controls show what Hauler holds, not what was typed or chosen.
    onRevChanged: {
        folder.text = Qt.binding(() => tab.loaded ? tab.storage.dir : "");
        reserveBox.currentIndex = Qt.binding(() => tab.reserve.index);
    }

    FolderDialog {
        id: folderDialog
        title: "Choose the download folder"
        onAccepted: tab.host.setConfig("download_dir", Logic.folderPath(selectedFolder), false)
    }

    SettingsCard {
        title: "Download folder"
        iconName: "folder"
        help: "Where new downloads are saved when a download doesn't ask for another place. Downloads already started stay where they are."
        wide: true
        Layout.fillWidth: true

        Loading {
            loading: tab.storage === null
            problem: tab.storage !== null && !tab.storage.ok ? tab.storage.problem : ""
            what: "the download folder"
            Layout.fillWidth: true
        }
        SettingRow {
            settingId: "folder"
            label: "Folder"
            highlight: tab.host ? tab.host.highlight : ""
            visible: tab.loaded
            enabled: !!tab.host && !tab.host.busy
            Layout.fillWidth: true

            QQC2.TextField {
                id: folder
                objectName: "folder-field"
                text: tab.loaded ? tab.storage.dir : ""
                implicitWidth: 320
                onEditingFinished: {
                    if (!tab.loaded || folder.text === tab.storage.dir)
                        return;
                    const why = Logic.folderProblem(folder.text);
                    if (why !== "") {
                        tab.host.setNote("cfg:download_dir", { message: why, failed: true });
                        folder.text = Qt.binding(() => tab.storage.dir);
                        return;
                    }
                    tab.host.setConfig("download_dir", folder.text, false);
                }
            }
            QQC2.Button {
                objectName: "browse-folder"
                text: "Choose…"
                onClicked: folderDialog.open()
            }
        }
        UqLabel {
            objectName: "folder-missing"
            text: "That folder doesn't exist any more. New downloads will be refused until you choose another."
            visible: tab.loaded && !tab.storage.dirOk
            wrapMode: Text.Wrap
            color: tab.host ? tab.host.errorColor() : "red"
            Layout.fillWidth: true
        }
        Outcome {
            host: tab.host
            noteKey: "cfg:download_dir"
            Layout.fillWidth: true
        }
    }
    SettingsCard {
        title: "Disk space"
        iconName: "drive-harddisk"
        help: "Finished files are yours: Hauler never counts or deletes anything it didn't download."
        wide: true
        visible: tab.loaded
        Layout.fillWidth: true

        SettingRow {
            settingId: "space"
            label: tab.loaded ? tab.storage.freeLine : ""
            highlight: tab.host ? tab.host.highlight : ""
            Layout.fillWidth: true
        }
        FlatBar {
            objectName: "space-bar"
            visible: tab.loaded && tab.storage.usedFraction >= 0
            from: 0
            to: 1
            value: tab.loaded ? Math.max(0, tab.storage.usedFraction) : 0
            Layout.fillWidth: true
        }
        Repeater {
            model: tab.loaded ? Logic.storageLines(tab.storage) : []

            UqLabel {
                required property string modelData

                text: modelData
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
        }
    }
    SettingsCard {
        title: "Keep space free"
        iconName: "dialog-warning"
        help: "A download that would leave less than this free is refused before it starts, instead of filling the disk."
        visible: tab.loaded
        Layout.fillWidth: true

        SettingRow {
            settingId: "reserve"
            label: "Always leave free"
            highlight: tab.host ? tab.host.highlight : ""
            enabled: !!tab.host && !tab.host.busy
            Layout.fillWidth: true

            UqComboBox {
                id: reserveBox
                objectName: "reserve"
                model: tab.reserve.labels
                currentIndex: tab.reserve.index
                onActivated: index => tab.host.setConfig("min_free", tab.reserve.values[index], false)
            }
        }
        Outcome {
            host: tab.host
            noteKey: "cfg:min_free"
            Layout.fillWidth: true
        }
    }
    SettingsCard {
        title: "Unfinished leftovers"
        iconName: "edit-delete"
        help: "A partly downloaded file nothing is working on and no queue entry owns, such as from hauler get or a removed download. Resumable ones in the queue are never listed."
        wide: true
        visible: tab.loaded && tab.storage.leftovers.count > 0
        Layout.fillWidth: true
        actions: [
            QQC2.Button {
                objectName: "clean-leftovers"
                text: "Delete them"
                enabled: !!tab.host && !tab.host.busy
                onClicked: tab.host.cleanLeftovers()
            }
        ]

        SettingRow {
            settingId: "leftovers"
            label: tab.loaded ? Logic.plural(tab.storage.leftovers.count, "unfinished download", "unfinished downloads") + ", " + Logic.bytesText(tab.storage.leftovers.bytes) : ""
            highlight: tab.host ? tab.host.highlight : ""
            Layout.fillWidth: true
        }
        Repeater {
            model: tab.loaded ? tab.storage.leftovers.files : []

            UqLabel {
                required property var modelData

                text: modelData.name + " · " + Logic.bytesText(modelData.bytes) + " · " + modelData.dir
                elide: Text.ElideMiddle
                opacity: 0.7
                Layout.fillWidth: true
            }
        }
        Outcome {
            host: tab.host
            noteKey: "leftovers"
            Layout.fillWidth: true
        }
    }
}
