// Node tests for the Downloads page: logic.js, page.toml and what the QML may send.
//   node --test 'tests/uq/*.test.mjs'
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { loadLogic } from "./load-logic.mjs";
import { parseToml } from "./toml-lite.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const pageDir = join(here, "../../uq/pages/hauler");
const logic = loadLogic(join(pageDir, "logic.js"));
const manifest = parseToml(readFileSync(join(pageDir, "page.toml"), "utf8"));
const qmlFiles = readdirSync(pageDir).filter((f) => f.endsWith(".qml")).sort();
const qmlOf = (f) => readFileSync(join(pageDir, f), "utf8");
const allQml = qmlFiles.map(qmlOf).join("\n");
const fixture = (name) => JSON.parse(readFileSync(join(here, "fixtures", name), "utf8"));
// logic.js runs in its own context; its arrays and objects have another prototype, so compare plain copies
const plain = (v) => JSON.parse(JSON.stringify(v));
const ok = (output) => ({ code: 0, output });
const command = (id) => manifest.command.find((c) => c.id === id);
// The moment the history fixture was recorded, a little after its newest finish
const NOW = 1794843000 * 1000;

// ---- page.toml ----------------------------------------------------------------------------------------------

test("manifest: uq_api 3, unique ids, and the file the page names exists", () => {
  assert.equal(manifest.uq_api, 3);
  assert.equal(manifest.id, "hauler");
  assert.ok(qmlFiles.includes(manifest.qml));
  const ids = manifest.command.map((c) => c.id);
  assert.equal(new Set(ids).size, ids.length);
  const settings = manifest.setting.map((s) => s.id);
  assert.equal(new Set(settings).size, settings.length);
});

test("manifest: every command is hauler as you, and every one is used by the page", () => {
  for (const c of manifest.command) {
    assert.equal(c.argv[0], "/usr/bin/hauler", c.id);
    assert.ok(allQml.includes(`"${c.id}"`) || c.id.startsWith("preview-") || manifest.command.some((x) => x.preview === c.id), `${c.id} is used in QML`);
  }
  assert.deepEqual(plain(manifest.requires.bins), ["hauler"]);
});

test("manifest: every preview is a read that takes the same arguments as the command it explains", () => {
  const previewed = manifest.command.filter((c) => c.preview);
  assert.ok(previewed.length >= 7);
  for (const c of previewed) {
    const p = command(c.preview);
    assert.ok(p, `${c.id}: preview ${c.preview} exists`);
    assert.ok(c.confirm, `${c.id} has a confirm to show the plan under`);
    assert.deepEqual(plain(p.args ?? []), plain(c.args ?? []), `${c.id}: the preview takes the same arguments`);
    assert.ok(!p.confirm && !p.preview, `${p.id} asks nothing`);
    assert.equal(p.output, "json");
    assert.ok(p.argv.includes("--dry-run"), `${p.id} is a --dry-run`);
    assert.ok(!c.argv.includes("--dry-run"), c.id);
  }
});

test("manifest: what removes something asks first, with a plan, and nothing can be forced", () => {
  const removes = ["remove-one", "remove-one-files", "clear-history", "clear-history-files", "forget-headers", "cleanup"];
  for (const id of removes) {
    assert.ok(command(id).confirm, `${id} asks first`);
    assert.ok(command(id).preview, `${id} shows a plan`);
  }
  assert.ok(command("config-set-confirmed").confirm && command("config-set-confirmed").preview === "preview-config");
  assert.ok(!command("config-set").confirm, "a plain setting change asks nothing");
  for (const c of manifest.command) for (const flag of ["--force", "-f", "--yes"]) assert.ok(!c.argv.includes(flag), `${c.id} ${flag}`);
  // files are only deleted by the two commands that say so, and only after a plan
  for (const c of manifest.command.filter((x) => x.argv.includes("--delete") || x.argv.includes("--delete-files"))) {
    assert.ok(c.confirm || c.id.startsWith("preview-"), `${c.id} deletes files, so it asks first`);
  }
});

test("manifest: every command prints one JSON document, or a line of text for the three that only act", () => {
  const text = ["pause-all", "resume-all", "pause-one", "resume-one", "add-url"];
  for (const c of manifest.command) {
    if (text.includes(c.id)) assert.equal(c.output, "text", c.id);
    else {
      assert.equal(c.output, "json", c.id);
      assert.ok(c.argv.includes("--json"), c.id);
    }
  }
});

