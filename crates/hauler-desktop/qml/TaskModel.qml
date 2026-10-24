import QtQuick

// Downloads as the daemon reports them, newest first. Every role keeps one
// type (no nulls), which ListModel requires.
ListModel {
    id: model

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

    function indexOfId(id) {
        for (let i = 0; i < count; ++i) {
            if (get(i).taskId === id)
                return i;
        }
        return -1;
    }

    function reset(list) {
        clear();
        list.sort((a, b) => b.id - a.id).forEach(t => append(norm(t)));
    }

    function upsert(t) {
        const i = indexOfId(t.id);
        if (i < 0)
            insert(0, norm(t));
        else
            set(i, norm(t));
    }

    function removeId(id) {
        const i = indexOfId(id);
        if (i >= 0)
            remove(i);
    }

    function applyProgress(updates) {
        for (const u of updates) {
            const i = indexOfId(u.id);
            if (i < 0)
                continue;
            setProperty(i, "received", u.received);
            setProperty(i, "speed", u.speed);
            setProperty(i, "connections", u.connections);
        }
    }

    function countWhere(pred) {
        let n = 0;
        for (let i = 0; i < count; ++i) {
            if (pred(get(i)))
                n++;
        }
        return n;
    }

    function totalSpeed() {
        let s = 0;
        for (let i = 0; i < count; ++i) {
            const t = get(i);
            if (t.status === "running")
                s += t.speed;
        }
        return s;
    }
}
