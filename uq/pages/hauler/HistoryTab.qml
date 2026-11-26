// The History tab: finished downloads, newest first, with a plan before anything is removed; clearing them in
// one go; and how long Hauler keeps them.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.uq.settings
import "logic.js" as Logic
import "edit.js" as Edit

SettingsTab {
    id: tab

    property var host
    property int filterIndex: 0
    readonly property string filter: Logic.FILTERS[tab.filterIndex].id
    readonly property var rows: Logic.historyRows(tab.host ? tab.host.history : null, tab.filter)
    readonly property bool loaded: !!tab.host && tab.host.history !== null && tab.host.history.ok
    readonly property var days: Edit.retentionChoices(tab.host ? tab.host.setting("history_days") : undefined)

    title: "History"
    iconName: "view-history"

    SettingsCard {
        title: "Finished downloads"
        iconName: "folder-download"
        help: "Removing a download from the list never touches its file, unless you choose Delete file."
        wide: true
        Layout.fillWidth: true
        actions: [
            UqComboBox {
                id: filterBox
                objectName: "history-filter"
                model: Logic.FILTERS.map(f => f.label)
                currentIndex: tab.filterIndex
                onActivated: index => tab.filterIndex = index
            }
        ]

        Loading {
            loading: !!tab.host && tab.host.history === null
            problem: tab.host && tab.host.history !== null && !tab.host.history.ok ? tab.host.history.problem : ""
            what: "the history"
            Layout.fillWidth: true
        }
        SettingRow {
            settingId: "history"
            label: tab.loaded ? Logic.historyLine(tab.rows) : ""
            highlight: tab.host ? tab.host.highlight : ""
            visible: tab.loaded
            Layout.fillWidth: true
        }
        Repeater {
            model: tab.rows.shown

            HistoryRow {
                required property var modelData

                host: tab.host
                task: modelData
                Layout.fillWidth: true
            }
        }
        UqLabel {
            text: "And " + tab.rows.more + " older ones. Clear some to see them."
            visible: tab.rows.more > 0
            opacity: 0.7
            Layout.fillWidth: true
        }
    }
    SettingsCard {
        title: "Clear the list"
        iconName: "edit-clear-history"
        help: "Shows exactly what goes before anything does."
        Layout.fillWidth: true

        QQC2.CheckBox {
            id: withFiles
            objectName: "clear-with-files"
            text: "Also delete the downloaded files"
        }
        RowLayout {
            spacing: 8
            Layout.fillWidth: true
            enabled: !!tab.host && !tab.host.busy

            QQC2.Button {
                objectName: "clear-completed"
                text: "Clear completed"
                onClicked: tab.host.clearHistory("completed", withFiles.checked)
            }
            QQC2.Button {
                objectName: "clear-failed"
                text: "Clear failed"
                onClicked: tab.host.clearHistory("failed", withFiles.checked)
            }
            QQC2.Button {
                objectName: "clear-all"
                text: "Clear all finished"
                onClicked: tab.host.clearHistory("finished", withFiles.checked)
            }
        }
        Outcome {
            host: tab.host
            noteKey: "clear"
            Layout.fillWidth: true
        }
    }
    // After a save (accepted or refused) the box shows what Hauler holds, not what was chosen.
    Connections {
        target: tab.host
        function onConfigRevChanged() {
            retention.currentIndex = Qt.binding(() => tab.days.index);
        }
    }
    SettingsCard {
        title: "Keep history for"
        iconName: "chronometer"
        help: "Older finished downloads leave the list by themselves. Their files stay."
        Layout.fillWidth: true

        SettingRow {
            settingId: "retention"
            label: "Forget finished downloads after"
            highlight: tab.host ? tab.host.highlight : ""
            enabled: !!tab.host && !tab.host.busy && tab.host.config !== null && tab.host.config.ok
            Layout.fillWidth: true

            UqComboBox {
                id: retention
                objectName: "retention"
                model: tab.days.labels
                currentIndex: tab.days.index
                onActivated: index => {
                    const days = tab.days.values[index];
                    tab.host.setConfig("history_days", days, days > 0);
                }
            }
        }
        Outcome {
            host: tab.host
            noteKey: "cfg:history_days"
            Layout.fillWidth: true
        }
    }
}
