import QtQuick
import org.kde.kirigami as Kirigami

// How a download's state reads and looks: a word, its colour, the progress
// bar's colour and a small emblem. Shared by the list and the details dialog.
// An Item rather than a QtObject so it picks up the surrounding theme.
Item {
    required property string status
    property real speed: 0
    property real received: 0

    // A running download that moves no bytes: still connecting, or stuck.
    readonly property bool stalled: status === "running" && speed <= 0

    readonly property string label: {
        switch (status) {
        case "running":
            return !stalled ? qsTr("Downloading") : received > 0 ? qsTr("Stalled") : qsTr("Connecting…");
        case "queued":
            return qsTr("Queued");
        case "paused":
            return qsTr("Paused");
        case "waiting":
            return qsTr("Waiting for network");
        case "completed":
            return qsTr("Finished");
        case "failed":
            return qsTr("Failed");
        }
        return "";
    }

    readonly property color color: {
        switch (status) {
        case "running":
            return stalled ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.linkColor;
        case "paused":
        case "waiting":
            return Kirigami.Theme.neutralTextColor;
        case "completed":
            return Kirigami.Theme.positiveTextColor;
        case "failed":
            return Kirigami.Theme.negativeTextColor;
        }
        return Kirigami.Theme.disabledTextColor;
    }

    readonly property color barColor: {
        switch (status) {
        case "running":
            return stalled ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.highlightColor;
        case "waiting":
            return Kirigami.Theme.neutralTextColor;
        case "failed":
            return Kirigami.Theme.negativeTextColor;
        }
        return Kirigami.Theme.disabledTextColor;
    }

    readonly property string emblem: {
        switch (status) {
        case "paused":
            return "emblem-pause";
        case "waiting":
            return "emblem-warning";
        case "completed":
            return "emblem-success";
        case "failed":
            return "emblem-error";
        }
        return "";
    }

    visible: false
}
