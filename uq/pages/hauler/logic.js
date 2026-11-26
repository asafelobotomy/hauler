.pragma library
// What the Downloads page knows that isn't layout: reading what `hauler --json` prints, wording what it
// says, and checking what the page may send. No Qt calls, so Node can test it (tests/uq/logic.test.mjs).
// Names and addresses come from servers: they are only ever shown as plain text and cut to length.

var MAX_ROWS = 100;
var MAX_TEXT = 300;
var WORKING = "Working…";
var APP_ID = "hauler.desktop";

var SETTING_KEYS = "^(max_active|connections|global_limit|download_dir|pause_on_metered|history_days|scrub_headers|https_only|min_free|scan_downloads)$";
var ID = "^[1-9][0-9]{0,17}$";
var STATE = "^(finished|completed|failed)$";
var URL_PATTERN = "^https?://[^\\x00-\\x20\\x7f]{1,1000}$";
// The patterns of page.toml, so a bad value is refused here with a sentence instead of by uQ with a code.
// tests/uq/logic.test.mjs checks they are the same.
var PATTERNS = {
    "config-set": [SETTING_KEYS, "^[^\\x00-\\x1f\\x7f]{0,600}$"],
    "config-set-confirmed": [SETTING_KEYS, "^[^\\x00-\\x1f\\x7f]{0,600}$"],
    "preview-config": [SETTING_KEYS, "^[^\\x00-\\x1f\\x7f]{0,600}$"],
    "add-url": [URL_PATTERN],
    "pause-one": [ID],
    "resume-one": [ID],
    "scan-one": [ID],
    "remove-one": [ID],
    "remove-one-files": [ID],
    "preview-remove": [ID],
    "preview-remove-files": [ID],
    "clear-history": [STATE],
    "clear-history-files": [STATE],
    "preview-clear": [STATE],
    "preview-clear-files": [STATE]
};

// Settings of the Preferences tab, in order. `kind`: int, size (bytes, "5M"), bool.
var PREFS = [
    { key: "max_active", label: "Downloads at once", kind: "int", min: 1, max: 0, help: "More wait in the queue and start as others finish." },
    { key: "connections", label: "Connections per download", kind: "int", min: 1, max: 32, help: "Used when a download doesn't say. More is faster on slow servers, and some refuse many." },
    { key: "global_limit", label: "Speed limit for all downloads", kind: "size", min: 0, max: 0, help: "Shared by every download, like 2M for 2 MiB a second. 0 means no limit." },
    { key: "pause_on_metered", label: "Pause on a metered connection", kind: "bool", help: "Holds downloads on a connection NetworkManager calls metered, and resumes them by themselves." }
];

var RETENTION = [
    { days: 0, label: "For ever" },
    { days: 7, label: "7 days" },
    { days: 30, label: "30 days" },
    { days: 90, label: "90 days" },
    { days: 365, label: "A year" }
];
var RESERVE = [
    { bytes: 0, label: "Off" },
    { bytes: 1073741824, label: "1 GiB" },
    { bytes: 5368709120, label: "5 GiB" },
    { bytes: 10737418240, label: "10 GiB" },
    { bytes: 53687091200, label: "50 GiB" }
];
var FILTERS = [
    { id: "finished", label: "All finished" },
    { id: "completed", label: "Completed" },
    { id: "failed", label: "Failed" }
];

function clip(value) {
    var text = String(value === undefined || value === null ? "" : value);
    return text.length > MAX_TEXT ? text.slice(0, MAX_TEXT - 1) + "…" : text;
}

function isObject(v) {
    return v !== null && typeof v === "object" && !Array.isArray(v);
}

function plural(n, one, many) {
    return n + " " + (n === 1 ? one : many);
}

function isCancelled(result) {
    return !!result && result.cancelled === true;
}

/// Why `args` can't be sent to `command` ("" when they can), by the patterns of page.toml.
function argsProblem(command, args) {
    var patterns = PATTERNS[command];
    if (!patterns)
        return args.length === 0 ? "" : "That doesn't take arguments.";
    if (args.length !== patterns.length)
        return "That needs " + patterns.length + " value(s).";
    for (var i = 0; i < patterns.length; ++i) {
        var text = String(args[i]);
        if (text.charAt(0) === "-" || text.indexOf("\0") >= 0 || !new RegExp(patterns[i]).test(text))
            return "That value isn't allowed here.";
    }
    return "";
}

