import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import org.hauler.desktop

// The last minute of download speed as a filled line, newest on the right.
Item {
    id: graph

    // Bytes per second, oldest first; one sample a second.
    property var samples: []
    property int capacity: 60

    readonly property real peak: samples.length > 0 ? Math.max(...samples) : 0

    implicitHeight: Kirigami.Units.gridUnit * 3

    function push(value) {
        const next = samples.concat([value]);
        samples = next.length > capacity ? next.slice(next.length - capacity) : next;
    }

    function clear() {
        samples = [];
    }

    Canvas {
        id: canvas

        readonly property color lineColor: Kirigami.Theme.highlightColor
        readonly property color fillColor: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.backgroundColor, Kirigami.Theme.highlightColor, 0.25)
        readonly property color gridColor: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.backgroundColor, Kirigami.Theme.textColor, 0.1)

        anchors.fill: parent
        onLineColorChanged: requestPaint()
        onGridColorChanged: requestPaint()
        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()

        Connections {
            target: graph

            function onSamplesChanged() {
                canvas.requestPaint();
            }
        }

        onPaint: {
            const ctx = getContext("2d");
            ctx.reset();
            const r = Kirigami.Units.smallSpacing;
            ctx.beginPath();
            ctx.roundedRect(0, 0, width, height, r, r);
            ctx.clip();
            // Guide lines at half and full scale.
            ctx.fillStyle = gridColor;
            ctx.fillRect(0, Math.round(height / 2), width, 1);
            ctx.fillRect(0, height - 1, width, 1);

            const n = graph.samples.length;
            if (n < 2 || graph.peak <= 0)
                return;
            const top = Kirigami.Units.smallSpacing;
            const step = width / (graph.capacity - 1);
            const x0 = width - (n - 1) * step;
            const y = v => height - (height - top) * v / graph.peak;

            ctx.beginPath();
            ctx.moveTo(x0, height);
            for (let i = 0; i < n; ++i)
                ctx.lineTo(x0 + i * step, y(graph.samples[i]));
            ctx.lineTo(width, height);
            ctx.closePath();
            ctx.fillStyle = fillColor;
            ctx.fill();

            ctx.beginPath();
            for (let i = 0; i < n; ++i) {
                if (i === 0)
                    ctx.moveTo(x0, y(graph.samples[0]));
                else
                    ctx.lineTo(x0 + i * step, y(graph.samples[i]));
            }
            ctx.strokeStyle = lineColor;
            ctx.lineWidth = 2;
            ctx.lineJoin = "round";
            ctx.stroke();
        }
    }

    QQC2.Label {
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.margins: Kirigami.Units.smallSpacing
        visible: graph.peak > 0
        text: qsTr("Peak %1").arg(Fmt.speed(graph.peak))
        font: Kirigami.Theme.smallFont
        color: Kirigami.Theme.disabledTextColor
    }
}
