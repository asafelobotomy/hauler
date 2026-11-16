// The Overview tab, the banners and the states a read can be in.
import QtQuick
import QtTest

PageHarness {
    id: root

    TestCase {
        id: tc
        name: "Overview"
        when: windowShown

        function init() {
            root.reset();
        }

        function test_shows_what_is_downloading_and_the_page_follows_it() {
            const page = root.pageLoader.item;
            verify(root.showing("1 download running at") !== null);
            verify(root.shown("big-archive.tar.zst") !== null);
            verify(root.showing("92% of 64 MiB") !== null, "progress in words beside the bar");
            verify(root.shown("later.7z") !== null && root.shown("another.bin") !== null, "paused ones are listed too");
            verify(root.shown("broken.zip") === null, "a failed one is history, not queue");
            const names = ["status", "queue", "history", "storage", "config"];
            for (const n of names)
                verify(root.fakeCtx.calls.some(c => c[0] === "watch" && c[1] === n), n + " is followed");
            // a download finishing somewhere else changes the status: the watch re-runs the read
            root.fakeCtx.commands.status = root.ok(Object.assign(root.fixture("status.json"), { running: 0, queued: 0 }));
            root.fakeCtx.change("status");
            tryVerify(() => root.shown("Nothing is downloading.") !== null || root.showing("Nothing is downloading") !== null);
        }

        function test_the_bar_shows_how_far_a_running_download_got() {
            const bars = root.allNamed("download-bar");
            compare(bars.length, 1, "only the running one has a bar: the paused ones never started");
            compare(bars[0].value, 92);
            verify(!bars[0].indeterminate, "the size is known");
        }

        function test_pause_and_resume_send_the_id() {
            root.press(root.named("pause-5"));
            tryVerify(() => root.runs("pause-one").length === 1);
            compare(root.runs("pause-one")[0][2], ["5"]);
            tryVerify(() => !root.pageLoader.item.busy);
            root.press(root.named("resume-6"));
            tryVerify(() => root.runs("resume-one").length === 1);
            compare(root.runs("resume-one")[0][2], ["6"]);
            verify(root.named("resume-5") === null, "a running download has no Resume");
            verify(root.named("pause-6") === null, "a paused one has no Pause");
        }

        function test_pause_all_and_resume_all() {
            root.press(root.named("pause-all"));
            tryVerify(() => root.runs("pause-all").length === 1);
            tryVerify(() => root.showing("Paused everything") !== null);
            root.press(root.named("resume-all"));
            tryVerify(() => root.runs("resume-all").length === 1);
            // every write is followed by a fresh read of everything
            tryVerify(() => root.runs("queue").length >= 3);
        }

        function test_pause_all_waits_when_there_is_nothing_to_pause() {
            root.fakeCtx.commands.status = root.ok(Object.assign(root.fixture("status.json"), { running: 0, queued: 0, counts: { total: 0, running: 0, queued: 0, paused: 0, completed: 0, failed: 0 } }));
            root.fakeCtx.change("status");
            tryVerify(() => !root.named("pause-all").enabled);
            verify(!root.named("resume-all").enabled);
        }

        function test_an_empty_queue_says_so() {
            root.fakeCtx.commands.queue = root.ok({ ok: true, tasks: [] });
            root.fakeCtx.change("queue");
            tryVerify(() => root.named("queue-empty") !== null);
        }

        function test_a_metered_connection_is_explained() {
            const doc = root.fixture("status.json");
            doc.network = { online: true, metered: true, open: false };
            root.fakeCtx.commands.status = root.ok(doc);
            root.fakeCtx.change("status");
            tryVerify(() => root.named("network-line") !== null && root.named("network-line").text.indexOf("Metered") === 0);
        }

        function test_adding_a_link_refuses_a_non_link_and_sends_a_good_one() {
            const field = root.named("link-field");
            field.text = "not a link";
            root.press(root.named("add-link"));
            verify(root.showing("Paste a full link") !== null);
            compare(root.runs("add-url").length, 0, "nothing was sent");
            field.text = "  https://example.com/a.iso  ";
            root.press(root.named("add-link"));
            tryVerify(() => root.runs("add-url").length === 1);
            compare(root.runs("add-url")[0][2], ["https://example.com/a.iso"]);
            tryVerify(() => field.text === ""); // the field empties once Hauler took the link
            verify(root.showing("Added to the queue.") !== null);
        }

        function test_a_refused_link_keeps_what_was_typed_and_says_why() {
            root.fakeCtx.commands["add-url"] = root.refused("only https downloads are allowed (Settings: https only) (code -32602)");
            const field = root.named("link-field");
            field.text = "http://example.com/a.iso";
            root.press(root.named("add-link"));
            tryVerify(() => root.showing("only https downloads are allowed") !== null);
            compare(field.text, "http://example.com/a.iso");
        }

        function test_hauler_not_running_says_so_and_keeps_the_page() {
            const answer = root.down();
            for (const id of ["status", "queue", "history", "storage", "config"])
                root.fakeCtx.commands[id] = answer;
            root.open();
            tryVerify(() => root.named("daemon-down") !== null);
            verify(root.showing("systemctl --user start hauler.socket") !== null);
            verify(root.named("tab-History") !== null, "the tabs are still there");
        }

        function test_loading_shows_a_bar_not_a_word() {
            root.open();
            const bars = [];
            root.walk(root.pageLoader.item, it => {
                if (it.visible && it.indeterminate === true)
                    bars.push(it);
            });
            verify(bars.length > 0, "an indeterminate bar while the status is read");
            verify(root.shown("Checking…") === null, "no bare Checking…");
            tryVerify(() => root.showing("1 download running") !== null);
        }

        function test_open_hauler_opens_the_app() {
            root.press(root.named("open-hauler"));
            compare(root.fakeCtx.calls.filter(c => c[0] === "openApp")[0], ["openApp", "hauler.desktop"]);
        }
    }

    Component.onCompleted: root.tc = tc
}
