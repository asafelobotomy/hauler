import QtQuick

// Downloads as the daemon reports them, newest first. Every role keeps one
// type (no nulls), which ListModel requires.
ListModel {
    id: model

    // Totals kept current by the functions below, so views never rescan the
    // whole model on every progress tick.
    property int runningCount: 0
    property real totalSpeed: 0
    // Bumped once per batch of changes, for bindings that re-read rows.
    property int changes: 0
    // taskId -> row. Rebuilt only when rows move (add, remove, reset).
    property var rows: ({})

    function norm(t) {
        return {
            taskId: t.id,
            url: t.url,
            dir: t.dir,
            fileName: t.name ?? "",
            status: t.state,
            size: t.size ?? -1,
            received: t.received,
            speed: t.speed,
            connections: t.connections,
            limit: t.limit,
            priority: t.priority,
            resumable: t.resumable !== false,
            error: t.error ?? ""
        };
    }

    function reindex() {
        const r = {};
        for (let i = 0; i < count; ++i)
            r[get(i).taskId] = i;
        rows = r;
    }

    function recount() {
        let running = 0;
        let speed = 0;
        for (let i = 0; i < count; ++i) {
            const t = get(i);
            if (t.status === "running") {
                running++;
                speed += t.speed;
            }
        }
        runningCount = running;
        totalSpeed = speed;
    }

    function indexOfId(id) {
        const i = rows[id];
        return i === undefined ? -1 : i;
    }

    function reset(list) {
        clear();
        list.sort((a, b) => b.id - a.id).forEach(t => append(norm(t)));
        reindex();
        recount();
        changes++;
    }

    function upsert(t) {
        const i = indexOfId(t.id);
        if (i < 0) {
            insert(0, norm(t));
            reindex();
        } else {
            set(i, norm(t));
        }
        recount();
        changes++;
    }

    function removeId(id) {
        const i = indexOfId(id);
        if (i < 0)
            return;
        remove(i);
        reindex();
        recount();
        changes++;
    }

    // Called about once a second with the running downloads only: O(updates).
    function applyProgress(updates) {
        let speed = totalSpeed;
        for (const u of updates) {
            const i = indexOfId(u.id);
            if (i < 0)
                continue;
            const t = get(i);
            if (t.status === "running")
                speed += u.speed - t.speed;
            setProperty(i, "received", u.received);
            setProperty(i, "speed", u.speed);
            setProperty(i, "connections", u.connections);
        }
        totalSpeed = speed;
        changes++;
    }
}