test("patterns_match_manifest: logic.js and page.toml carry the same argument patterns", () => {
  const fromManifest = {};
  for (const c of manifest.command) if (c.args) fromManifest[c.id] = c.args.map((a) => a.pattern);
  for (const c of manifest.command) for (const a of c.args ?? []) assert.ok(a.pattern.startsWith("^") && a.pattern.endsWith("$"), `${c.id}.${a.name} is anchored`);
  assert.deepEqual(plain(logic.PATTERNS), fromManifest);
});

test("watch: the reads follow haulerd's own signals, from its name only", () => {
  for (const id of ["status", "queue", "history", "storage", "config"]) {
    const signals = command(id).watch.signals;
    assert.ok(signals.length >= 2, id);
    for (const s of signals) {
      assert.equal(s.sender, "org.hauler.Hauler1", id);
      assert.equal(s.path, "/org/hauler/Hauler1", id);
      assert.equal(s.bus, "session", id);
    }
    const members = signals.map((s) => s.member);
    assert.ok(members.includes("Changed") && members.includes("Finished"), id);
  }
  assert.ok(manifest.command.filter((c) => c.watch).length <= 8, "uQ allows eight watches per page");
  // the signals Hauler's bar widget already follows exist on the same interface
  assert.ok(readFileSync(join(here, "../../crates/hauler-daemon/src/bus/hauler1.rs"), "utf8").includes("pub async fn changed"));
});

test("qml: every [[setting]] row is reachable, and no file is over 400 lines", () => {
  for (const s of manifest.setting) assert.ok(allQml.includes(`settingId: "${s.id}"`) || allQml.includes(`? "${s.id}"`), `${s.id}`);
  for (const f of qmlFiles) assert.ok(qmlOf(f).split("\n").length <= 400, `${f} is over 400 lines`);
});

