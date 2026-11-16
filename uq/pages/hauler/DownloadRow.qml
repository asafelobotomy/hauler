// One download of the queue: icon, name, a progress bar (FlatBar), what it is doing, and pause or resume.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.uq.settings
import "logic.js" as Logic

ColumnLayout {
    id: row

    property var host
    /// A row from Logic.parseList.
    property var task
    readonly property bool going: row.task.state === "running" || row.task.state === "queued" || row.task.state === "waiting"

    spacing: 2

    RowLayout {
        spacing: 10
        Layout.fillWidth: true

        UqAppIcon {
            name: row.task.icon
            label: row.task.name
            size: 32
        }
        ColumnLayout {
            spacing: 2
            Layout.fillWidth: true

            UqLabel {
                objectName: "download-name"
                text: row.task.name
                elide: Text.ElideMiddle
                font.bold: true
                Layout.fillWidth: true
            }
            FlatBar {
                objectName: "download-bar"
                visible: row.task.state === "running" || row.task.percent >= 0
                indeterminate: row.task.state === "running" && row.task.percent < 0
                from: 0
                to: 100
                value: Math.max(0, row.task.percent)
                Layout.fillWidth: true
            }
            UqLabel {
                objectName: "download-line"
                text: Logic.queueLine(row.task) + (row.task.host !== "" ? " · " + row.task.host : "")
                opacity: 0.7
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
        }
        QQC2.Button {
            objectName: "pause-" + row.task.id
            text: "Pause"
            visible: row.going
            enabled: !!row.host && !row.host.busy
            onClicked: row.host.pauseOne(row.task.id)
        }
        QQC2.Button {
            objectName: "resume-" + row.task.id
            text: "Resume"
            visible: !row.going
            enabled: !!row.host && !row.host.busy
            onClicked: row.host.resumeOne(row.task.id)
        }
    }
    Outcome {
        host: row.host
        noteKey: "task:" + row.task.id
        Layout.fillWidth: true
    }
}
