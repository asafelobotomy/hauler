// What the page tests share: the Downloads page in a Loader, a fake ctx (FakeContext.qml) with an answer for every
// command (from fixtures/, which are what the real `hauler` commands print), and helpers that find items by their
// text or objectName and act on them the way a person does: a switch flips (its `toggled` signal), a box is chosen
// (`activated`), a button is pressed (`clicked`), a field is typed in (`editingFinished`). The fake ctx logs every
// request in `fake.calls`. A test file is `PageHarness { id: root; TestCase { id: tc; ... } Component.onCompleted: root.tc = tc }`.
import QtQuick
import QtTest

Item {
    id: root
    width: 900
    height: 700

    /// The TestCase of the file that uses this harness (its verify and tryVerify).
    property var tc
    default property alias tests: holder.data
    readonly property alias fakeCtx: fake
    readonly property alias pageLoader: loader
    Item {
        id: holder
    }

    readonly property url pageUrl: Qt.resolvedUrl("../../uq/pages/hauler/Page.qml")

    function fixture(name) {
        const x = new XMLHttpRequest();
        x.open("GET", Qt.resolvedUrl("fixtures/" + name), false);
        x.send();
        return JSON.parse(x.responseText);
    }

    function ok(output) {
        return { code: 0, output: output };
    }

    /// What `hauler ... --json` prints for a refusal: the exit code is 1.
    function refused(error, code) {
        return { code: 1, error: "exit code 1", output: { ok: false, error: error, code: code || "refused" } };
    }

    function down() {
        return { code: 1, error: "exit code 1", output: fixture("daemon-down.json") };
    }

    /// The recorded settings, with `changes` (key -> value) applied.
    function config(changes) {
        const doc = fixture("config.json");
        for (const k in (changes || {}))
            doc.settings[k] = changes[k];
        return ok(doc);
    }

    /// The recorded history, `edit` applied to the list of tasks.
    function history(edit) {
        const doc = fixture("history.json");
        if (edit)
            edit(doc.tasks);
        return ok(doc);
    }

    function storage(extra) {
        return ok(Object.assign(fixture("storage.json"), extra || {}));
    }

    function answers() {
        return {
            status: ok(fixture("status.json")),
            queue: ok(fixture("queue.json")),
            history: ok(fixture("history.json")),
            storage: ok(fixture("storage.json")),
            config: root.config(),
            "preview-config": args => args[0] === "history_days" ? ok(fixture("preview-config-retention.json")) : args[0] === "scrub_headers" ? ok(fixture("preview-config-scrub.json")) : args[0] === "https_only" ? ok(fixture("preview-config-https.json")) : ok(fixture("preview-config-refused.json")),
            "preview-remove": ok(fixture("preview-remove.json")),
            "preview-remove-files": ok(fixture("preview-remove-files.json")),
            "preview-clear": ok(fixture("preview-clear.json")),
            "preview-clear-files": ok(fixture("preview-clear-files.json")),
            "preview-forget-headers": ok(fixture("preview-forget-headers.json")),
            "preview-cleanup": ok(fixture("preview-cleanup.json")),
            "scan-one": ok(fixture("scan-one.json")),
            "scan-all": ok(fixture("scan-all.json")),
            "pause-all": { code: 0, output: "paused 1\n" },
            "resume-all": { code: 0, output: "resumed 2\n" },
            "pause-one": { code: 0, output: "5\tpaused\n" },
            "resume-one": { code: 0, output: "6\tqueued\n" },
            "add-url": { code: 0, output: "9\tqueued\thttp://127.0.0.1:8099/new.bin\n" },
            "config-set": ok(fixture("config-set.json")),
            "config-set-confirmed": ok(fixture("config-set.json")),
            "remove-one": ok(fixture("remove-one.json")),
            "remove-one-files": ok({ ok: true, removed: 1, deleted_files: 1, bytes: 3145728 }),
            "clear-history": ok(fixture("clear-history.json")),
            "clear-history-files": ok({ ok: true, removed: 4, deleted_files: 3, bytes: 9437184 }),
            "forget-headers": ok(fixture("forget-headers.json")),
            cleanup: ok(fixture("cleanup.json"))
        };
    }

    FakeContext {
        id: fake
        commands: root.answers()
        confirmed: ["config-set-confirmed", "remove-one", "remove-one-files", "clear-history", "clear-history-files", "forget-headers", "cleanup"]
    }

    Loader {
        id: loader
        anchors.fill: parent
    }

    function walk(item, visit) {
        visit(item);
        for (let i = 0; i < item.children.length; ++i)
            walk(item.children[i], visit);
    }

    /// The first visible item whose `text` (or `label`, `title`) is exactly `text`.
    function shown(text) {
        let hit = null;
        walk(loader.item, it => {
            if (!hit && it.visible && (it.text === text || it.label === text || it.title === text))
                hit = it;
        });
        return hit;
    }

    /// The first visible item whose `text` contains `part`.
    function showing(part) {
        let hit = null;
        walk(loader.item, it => {
            if (!hit && it.visible && typeof it.text === "string" && it.text.indexOf(part) >= 0)
                hit = it;
        });
        return hit;
    }

    function named(name) {
        let hit = null;
        walk(loader.item, it => {
            if (!hit && it.objectName === name && it.visible)
                hit = it;
        });
        return hit;
    }

    /// Every item with that objectName, shown or not.
    function allNamed(prefix) {
        const all = [];
        walk(loader.item, it => {
            if (typeof it.objectName === "string" && it.objectName.indexOf(prefix) === 0 && it.visible)
                all.push(it);
        });
        return all;
    }

    function everyText() {
        const all = [];
        walk(loader.item, it => {
            for (const k of ["text", "label", "title"])
                if (typeof it[k] === "string" && it[k] !== "")
                    all.push(it[k]);
        });
        return all;
    }

    function tab(title) {
        const button = root.named("tab-" + title);
        root.tc.verify(button !== null, "a tab called " + title);
        button.clicked();
    }

    function flip(sw) {
        sw.checked = !sw.checked;
        sw.toggled();
    }

    function choose(box, index) {
        box.currentIndex = index;
        box.activated(index);
    }

    function press(button) {
        root.tc.verify(button !== null, "a button to press");
        root.tc.verify(button.enabled, "the button is enabled");
        button.clicked();
    }

    function type(field, text) {
        field.text = text;
        field.editingFinished();
    }

    function runs(id) {
        return fake.calls.filter(c => c[0] === "run" && c[1] === id);
    }

    function open() {
        loader.source = "";
        loader.source = root.pageUrl;
        root.tc.verify(loader.status === Loader.Ready, "the page loads");
        loader.item.ctx = fake.ctx;
    }

    /// Start every test from a freshly opened page whose first reads have been answered.
    function reset() {
        fake.calls = [];
        fake.watches = ({});
        fake.confirms = true;
        fake.commands = root.answers();
        root.open();
        root.tc.tryVerify(() => loader.item.status !== null && loader.item.queue !== null && loader.item.history !== null && loader.item.storage !== null && loader.item.config !== null);
    }
}
