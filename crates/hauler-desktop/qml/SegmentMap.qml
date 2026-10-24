import QtQuick
import org.kde.kirigami as Kirigami

// A bar showing which byte ranges are done. Segments with a live connection
// use the highlight colour; finished and paused ranges use the positive one.
Canvas {
    id: canvas

    // { size, segments: [{ start, end, done, active }] }
    property var map: null

    implicitHeight: Kirigami.Units.gridUnit * 1.5
    onMapChanged: requestPaint()
    onWidthChanged: requestPaint()

    onPaint: {
        const ctx = getContext("2d");
        ctx.reset();
        ctx.fillStyle = Kirigami.Theme.alternateBackgroundColor;
        ctx.fillRect(0, 0, width, height);
        if (!map || !map.size || map.size <= 0)
            return;
        const scale = width / map.size;
        for (const s of map.segments) {
            const x = s.start * scale;
            const w = Math.max(0, (s.done - s.start) * scale);
            ctx.fillStyle = s.active ? Kirigami.Theme.highlightColor : Kirigami.Theme.positiveTextColor;
            ctx.fillRect(x, 0, w, height);
            // A thin mark where each segment begins.
            ctx.fillStyle = Kirigami.Theme.backgroundColor;
            ctx.fillRect(x, 0, 1, height);
        }
        ctx.strokeStyle = Kirigami.Theme.disabledTextColor;
        ctx.lineWidth = 1;
        ctx.strokeRect(0.5, 0.5, width - 1, height - 1);
    }
}
