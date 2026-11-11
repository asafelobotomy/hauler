// hauler's downloads widget for uQ's bar (docs/integrating.md in the uQ repository).
// The icon: a download arrow, with a progress ring around it while anything downloads (a
// spinning arc while the total size is unknown). Clicking it opens the queue.
// Every string goes through ctx.UqText as plain text. Everything else goes through ctx.dbus,
// which reaches only what widget.toml lists on org.hauler.Hauler1.
import QtQuick

Item {
    id: widget

    property var ctx
    property int active: 0
    property int queued: 0
    property real bytesDone: 0
    property real bytesTotal: 0
    // Unfinished downloads from Queue(), in order: [{id, name, state, received, size, speed}].
    property var queue: []
    property string problem: ""

    readonly property bool busy: active > 0 || queued > 0
    // 0-1, or -1 while the total size is unknown.
    readonly property real fraction: bytesTotal > 0 ? Math.min(1, bytesDone / bytesTotal) : -1

    function apply(p) {
        if (!p)
            return;
        if (p.ActiveCount !== undefined)
            active = Number(p.ActiveCount);
        if (p.QueuedCount !== undefined)
            queued = Number(p.QueuedCount);
        if (p.BytesDone !== undefined)
            bytesDone = Number(p.BytesDone);
        if (p.BytesTotal !== undefined)
            bytesTotal = Number(p.BytesTotal);
    }

    function refreshQueue() {
        ctx.dbus.call("Queue", [], function (r) {
            if (r.error) {
                widget.problem = String(r.error);
                return;
            }
            widget.problem = "";
            var map = r.value || {};
            widget.queue = Object.keys(map).sort().map(function (k) { return map[k]; });
        });
    }

    function callAll(method) {
        ctx.dbus.call(method, [], function (r) {
            widget.problem = r.error ? String(r.error) : "";
            widget.refreshQueue();
        });
    }

    function sizeText(bytes) {
        var units = ["B", "KiB", "MiB", "GiB", "TiB"];
        var n = Number(bytes), i = 0;
        while (n >= 1024 && i < units.length - 1) {
            n /= 1024;
            i++;
        }
        return (i === 0 ? n.toFixed(0) : n.toFixed(1)) + " " + units[i];
    }

    function entryLine(e) {
        var size = Number(e.size), got = Number(e.received);
        var parts = [String(e.state)];
        if (size > 0)
            parts.unshift(Math.floor(got * 100 / size) + "%");
        if (e.state === "running" && Number(e.speed) > 0)
            parts.push(sizeText(e.speed) + "/s");
        return parts.join(" · ");
    }

    Component.onCompleted: {
        ctx.dbus.get(function (r) {
            if (r.error)
                widget.problem = String(r.error);
            else
                widget.apply(r.value);
        });
        ctx.dbus.onProps(function (changed) { widget.apply(changed); });
        ctx.dbus.onSignal(function (member, args) {
            if (member === "Finished")
                widget.refreshQueue();
        });
    }

    onBusyChanged: ring.requestPaint()
    onFractionChanged: ring.requestPaint()

    Rectangle {
        anchors.fill: parent
        radius: widget.ctx ? Math.max(0, widget.ctx.Theme.radius - 2) : 4
        color: mouse.containsMouse && widget.ctx ? widget.ctx.Theme.overlay : "transparent"
    }

    // The ring, and the arrow inside it.
    Canvas {
        id: ring
        anchors.centerIn: parent
        width: 20
        height: 20
        property real spin

        onPaint: {
            var g = getContext("2d");
            g.reset();
            if (!widget.ctx)
                return;
            var t = widget.ctx.Theme, c = width / 2, r = c - 1.5;
            g.lineWidth = 2;
            g.lineCap = "round";
            if (widget.busy) {
                g.strokeStyle = String(t.overlay);
                g.beginPath();
                g.arc(c, c, r, 0, 2 * Math.PI);
                g.stroke();
                g.strokeStyle = String(t.accent);
                g.beginPath();
                if (widget.fraction >= 0) {
                    var start = -Math.PI / 2;
                    g.arc(c, c, r, start, start + 2 * Math.PI * Math.max(0.02, widget.fraction));
                } else {
                    g.arc(c, c, r, spin, spin + Math.PI / 2);
                }
                g.stroke();
            }
            // The arrow: a stem and a head over a tray.
            g.strokeStyle = String(widget.busy ? t.accent : t.text);
            g.lineWidth = 1.5;
            g.beginPath();
            g.moveTo(c, c - 5);
            g.lineTo(c, c + 2);
            g.moveTo(c - 3.5, c - 1.5);
            g.lineTo(c, c + 2);
            g.lineTo(c + 3.5, c - 1.5);
            g.moveTo(c - 4.5, c + 5);
            g.lineTo(c + 4.5, c + 5);
            g.stroke();
        }

        NumberAnimation on spin {
            running: widget.busy && widget.fraction < 0
            from: 0
            to: 2 * Math.PI
            duration: 1200
            loops: Animation.Infinite
        }
        onSpinChanged: requestPaint()
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        onClicked: {
            widget.refreshQueue();
            widget.ctx.openPopup();
        }
    }

    // The queue, under the bar icon.
    property Component popup: Component {
        Item {
            id: pop
            property var ctx
            implicitWidth: 300
            implicitHeight: column.implicitHeight + 16

            Timer {
                interval: 1000
                running: true
                repeat: true
                onTriggered: widget.refreshQueue()
            }

            Column {
                id: column
                x: 8
                y: 8
                width: pop.width - 16
                spacing: 6

                Loader {
                    width: column.width
                    sourceComponent: pop.ctx ? pop.ctx.UqText : null
                    onLoaded: {
                        item.width = Qt.binding(function () { return column.width; });
                        item.font.bold = true;
                        item.text = Qt.binding(function () {
                            if (!widget.busy)
                                return "Downloads";
                            var s = widget.active + " running";
                            if (widget.queued > 0)
                                s += ", " + widget.queued + " queued";
                            if (widget.bytesTotal > 0)
                                s += " · " + widget.sizeText(widget.bytesDone) + " of "
                                    + widget.sizeText(widget.bytesTotal);
                            return s;
                        });
                    }
                }

                Loader {
                    width: column.width
                    active: widget.queue.length === 0 || widget.problem !== ""
                    visible: active
                    sourceComponent: pop.ctx ? pop.ctx.UqText : null
                    onLoaded: {
                        item.width = Qt.binding(function () { return column.width; });
                        item.color = Qt.binding(function () { return pop.ctx.Theme.textMuted; });
                        item.text = Qt.binding(function () {
                            return widget.problem !== "" ? widget.problem : "Nothing to download";
                        });
                    }
                }

                Repeater {
                    model: widget.queue
                    delegate: Column {
                        id: row
                        required property var modelData
                        width: column.width
                        spacing: 2

                        Loader {
                            width: row.width
                            sourceComponent: pop.ctx ? pop.ctx.UqText : null
                            onLoaded: {
                                item.width = Qt.binding(function () { return row.width; });
                                item.text = Qt.binding(function () { return String(row.modelData.name); });
                            }
                        }
                        Rectangle {
                            width: row.width
                            height: 3
                            radius: 1.5
                            color: pop.ctx ? pop.ctx.Theme.overlay : "gray"
                            Rectangle {
                                readonly property real size: Number(row.modelData.size)
                                width: size > 0 ? parent.width * Math.min(1, Number(row.modelData.received) / size) : 0
                                height: parent.height
                                radius: parent.radius
                                color: pop.ctx ? (row.modelData.state === "failed" ? pop.ctx.Theme.error
                                                                                     : pop.ctx.Theme.accent)
                                               : "white"
                            }
                        }
                        Loader {
                            width: row.width
                            sourceComponent: pop.ctx ? pop.ctx.UqText : null
                            onLoaded: {
                                item.width = Qt.binding(function () { return row.width; });
                                item.color = Qt.binding(function () { return pop.ctx.Theme.textMuted; });
                                item.text = Qt.binding(function () { return widget.entryLine(row.modelData); });
                            }
                        }
                    }
                }

                Row {
                    spacing: 6
                    Repeater {
                        model: [
                            { label: "Pause all", method: "PauseAll" },
                            { label: "Resume all", method: "ResumeAll" }
                        ]
                        delegate: Rectangle {
                            id: button
                            required property var modelData
                            width: caption.width + 16
                            height: 24
                            radius: pop.ctx ? Math.max(0, pop.ctx.Theme.radius - 4) : 4
                            color: pop.ctx ? (press.containsMouse ? pop.ctx.Theme.overlay : pop.ctx.Theme.surfaceAlt)
                                           : "gray"
                            Loader {
                                id: caption
                                anchors.centerIn: parent
                                sourceComponent: pop.ctx ? pop.ctx.UqText : null
                                onLoaded: item.text = button.modelData.label
                            }
                            MouseArea {
                                id: press
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: widget.callAll(button.modelData.method)
                            }
                        }
                    }
                }
            }
        }
    }
}
