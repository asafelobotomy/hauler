import QtQuick
import org.kde.kirigami as Kirigami

// A bar showing which byte ranges are done. Downloaded bytes use the accent
// colour; the rest of a segment a connection is working on is a lighter
// tint of it, with a bright edge where that connection is writing.
Canvas {
    id: canvas

    // { size, segments: [{ start, end, done, active }] }
    property var map: null

    readonly property color doneColor: Kirigami.Theme.highlightColor
    readonly property color activeColor: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.backgroundColor, Kirigami.Theme.highlightColor, 0.5)
    readonly property color edgeColor: Kirigami.Theme.textColor
    readonly property color trackColor: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.backgroundColor, Kirigami.Theme.textColor, 0.1)
    readonly property color gapColor: Kirigami.Theme.backgroundColor

    implicitHeight: Kirigami.Units.gridUnit * 1.5
    onMapChanged: requestPaint()
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
    onDoneColorChanged: requestPaint()
    onTrackColorChanged: requestPaint()

    onPaint: {
        const ctx = getContext("2d");
        ctx.reset();
        const r = Kirigami.Units.smallSpacing;
        ctx.beginPath();
        ctx.roundedRect(0, 0, width, height, r, r);
        ctx.clip();
        ctx.fillStyle = trackColor;
        ctx.fillRect(0, 0, width, height);
        if (!map || !map.size || map.size <= 0)
            return;
        const scale = width / map.size;
        for (const s of map.segments) {
            const x = s.start * scale;
            const doneW = Math.max(0, (s.done - s.start) * scale);
            if (s.active) {
                ctx.fillStyle = activeColor;
                ctx.fillRect(x, 0, (s.end - s.start) * scale, height);
            }
            ctx.fillStyle = doneColor;
            ctx.fillRect(x, 0, doneW, height);
            if (s.active && s.done < s.end) {
                ctx.fillStyle = edgeColor;
                ctx.fillRect(x + doneW - 1, 0, 2, height);
            }
            // A thin gap where each segment begins.
            if (s.start > 0) {
                ctx.fillStyle = gapColor;
                ctx.fillRect(x, 0, 1, height);
            }
        }
    }
}
