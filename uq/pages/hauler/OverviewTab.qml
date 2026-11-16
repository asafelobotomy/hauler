// The Overview tab: what is downloading now, with pause and resume for all or one, and a field to add a link.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.uq.settings
import "logic.js" as Logic

SettingsTab {
    id: tab

    property var host
    readonly property var status: tab.host ? tab.host.status : null
    readonly property var queue: Logic.queueRows(tab.host ? tab.host.queue : null)
    readonly property bool queueOk: !!tab.host && tab.host.queue !== null && tab.host.queue.ok

    title: "Overview"
    iconName: "folder-download"

    SettingsCard {
        title: "Downloading now"
        iconName: "download"
        help: "Closing Hauler's window keeps downloads running. This follows them live."
        wide: true
        Layout.fillWidth: true
        actions: [
            QQC2.Button {
                objectName: "pause-all"
                text: "Pause all"
                enabled: !!tab.host && !tab.host.busy && tab.status !== null && tab.status.ok && tab.status.running + tab.status.queued > 0
                onClicked: tab.host.pauseAll()
            },
            QQC2.Button {
                objectName: "resume-all"
                text: "Resume all"
                enabled: !!tab.host && !tab.host.busy && tab.status !== null && tab.status.ok && tab.status.counts.paused + tab.status.counts.failed > 0
                onClicked: tab.host.resumeAll()
            }
        ]

        Loading {
            loading: tab.status === null
            problem: tab.status !== null && !tab.status.ok ? tab.status.problem : ""
            what: "Hauler's status"
            Layout.fillWidth: true
        }
        SettingRow {
            settingId: "now"
            label: tab.status !== null && tab.status.ok ? tab.status.line : ""
            highlight: tab.host ? tab.host.highlight : ""
            visible: tab.status !== null && tab.status.ok
            Layout.fillWidth: true
        }
        UqLabel {
            objectName: "network-line"
            text: tab.status !== null && tab.status.ok ? tab.status.networkLine : ""
            visible: text !== ""
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        Outcome {
            host: tab.host
            noteKey: "all"
            Layout.fillWidth: true
        }
    }
    SettingsCard {
        title: "Queue"
        iconName: "view-list-details"
        wide: true
        Layout.fillWidth: true

        Loading {
            loading: !!tab.host && tab.host.queue === null
            problem: tab.host && tab.host.queue !== null && !tab.host.queue.ok ? tab.host.queue.problem : ""
            what: "the queue"
            Layout.fillWidth: true
        }
        UqLabel {
            objectName: "queue-empty"
            text: "Nothing waiting or downloading. Add a link below, or send one from your browser."
            visible: tab.queueOk && tab.queue.count === 0
            opacity: 0.7
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        Repeater {
            model: tab.queue.shown

            DownloadRow {
                required property var modelData

                host: tab.host
                task: modelData
                Layout.fillWidth: true
            }
        }
        UqLabel {
            text: "And " + tab.queue.more + " more in Hauler."
            visible: tab.queue.more > 0
            opacity: 0.7
            Layout.fillWidth: true
        }
    }
    SettingsCard {
        title: "Add a link"
        iconName: "list-add"
        help: "Hauler checks the link and queues it. Several links, a folder and a speed limit are in Hauler's own window."
        Layout.fillWidth: true

        SettingRow {
            settingId: "add"
            label: "Link"
            highlight: tab.host ? tab.host.highlight : ""
            enabled: !!tab.host && !tab.host.busy
            Layout.fillWidth: true

            QQC2.TextField {
                id: link
                objectName: "link-field"
                placeholderText: "https://…"
                implicitWidth: 280
                onAccepted: add.clicked()
            }
            QQC2.Button {
                id: add
                objectName: "add-link"
                text: "Add"
                enabled: link.text.trim() !== ""
                onClicked: {
                    const why = Logic.linkProblem(link.text);
                    if (why !== "") {
                        tab.host.setNote("add", { message: why, failed: true });
                        return;
                    }
                    // The field empties once Hauler has taken the link; a refusal leaves it to be fixed.
                    tab.host.addLink(link.text, r => {
                        if (Logic.envelope(r).ok)
                            link.text = "";
                    });
                }
            }
        }
        Outcome {
            host: tab.host
            noteKey: "add"
            Layout.fillWidth: true
        }
    }
}