/// What a command answered, whatever its exit code: {ok, cancelled, error, code, output}. `hauler --json` prints
/// {ok: false, error, code} for a refusal or a failure, so its words are kept; `code` is "daemon_down" when
/// haulerd can't be reached.
function envelope(result) {
    if (!isObject(result))
        return { ok: false, cancelled: false, error: "No answer.", code: "error", output: null };
    if (result.cancelled === true)
        return { ok: false, cancelled: true, error: "Cancelled", code: "cancelled", output: null };
    var out = result.output === undefined ? null : result.output;
    if (isObject(out) && out.ok === false)
        return { ok: false, cancelled: false, error: clip(out.error || result.error || "Hauler refused."), code: String(out.code || "error"), output: out };
    if (result.error)
        return { ok: false, cancelled: false, error: clip(result.error), code: "error", output: out };
    return { ok: true, cancelled: false, error: "", code: "", output: out };
}

function outcome(result, doing) {
    var env = envelope(result);
    if (env.cancelled)
        return null;
    if (env.ok)
        return { message: doing || "Done.", failed: false };
    return { message: env.error, failed: true };
}

function withNote(notes, key, note) {
    var next = Object.assign({}, notes);
    if (note === null)
        delete next[key];
    else
        next[key] = note;
    return next;
}

// ---- Sizes and times ---------------------------------------------------------------------------

/// "1.5 GiB", "340 KiB", "12 B".
function bytesText(n) {
    var v = Number(n);
    if (!isFinite(v) || v < 0)
        return "unknown";
    var units = ["B", "KiB", "MiB", "GiB", "TiB"];
    var i = 0;
    while (v >= 1024 && i < units.length - 1) {
        v /= 1024;
        ++i;
    }
    var text = i === 0 || v >= 100 ? String(Math.round(v)) : v.toFixed(1).replace(/\.0$/, "");
    return text + " " + units[i];
}

function speedText(n) {
    return bytesText(n) + "/s";
}

/// "today", "3 hours ago", "2 days ago" for a time in unix seconds, as of `nowMs`.
function ageText(unixSeconds, nowMs) {
    var t = Number(unixSeconds);
    if (!isFinite(t) || t <= 0)
        return "";
    var s = Math.max(0, Math.floor(nowMs / 1000 - t));
    if (s < 90)
        return "just now";
    if (s < 3600)
        return plural(Math.round(s / 60), "minute", "minutes") + " ago";
    if (s < 86400)
        return plural(Math.round(s / 3600), "hour", "hours") + " ago";
    var days = Math.floor(s / 86400);
    return days === 1 ? "yesterday" : plural(days, "day", "days") + " ago";
}

// ---- Reads ---------------------------------------------------------------------------------------

/// {ok, daemonDown, problem} for a read that failed, or null when it answered.
function failed(result, what) {
    var env = envelope(result);
    if (env.ok)
        return null;
    return { ok: false, daemonDown: env.code === "daemon_down", problem: env.error, what: what };
}

function networkLine(n) {
    if (!isObject(n))
        return "";
    if (!n.online)
        return "Offline: downloads wait until the connection is back.";
    if (n.metered && !n.open)
        return "Metered connection: downloads are held. Change that under Preferences.";
    if (n.metered)
        return "Metered connection, downloading anyway.";
    return "";
}

function statusLine(s) {
    if (s.running === 0 && s.queued === 0)
        return "Nothing is downloading.";
    var parts = [];
    if (s.running > 0)
        parts.push(plural(s.running, "download", "downloads") + " running at " + speedText(s.speed));
    if (s.queued > 0)
        parts.push(plural(s.queued, "waiting", "waiting"));
    return parts.join(", ") + ".";
}

function parseStatus(result) {
    var bad = failed(result, "Hauler's status");
    if (bad)
        return bad;
    var o = envelope(result).output;
    if (!isObject(o) || !isObject(o.counts))
        return { ok: false, daemonDown: false, problem: "Hauler's answer wasn't what this page expects.", what: "Hauler's status" };
    var s = {
        ok: true,
        version: clip(o.version),
        running: Number(o.running) || 0,
        queued: Number(o.queued) || 0,
        speed: Number(o.speed) || 0,
        network: o.network,
        counts: o.counts
    };
    s.line = statusLine(s);
    s.networkLine = networkLine(o.network);
    return s;
}