test("qml: text is shown through UqLabel only, no Kirigami, and buttons use fixed text", () => {
  const banned = /(^|[^A-Za-z0-9_.])(Text|Label|TextEdit|TextInput)\s*\{/m;
  for (const f of qmlFiles) assert.ok(!banned.test(qmlOf(f)), `${f} shows text with a stock item`);
  assert.ok(!/Qt\.createQmlObject/.test(allQml));
  assert.ok(!/import org\.kde\.kirigami/.test(allQml));
  for (const m of allQml.matchAll(/QQC2\.(?:Button|CheckBox)\s*\{[^}]*?\btext:\s*([^\n]+)/g)) assert.match(m[1].trim(), /^"[^"+]*"$/, `a button's text is a fixed string: ${m[1]}`);
});

// ---- Arguments -------------------------------------------------------------------------------------------------------

test("argsProblem: accepts what the page sends and refuses options and junk", () => {
  assert.equal(logic.argsProblem("pause-one", ["5"]), "");
  assert.notEqual(logic.argsProblem("pause-one", ["-5"]), "");
  assert.notEqual(logic.argsProblem("pause-one", ["0"]), "");
  assert.notEqual(logic.argsProblem("pause-one", ["5 6"]), "");
  assert.notEqual(logic.argsProblem("remove-one", ["--delete"]), "");
  assert.equal(logic.argsProblem("clear-history", ["finished"]), "");
  assert.notEqual(logic.argsProblem("clear-history", ["all"]), "");
  assert.equal(logic.argsProblem("add-url", ["https://example.com/a.iso?x=1&y=2"]), "");
  assert.notEqual(logic.argsProblem("add-url", ["ftp://example.com/a"]), "");
  assert.notEqual(logic.argsProblem("add-url", ["https://example.com/a b"]), "");
  assert.notEqual(logic.argsProblem("add-url", ["-H"]), "");
  assert.equal(logic.argsProblem("config-set", ["download_dir", "/home/me/My Downloads"]), "");
  assert.equal(logic.argsProblem("config-set", ["min_free", "0"]), "");
  assert.notEqual(logic.argsProblem("config-set", ["colour", "red"]), "");
  assert.notEqual(logic.argsProblem("config-set", ["max_active", "3\n4"]), "");
  assert.notEqual(logic.argsProblem("config-set", ["max_active"]), "");
  assert.equal(logic.argsProblem("pause-all", []), "");
  assert.notEqual(logic.argsProblem("pause-all", ["x"]), "");
});

test("config-set: every key hauler lists is accepted by the manifest's key pattern, and no other", () => {
  const keys = Object.keys(fixture("config.json").settings);
  assert.deepEqual(plain(keys).sort(), fixture("config.json").schema.map((s) => s.key).sort());
  const pattern = new RegExp(command("config-set").args[0].pattern);
  for (const k of keys) assert.ok(pattern.test(k), k);
  assert.ok(!pattern.test("max_active2") && !pattern.test("x_max_active"));
  // the preferences tab covers the four settings that have no other home
  const elsewhere = ["download_dir", "history_days", "scrub_headers", "https_only", "min_free"];
  assert.deepEqual(plain(logic.PREFS.map((p) => p.key)).concat(elsewhere).sort(), plain(keys).sort());
});

test("preferences: the limits the page checks are the ones hauler publishes", () => {
  const schema = Object.fromEntries(fixture("config.json").schema.map((s) => [s.key, s]));
  for (const def of logic.PREFS) {
    const s = schema[def.key];
    assert.ok(s, def.key);
    assert.equal(s.kind, def.kind, def.key);
    if (def.kind === "int") {
      assert.equal(s.min, def.min, def.key);
      assert.equal(s.max ?? 0, def.max, def.key);
    }
  }
});

// ---- envelope --------------------------------------------------------------------------------------------------------

test("envelope: a refusal keeps hauler's words; a cancel is not an error; text output is fine", () => {
  assert.equal(logic.envelope(ok({ ok: true })).ok, true);
  assert.equal(logic.envelope({ code: 0, output: "paused 1\n" }).ok, true);
  const refused = logic.envelope({ code: 1, error: "exit code 1", output: { ok: false, error: "connections: must be 1 to 32", code: "error" } });
  assert.equal(refused.ok, false);
  assert.equal(refused.error, "connections: must be 1 to 32");
  assert.equal(logic.envelope({ error: "Cancelled", cancelled: true }).cancelled, true);
  assert.equal(logic.envelope(null).ok, false);
  assert.equal(logic.envelope({ error: "timed out" }).error, "timed out");
  assert.equal(logic.outcome({ error: "Cancelled", cancelled: true }, "x"), null);
  assert.deepEqual(plain(logic.outcome(ok({}), "Saved.")), { message: "Saved.", failed: false });
  assert.deepEqual(plain(logic.outcome({ code: 1, output: { ok: false, error: "no", code: "refused" } }, "Saved.")), { message: "no", failed: true });
});

test("withNote adds and removes without touching the old notes", () => {
  const a = {};
  const b = logic.withNote(a, "x", { message: "m" });
  assert.deepEqual(plain(a), {});
  assert.deepEqual(plain(logic.withNote(b, "x", null)), {});
});

// ---- The reads, from what the real commands print ------------------------------------------------------------------

test("parseStatus: the line, the network, and a daemon that isn't running", () => {
  const s = logic.parseStatus(ok(fixture("status.json")));
  assert.equal(s.ok, true);
  assert.equal(s.line, "1 download running at 1.5 MiB/s.");
  assert.equal(s.networkLine, "");
  assert.equal(s.counts.paused, 2);
  assert.equal(logic.parseStatus(ok({ ...fixture("status.json"), running: 0, queued: 3 })).line, "3 waiting.");
  assert.equal(logic.parseStatus(ok({ ...fixture("status.json"), running: 0, queued: 0 })).line, "Nothing is downloading.");
  assert.match(logic.parseStatus(ok({ ...fixture("status.json"), network: { online: false, metered: false, open: false } })).networkLine, /^Offline/);
  assert.match(logic.parseStatus(ok({ ...fixture("status.json"), network: { online: true, metered: true, open: false } })).networkLine, /^Metered connection: downloads are held/);
  const down = logic.parseStatus({ code: 1, error: "exit code 1", output: fixture("daemon-down.json") });
  assert.equal(down.ok, false);
  assert.equal(down.daemonDown, true);
  assert.match(down.problem, /haulerd isn't running/);
  assert.equal(logic.parseStatus(ok({ nope: 1 })).ok, false);
  assert.equal(logic.parseStatus(null).ok, false);
});

test("parseList: rows with what the page shows, and nothing secret", () => {
  const list = logic.parseList(ok(fixture("history.json")), NOW);
  assert.equal(list.ok, true);
  assert.equal(list.rows.length, 5);
  const row = list.rows.find((r) => r.name === "wallpaper.png");
  assert.equal(row.icon, "image-x-generic");
  assert.equal(row.host, "127.0.0.1");
  assert.equal(row.shownUrl, "http://127.0.0.1:8099/w.png");
  assert.equal(row.hasHeaders, true);
  assert.equal(row.fileExists, true);
  const text = JSON.stringify(plain(list.rows));
  assert.ok(!text.includes("sig=abc") && !text.includes("token=") && !text.includes("secret"), "no query, token or cookie reaches the page's rows");
  assert.equal(logic.parseList(ok({ ok: true }), NOW).ok, false);
});

test("queueRows: running first, then waiting, paused; failed and completed are history", () => {
  const q = logic.queueRows(logic.parseList(ok(fixture("queue.json")), NOW));
  assert.deepEqual(plain(q.shown.map((r) => r.name)), ["big-archive.tar.zst", "later.7z", "another.bin"]);
  assert.equal(q.more, 0);
  assert.equal(q.shown[0].percent, 92);
  assert.equal(logic.queueLine(q.shown[0]), "92% of 64 MiB · 1.5 MiB/s");
  assert.equal(logic.queueLine(q.shown[1]), "Paused");
  const many = { ok: true, rows: Array.from({ length: 130 }, (_, i) => ({ ...q.shown[1], id: String(i + 1) })) };
  const capped = logic.queueRows(many);
  assert.equal(capped.shown.length, logic.MAX_ROWS);
  assert.equal(capped.more, 30);
  assert.deepEqual(plain(logic.queueRows(null)), { shown: [], more: 0, count: 0 });
});

test("historyRows: newest first, filtered, capped; the summary line", () => {
  const list = logic.parseList(ok(fixture("history.json")), NOW);
  const all = logic.historyRows(list, "finished");
  assert.deepEqual(plain(all.shown.map((r) => r.name)), ["wallpaper.png", "holiday.mp4", "broken.zip", "notes.pdf", "ubuntu-26.04-desktop.iso"]);
  assert.deepEqual(plain(logic.historyRows(list, "failed").shown.map((r) => r.name)), ["broken.zip"]);
  assert.equal(logic.historyRows(list, "completed").count, 4);
  assert.equal(logic.historyLine(logic.historyRows(list, "completed")), "4 downloads, 12 MiB in all.");
  assert.equal(logic.historyLine(logic.historyRows(list, "nothing-matches")), "Nothing here yet.");
  assert.equal(logic.headerHolders(list), 1);
  const wall = list.rows.find((r) => r.name === "wallpaper.png");
  assert.equal(logic.rowLine(wall), "127.0.0.1 · 3 MiB · finished yesterday");
  assert.equal(logic.rowLine(list.rows.find((r) => r.name === "broken.zip")).includes("failed 31 days ago"), true);
});

test("parseStorage: space, the share used, what hauler keeps, and leftovers", () => {
  const s = logic.parseStorage(ok(fixture("storage.json")));
  assert.equal(s.ok, true);
  assert.equal(s.dir, "/home/user/Downloads");
  assert.ok(s.usedFraction > 0 && s.usedFraction < 1);
  assert.match(s.freeLine, /GiB free of 252 GiB\./);
  assert.deepEqual(plain(logic.storageLines(s)), ["Finished downloads: 4 files, 9 MiB.", "Unfinished downloads: 1 download, 64 MiB reserved on disk."]);
  assert.equal(s.leftovers.count, 1);
  assert.equal(s.leftovers.files[0].name, "stray-copy.iso");
  assert.equal(logic.parseStorage(ok({ ...fixture("storage.json"), free_bytes: null, total_bytes: null })).usedFraction, -1);
  assert.equal(logic.parseStorage(ok({ ok: true })).ok, false);
});

test("parseConfig: the values and the schema; presets gain the current value when it isn't one", () => {
  const c = logic.parseConfig(ok(fixture("config.json")));
  assert.equal(c.ok, true);
  assert.equal(c.values.max_active, 3);
  assert.equal(logic.parseConfig(ok({ ok: true })).ok, false);
  assert.deepEqual(plain(logic.retentionChoices(30)), { labels: ["For ever", "7 days", "30 days", "90 days", "A year"], values: [0, 7, 30, 90, 365], index: 2 });
  const odd = logic.retentionChoices(45);
  assert.equal(odd.labels[5], "45 days");
  assert.equal(odd.index, 5);
  assert.equal(logic.reserveChoices(2147483648).labels[5], "2 GiB");
  assert.equal(logic.reserveChoices(0).index, 0);
  assert.equal(logic.reserveChoices(undefined).index, -1);
});

// ---- Typing a value -------------------------------------------------------------------------------------------------------

test("parseSize and the preference checks agree with hauler's own", () => {
  assert.equal(logic.parseSize("2M"), 2097152);
  assert.equal(logic.parseSize(" 512 k "), 524288);
  assert.equal(logic.parseSize("0"), 0);
  assert.equal(logic.parseSize("1G"), 1073741824);
  assert.equal(logic.parseSize("lots"), null);
  assert.equal(logic.parseSize("1.5M"), null);
  assert.equal(logic.parseSize("2T"), null, "hauler takes K, M and G");
  const [maxActive, connections, limit] = logic.PREFS;
  assert.equal(logic.prefProblem(maxActive, "0"), "Use 1 or more.");
  assert.equal(logic.prefProblem(maxActive, "x"), "Use a whole number.");
  assert.equal(logic.prefProblem(maxActive, "4"), "");
  assert.equal(logic.prefProblem(connections, "33"), "Use a number from 1 to 32.");
  assert.equal(logic.prefProblem(limit, "2M"), "");
  assert.notEqual(logic.prefProblem(limit, "fast"), "");
  assert.equal(logic.prefValue(limit, "2M"), "2097152");
  assert.equal(logic.prefValue(maxActive, " 4 "), "4");
  assert.equal(logic.prefText(limit, 0), "0");
  assert.equal(logic.prefText(limit, 2097152), "2M");
  assert.equal(logic.prefText(maxActive, 3), "3");
});

test("folders and links", () => {
  assert.equal(logic.folderProblem("/data/downloads"), "");
  assert.notEqual(logic.folderProblem("downloads"), "");
  assert.notEqual(logic.folderProblem("/a\nb"), "");
  assert.equal(logic.folderPath("file:///home/me/My%20Files"), "/home/me/My Files");
  assert.equal(logic.linkProblem("https://example.com/a"), "");
  assert.notEqual(logic.linkProblem("example.com/a"), "");
  assert.notEqual(logic.linkProblem("javascript:alert(1)"), "");
});

test("plans and the words after a change", () => {
  const p = logic.parsePlan(ok(fixture("preview-config-retention.json")));
  assert.equal(p.ok, true);
  assert.ok(p.changes >= 1);
  assert.equal(p.refused, false);
  const refused = logic.parsePlan(ok(fixture("preview-config-refused.json")));
  assert.equal(refused.refused, true);
  assert.match(refused.reason, /must be 1 to 32/);
  assert.equal(logic.parsePlan({ error: "boom" }).ok, false);
  assert.equal(logic.removedLine(ok({ removed: 3, deleted_files: 0, bytes: 0 })), "Removed 3 downloads from the list.");
  assert.equal(logic.removedLine(ok({ removed: 1, deleted_files: 1, bytes: 3145728 })), "Removed 1 download from the list and deleted 1 file (3 MiB).");
  assert.equal(logic.cleanedLine(ok({ removed: 2, bytes: 1048576 })), "Deleted 2 unfinished downloads, freeing 1 MiB.");
  assert.equal(logic.forgottenLine(ok({ forgotten: 1 })), "Forgot the saved logins of 1 download.");
});

test("helpers: bytes, age, plural, clip, icons", () => {
  assert.equal(logic.bytesText(0), "0 B");
  assert.equal(logic.bytesText(1536), "1.5 KiB");
  assert.equal(logic.bytesText(104857600), "100 MiB");
  assert.equal(logic.bytesText(3145728), "3 MiB");
  assert.equal(logic.bytesText(-1), "unknown");
  assert.equal(logic.ageText(1000, 1000 * 1000 + 30000), "just now");
  assert.equal(logic.ageText(1000, 1000 * 1000 + 600000), "10 minutes ago");
  assert.equal(logic.ageText(1000, 1000 * 1000 + 7200000), "2 hours ago");
  assert.equal(logic.ageText(1000, 1000 * 1000 + 86400000), "yesterday");
  assert.equal(logic.ageText(1000, 1000 * 1000 + 5 * 86400000), "5 days ago");
  assert.equal(logic.ageText(null, 1), "");
  assert.equal(logic.plural(1, "file", "files"), "1 file");
  assert.equal(logic.clip("x".repeat(500)).length, logic.MAX_TEXT);
  assert.equal(logic.iconFor("a.ISO"), "media-optical");
  assert.equal(logic.iconFor("a.tar.zst"), "package-x-generic");
  assert.equal(logic.iconFor("readme"), "text-x-generic");
});
