// One finished download: icon, name, where it came from and when, and what can be done with it.
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
                objectName: "history-name"
                text: row.task.name
                elide: Text.ElideMiddle
                font.bold: true
                Layout.fillWidth: true
            }
            UqLabel {
                objectName: "history-line"
                text: Logic.rowLine(row.task)
                opacity: 0.7
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
            UqLabel {
                id: problem
                objectName: "history-problem"
                text: row.task.state === "failed" ? row.task.error : "The file is no longer in " + row.task.dir + "."
                visible: text !== "" && (row.task.state === "failed" || !row.task.fileExists)
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
            Binding {
                target: problem
                property: "color"
                value: row.host ? row.host.errorColor() : "red"
                when: row.task.state === "failed"
            }
        }
        QQC2.Button {
            objectName: "remove-" + row.task.id
            text: "Remove"
            enabled: !!row.host && !row.host.busy
            onClicked: row.host.removeOne(row.task.id, false)
        }
        QQC2.Button {
            objectName: "delete-" + row.task.id
            text: "Delete file"
            visible: row.task.state === "completed" && row.task.fileExists
            enabled: !!row.host && !row.host.busy
            onClicked: row.host.removeOne(row.task.id, true)
        }
    }
    Outcome {
        host: row.host
        noteKey: "row:" + row.task.id
        Layout.fillWidth: true
    }
}
