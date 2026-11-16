// What a read shows until it has answered: a bar, never a bare "Checking…". `problem` is why it failed.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.uq.settings

ColumnLayout {
    id: load

    /// True until the read has answered.
    property bool loading: false
    property string problem: ""
    property string what: "this"

    spacing: 4

    QQC2.ProgressBar {
        visible: load.loading
        indeterminate: true
        Layout.fillWidth: true
    }
    ErrorCard {
        title: "Couldn't read " + load.what
        detail: load.problem
        visible: !load.loading && load.problem !== ""
        Layout.fillWidth: true
    }
}
