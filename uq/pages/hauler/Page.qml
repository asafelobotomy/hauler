// Hauler's "Downloads" page in uQ Settings, laid out like Settings' own pages (uq_api = 3): five tabs of cards.
// Everything runs through the commands declared in page.toml (`hauler`, as you, over haulerd's socket) and is
// followed live through haulerd's D-Bus signals, so the page shows what Hauler's own window and the browser do.
// All text goes through org.uq.settings components (plain text). Parsing and the rules for what may be sent live
// in logic.js; the tabs are the *Tab.qml files beside this one.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.uq.settings
import "logic.js" as Logic

SettingsPage {
    id: page

    property var ctx
    property var theme
    /// The [[setting]] row Settings search jumped to; the rows flash, and their tab opens.
    property string highlight

    // What hauler last printed, parsed (Logic.parse*). null until the read has answered.
    property var status: null
    property var queue: null
    property var history: null
    property var storage: null
    property var config: null
    /// Counts the times the settings were read again after a write: a field then shows what is saved.
    property int configRev: 0

    /// What each write said, by row key: {message, failed}. Shown under the row.
    property var notes: ({})
    /// The key of the write that is running ("" = none). One write at a time.
    property string workingKey: ""
    readonly property bool busy: page.workingKey !== ""
    readonly property bool daemonDown: [page.status, page.queue, page.history, page.storage, page.config].some(r => r !== null && r.ok === false && r.daemonDown === true)

    property bool alive: true
    property var openedWith: null
    property string openProblem: ""

    pageId: "hauler"

    function colorOf(name, fallback) {
        const c = page.ctx && page.ctx.theme ? page.ctx.theme.colors : null;
        return (c && c[name]) || fallback;
    }

    function errorColor() {
        return page.colorOf("error", "red");
    }

    function mutedColor() {
        return page.colorOf("text_muted", "gray");
    }

    function noteFor(key) {
        return page.notes[key] || null;
    }

    function setNote(key, note) {
        page.notes = Logic.withNote(page.notes, key, note);
    }

    /// Wraps a callback so an answer that arrives after the page is gone is ignored.
    function guard(callback) {
        return result => {
            if (page.alive)
                callback(result);
        };
    }

    /// The saved value of a setting, or undefined until the settings have been read.
    function setting(key) {
        return page.config !== null && page.config.ok ? page.config.values[key] : undefined;
    }

    function load(name, command, parse) {
        if (!page.ctx)
            return;
        page.ctx.command.run(command, [], page.guard(result => {
            page[name] = parse(result);
            if (name === "config")
                page.configRev++;
        }));
    }

    function reloadAll() {
        page.load("status", "status", Logic.parseStatus);
        page.load("queue", "queue", r => Logic.parseList(r, Date.now()));
        page.load("history", "history", r => Logic.parseList(r, Date.now()));
        page.load("storage", "storage", Logic.parseStorage);
        page.load("config", "config", Logic.parseConfig);
    }

    /// Runs one write (a command that changes something) for the row `key`, then `done(result)` unless the person
    /// cancelled. A cancel changes nothing and shows nothing. `doing` words a success (text, or a function of the
    /// result). Everything is read again afterwards.
    function write(key, command, args, done, doing) {
        if (page.busy || !page.ctx)
            return false;
        const why = Logic.argsProblem(command, args);
        if (why !== "") {
            page.setNote(key, { message: why, failed: true });
            return false;
        }
        page.setNote(key, null);
        page.workingKey = key;
        page.ctx.command.run(command, args, page.guard(result => {
            page.workingKey = "";
            if (Logic.isCancelled(result))
                return;
            page.setNote(key, Logic.outcome(result, typeof doing === "function" ? doing(result) : doing));
            page.reloadAll();
            if (done)
                done(result);
        }));
        return true;
    }

    /// Saves one setting; Hauler checks the value and says why it refuses. With `previewFirst`, a change that
    /// would remove something (old history, saved logins) or break something waiting asks first, with the plan; a
    /// change that touches nothing is saved at once.
    function setConfig(key, value, previewFirst) {
        const noteKey = "cfg:" + key;
        if (page.busy || !page.ctx)
            return false;
        const why = Logic.argsProblem("config-set", [key, String(value)]);
        if (why !== "") {
            page.setNote(noteKey, { message: why, failed: true });
            return false;
        }
        if (!previewFirst)
            return page.write(noteKey, "config-set", [key, String(value)], null, "Saved.");
        page.setNote(noteKey, null);
        page.workingKey = noteKey;
        page.ctx.command.run("preview-config", [key, String(value)], page.guard(result => {
            page.workingKey = "";
            const plan = Logic.parsePlan(result);
            if (!plan.ok || plan.refused) {
                page.setNote(noteKey, { message: plan.ok ? plan.reason : plan.problem, failed: true });
                return;
            }
            page.write(noteKey, plan.changes > 0 ? "config-set-confirmed" : "config-set", [key, String(value)], null, "Saved.");
        }));
        return true;
    }

    function pauseAll() {
        page.write("all", "pause-all", [], null, "Paused everything that was running or waiting.");
    }

    function resumeAll() {
        page.write("all", "resume-all", [], null, "Resumed everything that was paused or had failed.");
    }

    function pauseOne(id) {
        page.write("task:" + id, "pause-one", [id], null, "Paused.");
    }

    function resumeOne(id) {
        page.write("task:" + id, "resume-one", [id], null, "Queued again.");
    }

    function addLink(url, done) {
        return page.write("add", "add-url", [url.trim()], done, "Added to the queue.");
    }

    function removeOne(id, withFile) {
        page.write("row:" + id, withFile ? "remove-one-files" : "remove-one", [id], null, Logic.removedLine);
    }

    function clearHistory(state, withFiles) {
        page.write("clear", withFiles ? "clear-history-files" : "clear-history", [state], null, Logic.removedLine);
    }

    function forgetLogins() {
        page.write("forget", "forget-headers", [], null, Logic.forgottenLine);
    }

    function scanOne(id) {
        page.write("row:" + id, "scan-one", [id], null, "Asked oysterAV to scan it. The answer shows here.");
    }

    function scanAll() {
        page.write("scan", "scan-all", [], null, r => Logic.scanStartedLine(r));
    }

    function cleanLeftovers() {
        page.write("leftovers", "cleanup", [], null, Logic.cleanedLine);
    }

    function openApp() {
        page.openProblem = "";
        page.ctx.openApp(Logic.APP_ID, page.guard(result => {
            if (result && result.error !== undefined)
                page.openProblem = "Couldn't open Hauler.";
        }));
    }

    onCtxChanged: {
        if (page.ctx && page.ctx !== page.openedWith) {
            page.openedWith = page.ctx;
            page.ctx.command.watch("status", page.guard(r => {
                page.status = Logic.parseStatus(r);
            }));
            page.ctx.command.watch("queue", page.guard(r => {
                page.queue = Logic.parseList(r, Date.now());
            }));
            page.ctx.command.watch("history", page.guard(r => {
                page.history = Logic.parseList(r, Date.now());
            }));
            page.ctx.command.watch("storage", page.guard(r => {
                page.storage = Logic.parseStorage(r);
            }));
            page.ctx.command.watch("config", page.guard(r => {
                page.config = Logic.parseConfig(r);
                page.configRev++;
            }));
        }
    }
    Component.onDestruction: page.alive = false

    headerItems: [
        QQC2.Button {
            objectName: "open-hauler"
            text: "Open Hauler"
            icon.name: "folder-download"
            onClicked: page.openApp()
        }
    ]

    readonly property var scanTotals: Logic.scanTotals(page.history)

    banners: [
        ErrorCard {
            objectName: "scan-threats"
            visible: page.scanTotals.threats > 0
            title: "oysterAV found something in " + Logic.plural(page.scanTotals.threats, "download", "downloads")
            detail: "Don't open them. They are marked in History; oysterAV can move them away."
            Layout.fillWidth: true
        },
        ErrorCard {
            objectName: "daemon-down"
            visible: page.daemonDown
            title: "Hauler's background service couldn't be started"
            detail: "This page starts it by itself when it opens, and that didn't work. Start it with systemctl --user start hauler.socket, or open Hauler; this page fills in once it answers."
            Layout.fillWidth: true
        },
        ErrorCard {
            visible: page.openProblem !== ""
            title: page.openProblem
            detail: "Start Hauler from your app launcher."
            Layout.fillWidth: true
        }
    ]

    OverviewTab {
        host: page
    }

    HistoryTab {
        host: page
    }

    StorageTab {
        host: page
    }

    SecurityTab {
        host: page
    }

    PreferencesTab {
        host: page
    }
}