function hostOf(url) {
    var m = /^[a-z][a-z0-9+.-]*:\/\/([^\/:?#]+)/i.exec(String(url));
    return m ? clip(m[1]) : "";
}

var ICONS = [
    [/\.(iso|img|dmg)$/i, "media-optical"],
    [/\.(zip|7z|rar|tar|gz|xz|zst|bz2|tgz)$/i, "package-x-generic"],
    [/\.(mp4|mkv|webm|avi|mov)$/i, "video-x-generic"],
    [/\.(mp3|flac|ogg|opus|wav|m4a)$/i, "audio-x-generic"],
    [/\.(png|jpe?g|gif|webp|svg)$/i, "image-x-generic"],
    [/\.pdf$/i, "application-pdf"],
    [/\.(deb|rpm|pkg|appimage|exe|msi|flatpak)$/i, "application-x-executable"]
];

function iconFor(name) {
    for (var i = 0; i < ICONS.length; ++i)
        if (ICONS[i][0].test(String(name)))
            return ICONS[i][1];
    return "text-x-generic";
}

var STATE_LABELS = { running: "Downloading", queued: "Waiting", waiting: "Held", paused: "Paused", completed: "Completed", failed: "Failed" };

function row(t, nowMs) {
    var name = clip(t.name || "") || clip(String(t.shown_url || "").split("/").pop()) || "download";
    var size = t.size === null || t.size === undefined ? null : Number(t.size);
    var received = Number(t.received) || 0;
    var percent = size && size > 0 ? Math.min(100, Math.floor(received * 100 / size)) : -1;
    var finishedAt = Number(t.finished_at) || 0;
    return {
        id: String(t.id),
        name: name,
        icon: iconFor(name),
        host: hostOf(t.shown_url),
        shownUrl: clip(t.shown_url),
        state: String(t.state),
        stateLabel: STATE_LABELS[t.state] || clip(t.state),
        size: size,
        received: received,
        percent: percent,
        speed: Number(t.speed) || 0,
        error: clip(t.error || ""),
        finishedAt: finishedAt,
        finished: ageText(finishedAt, nowMs),
        dir: clip(t.dir),
        path: t.path ? clip(t.path) : "",
        fileExists: t.file_exists === true,
        hasHeaders: t.has_headers === true,
        resumable: t.resumable,
        scan: isObject(t.scan) ? { state: String(t.scan.state), detail: clip(t.scan.detail || "") } : null
    };
}

/// What oysterAV said about a finished download, in words ("" when it was never scanned).
function scanText(r) {
    if (!r.scan)
        return "";
    switch (r.scan.state) {
    case "scanning":
        return "oysterAV is scanning it…";
    case "clean":
        return "oysterAV found no threats.";
    case "threat":
        return "oysterAV found " + (r.scan.detail || "a threat") + ". Don't open it; see oysterAV for what to do.";
    default:
        return "oysterAV couldn't scan it" + (r.scan.detail ? ": " + r.scan.detail : ".");
    }
}

/// Counts over the completed downloads: how many oysterAV found something in, is looking at, or hasn't seen.
function scanTotals(list) {
    var t = { threats: 0, scanning: 0, clean: 0, errors: 0, unscanned: 0, scannable: 0 };
    (list && list.ok ? list.rows : []).forEach(function (r) {
        if (r.state !== "completed")
            return;
        if (!r.scan) {
            t.unscanned++;
        } else if (r.scan.state === "threat") {
            t.threats++;
        } else if (r.scan.state === "scanning") {
            t.scanning++;
        } else if (r.scan.state === "clean") {
            t.clean++;
        } else {
            t.errors++;
        }
        if (r.fileExists)
            t.scannable++;
    });
    return t;
}

/// {ok, rows, problem}: every download `hauler list --json` printed, as rows.
function parseList(result, nowMs) {
    var bad = failed(result, "the download list");
    if (bad)
        return bad;
    var o = envelope(result).output;
    if (!isObject(o) || !Array.isArray(o.tasks))
        return { ok: false, daemonDown: false, problem: "Hauler's answer wasn't what this page expects.", what: "the download list" };
    return { ok: true, rows: o.tasks.map(function (t) { return row(t, nowMs); }) };
}

var QUEUE_ORDER = { running: 0, waiting: 1, queued: 2, paused: 3, failed: 4 };

/// What is still to finish, running ones first. `shown` is capped; `more` says how many aren't.
function queueRows(list) {
    var rows = (list && list.ok ? list.rows : []).filter(function (r) { return r.state !== "completed" && r.state !== "failed"; });
    rows.sort(function (a, b) { return (QUEUE_ORDER[a.state] - QUEUE_ORDER[b.state]) || (Number(a.id) - Number(b.id)); });
    return { shown: rows.slice(0, MAX_ROWS), more: Math.max(0, rows.length - MAX_ROWS), count: rows.length };
}

function matchesFilter(r, filter) {
    return filter === "finished" ? true : r.state === filter;
}

/// The finished downloads for the History tab, newest first.
function historyRows(list, filter) {
    var rows = (list && list.ok ? list.rows : []).filter(function (r) { return (r.state === "completed" || r.state === "failed") && matchesFilter(r, filter || "finished"); });
    rows.sort(function (a, b) { return (b.finishedAt - a.finishedAt) || (Number(b.id) - Number(a.id)); });
    return { shown: rows.slice(0, MAX_ROWS), more: Math.max(0, rows.length - MAX_ROWS), count: rows.length };
}

function historyLine(h) {
    if (h.count === 0)
        return "Nothing here yet.";
    var bytes = 0;
    for (var i = 0; i < h.shown.length; ++i)
        if (h.shown[i].state === "completed" && h.shown[i].size)
            bytes += h.shown[i].size;
    return plural(h.count, "download", "downloads") + (bytes > 0 && h.more === 0 ? ", " + bytesText(bytes) + " in all" : "") + ".";
}

/// The line under a history row: where it came from, how big, when.
function rowLine(r) {
    var parts = [];
    if (r.host)
        parts.push(r.host);
    if (r.state === "completed" && r.size !== null)
        parts.push(bytesText(r.size));
    if (r.finished)
        parts.push((r.state === "failed" ? "failed " : "finished ") + r.finished);
    return parts.join(" · ");
}

/// How many finished downloads still hold cookies or tokens.
function headerHolders(list) {
    return (list && list.ok ? list.rows : []).filter(function (r) { return r.state === "completed" && r.hasHeaders; }).length;
}

function queueLine(r) {
    if (r.state === "running")
        return (r.percent >= 0 ? r.percent + "% of " + bytesText(r.size) + " · " : bytesText(r.received) + " · ") + speedText(r.speed);
    if (r.state === "failed" && r.error)
        return r.error;
    if (r.percent >= 0)
        return r.stateLabel + " · " + r.percent + "% of " + bytesText(r.size);
    return r.stateLabel;
}

function storageLines(s) {
    var out = [];
    out.push("Finished downloads: " + plural(s.finished.count, "file", "files") + ", " + bytesText(s.finished.bytes) + ".");
    out.push("Unfinished downloads: " + plural(s.unfinished.count, "download", "downloads") + ", " + bytesText(s.unfinished.bytes) + " reserved on disk.");
    return out;
}

function parseStorage(result) {
    var bad = failed(result, "the download folder");
    if (bad)
        return bad;
    var o = envelope(result).output;
    if (!isObject(o) || !isObject(o.finished) || !isObject(o.unfinished) || !isObject(o.leftovers))
        return { ok: false, daemonDown: false, problem: "Hauler's answer wasn't what this page expects.", what: "the download folder" };
    var free = o.free_bytes === null || o.free_bytes === undefined ? null : Number(o.free_bytes);
    var total = o.total_bytes === null || o.total_bytes === undefined ? null : Number(o.total_bytes);
    var s = {
        ok: true,
        dir: clip(o.dir),
        dirOk: o.dir_ok === true,
        free: free,
        total: total,
        usedFraction: free !== null && total ? Math.max(0, Math.min(1, 1 - free / total)) : -1,
        minFree: Number(o.min_free) || 0,
        finished: { count: Number(o.finished.count) || 0, bytes: Number(o.finished.bytes) || 0 },
        unfinished: { count: Number(o.unfinished.count) || 0, bytes: Number(o.unfinished.bytes) || 0 },
        leftovers: {
            count: Number(o.leftovers.count) || 0,
            bytes: Number(o.leftovers.bytes) || 0,
            files: (Array.isArray(o.leftovers.files) ? o.leftovers.files : []).slice(0, 20).map(function (f) {
                return { name: clip(f.name), dir: clip(f.dir), bytes: Number(f.bytes) || 0 };
            })
        }
    };
    s.freeLine = free === null ? "Free space unknown." : bytesText(free) + " free" + (total ? " of " + bytesText(total) : "") + ".";
    return s;
}

/// {ok, values, schema, problem}: `hauler config show --json`.
function parseConfig(result) {
    var bad = failed(result, "Hauler's settings");
    if (bad)
        return bad;
    var o = envelope(result).output;
    if (!isObject(o) || !isObject(o.settings))
        return { ok: false, daemonDown: false, problem: "Hauler's answer wasn't what this page expects.", what: "Hauler's settings" };
    return { ok: true, values: o.settings, schema: Array.isArray(o.schema) ? o.schema : [] };
}

