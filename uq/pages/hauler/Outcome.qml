// What an action said, under its row: "Working…" while it runs, then Hauler's own answer in plain
// words. A failure is shown in the error colour.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.uq.settings
import "logic.js" as Logic

ColumnLayout {
    id: out

    /// The page (Page.qml): it holds the notes and which write is running.
    property var host
    /// The row's key in the page's notes.
    property string noteKey
    /// Show an indeterminate bar while this write runs (a long one).
    property bool bar: false
    readonly property var note: out.host ? out.host.noteFor(out.noteKey) : null
    readonly property bool working: !!out.host && out.host.workingKey === out.noteKey

    visible: out.working || out.note !== null
    spacing: 2

    QQC2.ProgressBar {
        visible: out.working && out.bar
        indeterminate: true
        Layout.fillWidth: true
    }
    UqLabel {
        id: message
        text: out.working ? Logic.WORKING : (out.note ? out.note.message : "")
        visible: text !== ""
        wrapMode: Text.Wrap
        Layout.fillWidth: true
    }
    Binding {
        target: message
        property: "color"
        value: out.host ? out.host.errorColor() : "red"
        when: !out.working && out.note !== null && out.note.failed
    }
}
