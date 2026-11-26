.pragma library
.import "logic.js" as Logic
// What the Downloads page checks and words when the person changes something: preset choices, typed
// values, and the outcome of a plan. No Qt calls, so Node can test it (tests/uq/logic.test.mjs).
// Reading and wording what hauler reports is in logic.js.

var clip = Logic.clip;
var isObject = Logic.isObject;
var plural = Logic.plural;
var envelope = Logic.envelope;
var bytesText = Logic.bytesText;

/// The choices of a box of preset numbers, with the current value added when it isn't one of them:
/// {labels, values, index}.
function choices(presets, key, label, current) {
    var values = presets.map(function (p) { return p[key]; });
    var labels = presets.map(function (p) { return p.label; });
    var index = values.indexOf(current);
    if (index < 0 && current !== undefined && current !== null) {
        values.push(current);
        labels.push(label(current));
        index = values.length - 1;
    }
    return { labels: labels, values: values, index: index };
}

function retentionChoices(days) {
    return choices(Logic.RETENTION, "days", function (d) { return plural(d, "day", "days"); }, days);
}

function reserveChoices(bytes) {
    return choices(Logic.RESERVE, "bytes", bytesText, bytes);
}

// ---- Typing a value --------------------------------------------------------------------------------

/// Bytes for "5M", "2G", "512K", "0", or null when it isn't a size hauler accepts.
function parseSize(text) {
    var m = /^\s*([0-9]{1,15})\s*([kKmMgG]?)\s*$/.exec(String(text));
    if (!m)
        return null;
    var mult = { "": 1, k: 1024, m: 1048576, g: 1073741824 }[m[2].toLowerCase()];
    return Number(m[1]) * mult;
}

/// The text a field shows for a setting's value.
function prefText(def, value) {
    if (def.kind === "size")
        return Number(value) === 0 ? "0" : bytesText(value).replace(" ", "").replace("iB", "");
    return String(value === undefined || value === null ? "" : value);
}

/// Why `text` isn't a good value for the setting `def` ("" when it is).
function prefProblem(def, text) {
    if (def.kind === "size")
        return parseSize(text) === null ? "Use a number, then K, M or G, like 2M. 0 means none." : "";
    if (def.kind === "int") {
        if (!/^[0-9]{1,9}$/.test(String(text)))
            return "Use a whole number.";
        var n = Number(text);
        if (n < def.min || (def.max > 0 && n > def.max))
            return def.max > 0 ? "Use a number from " + def.min + " to " + def.max + "." : "Use " + def.min + " or more.";
    }
    return "";
}

/// What to send for the field's text: a size as the text hauler parses, a number as it is.
function prefValue(def, text) {
    if (def.kind === "size")
        return String(parseSize(text));
    return String(text).trim();
}

function folderProblem(path) {
    var text = String(path);
    if (text.charAt(0) !== "/")
        return "Use a full path, starting with /.";
    if (/[\x00-\x1f\x7f]/.test(text))
        return "That path has characters that aren't allowed.";
    return "";
}

function folderPath(url) {
    var text = String(url);
    return text.indexOf("file://") === 0 ? decodeURIComponent(text.slice(7)) : text;
}

function linkProblem(text) {
    return new RegExp(Logic.URL_PATTERN).test(String(text).trim()) ? "" : "Paste a full link, starting with http:// or https://.";
}

// ---- Plans ---------------------------------------------------------------------------------------

/// A line for the outcome of a plan that was previewed: {changes, refused, reason, summary} or a problem.
function parsePlan(result) {
    var env = envelope(result);
    if (!env.ok)
        return { ok: false, problem: env.error };
    var p = env.output;
    if (!isObject(p) || !Array.isArray(p.changes))
        return { ok: false, problem: "Hauler's answer wasn't what this page expects." };
    return { ok: true, changes: p.changes.length, refused: p.would_refuse === true, reason: clip(p.reason), summary: clip(p.summary) };
}

/// The words after a removal: "Removed 3 downloads", with the files when they went too.
function removedLine(result) {
    var env = envelope(result);
    if (!env.ok || !isObject(env.output))
        return "Done.";
    var o = env.output;
    var text = "Removed " + plural(Number(o.removed) || 0, "download", "downloads") + " from the list";
    if (o.deleted_files > 0)
        text += " and deleted " + plural(o.deleted_files, "file", "files") + " (" + bytesText(o.bytes) + ")";
    return text + ".";
}

function cleanedLine(result) {
    var env = envelope(result);
    if (!env.ok || !isObject(env.output))
        return "Done.";
    return "Deleted " + plural(Number(env.output.removed) || 0, "unfinished download", "unfinished downloads") + ", freeing " + bytesText(env.output.bytes) + ".";
}

function forgottenLine(result) {
    var env = envelope(result);
    if (!env.ok || !isObject(env.output))
        return "Done.";
    return "Forgot the saved logins of " + plural(Number(env.output.forgotten) || 0, "download", "downloads") + ".";
}

/// The note after "Scan all": how many scans were asked for.
function scanStartedLine(result) {
    var o = envelope(result).output;
    var n = o && Array.isArray(o.started) ? o.started.length : 0;
    return n === 0 ? "Nothing to scan: no finished download has its file here." : "Asked oysterAV to scan " + plural(n, "download", "downloads") + ". The answers show in History.";
}

/// One sentence over every finished download: what oysterAV has said.
function scanSummary(t) {
    var parts = [];
    if (t.threats > 0)
        parts.push(plural(t.threats, "download has", "downloads have") + " a threat");
    if (t.clean > 0)
        parts.push(plural(t.clean, "is", "are") + " clean");
    if (t.scanning > 0)
        parts.push(plural(t.scanning, "is", "are") + " being scanned");
    if (t.errors > 0)
        parts.push(plural(t.errors, "couldn't be", "couldn't be") + " scanned");
    if (t.unscanned > 0)
        parts.push(plural(t.unscanned, "was never", "were never") + " scanned");
    return parts.length === 0 ? "No finished downloads yet." : parts.join(", ") + ".";
}
