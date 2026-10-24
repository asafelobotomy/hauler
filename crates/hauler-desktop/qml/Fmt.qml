pragma Singleton

import QtQuick

// Number formatting shared by the views.
QtObject {
    function bytes(n) {
        if (n === undefined || n === null || n < 0)
            return "?";
        const units = ["B", "KiB", "MiB", "GiB", "TiB"];
        let i = 0;
        let v = n;
        while (v >= 1024 && i < units.length - 1) {
            v /= 1024;
            i++;
        }
        return (i === 0 ? v.toFixed(0) : v.toFixed(v < 10 ? 2 : 1)) + " " + units[i];
    }

    function speed(n) {
        return bytes(n) + "/s";
    }

    function duration(seconds) {
        if (!isFinite(seconds) || seconds < 0)
            return "";
        seconds = Math.round(seconds);
        if (seconds < 60)
            return qsTr("%1 s").arg(seconds);
        if (seconds < 3600)
            return qsTr("%1 min %2 s").arg(Math.floor(seconds / 60)).arg(seconds % 60);
        return qsTr("%1 h %2 min").arg(Math.floor(seconds / 3600)).arg(Math.floor(seconds % 3600 / 60));
    }

    // Shorter, for the list: whole minutes once past a minute.
    function eta(seconds) {
        if (!isFinite(seconds) || seconds < 0)
            return "";
        seconds = Math.round(seconds);
        if (seconds < 60)
            return qsTr("%1 s").arg(seconds);
        if (seconds < 3600)
            return qsTr("%1 min").arg(Math.round(seconds / 60));
        return qsTr("%1 h %2 min").arg(Math.floor(seconds / 3600)).arg(Math.floor(seconds % 3600 / 60));
    }

    function percent(fraction) {
        return qsTr("%1%").arg(Math.floor(Math.max(0, Math.min(1, fraction)) * 100));
    }

    function host(url) {
        const m = /^[a-z]+:\/\/([^\/?#]+)/i.exec(url);
        return m ? m[1] : url;
    }

    // A file:// URL for a local path, for Qt.openUrlExternally.
    function fileUrl(path) {
        return "file://" + path.split("/").map(encodeURIComponent).join("/");
    }
}
