// The History tab: finished downloads, what can be removed (always after a plan), clearing, and how long they are kept.
import QtQuick
import QtTest

PageHarness {
    id: root

    TestCase {
        id: tc
        name: "History"
        when: windowShown

        function init() {
            root.reset();
            root.tab("History");
        }

        function names() {
            return root.allNamed("history-name").map(it => it.text);
        }

        function test_lists_finished_downloads_newest_first() {
            compare(names(), ["wallpaper.png", "holiday.mp4", "broken.zip", "notes.pdf", "ubuntu-26.04-desktop.iso"]);
            verify(root.showing("5 downloads") !== null);
        }

        function test_a_failed_download_says_why_and_a_missing_file_says_so() {
            verify(root.showing("network error: error sending request") !== null);
            verify(root.showing("The file is no longer in /home/user/Downloads.") !== null);
            compare(root.allNamed("history-problem").length, 2, "only the failed one and the one whose file is gone");
        }

        function test_the_address_is_shown_without_its_login_or_query() {
            for (const text of root.everyText()) {
                verify(text.indexOf("token=") < 0 && text.indexOf("sig=") < 0, text);
                verify(text.indexOf("secret") < 0, text);
            }
            verify(root.showing("127.0.0.1") !== null, "the host is shown");
        }

        function test_the_filter_narrows_the_list() {
            root.choose(root.named("history-filter"), 2);
            compare(names(), ["broken.zip"]);
            root.choose(root.named("history-filter"), 1);
            compare(names(), ["wallpaper.png", "holiday.mp4", "notes.pdf", "ubuntu-26.04-desktop.iso"]);
        }

        function test_removing_one_sends_its_id_after_a_plan_and_says_what_went() {
            root.press(root.named("remove-8"));
            tryVerify(() => root.runs("remove-one").length === 1);
            compare(root.runs("remove-one")[0][2], ["8"]);
            tryVerify(() => root.showing("Removed 1 download from the list.") !== null);
        }

        function test_deleting_the_file_is_offered_only_where_there_is_a_file() {
            verify(root.named("delete-8") !== null);
            verify(root.named("delete-2") === null, "its file is gone");
            verify(root.named("delete-4") === null, "a failed download made no file");
            root.press(root.named("delete-8"));
            tryVerify(() => root.runs("remove-one-files").length === 1);
            tryVerify(() => root.showing("deleted 1 file (3 MiB)") !== null);
        }

        function test_declining_the_plan_changes_nothing_and_says_nothing() {
            root.fakeCtx.confirms = false;
            root.press(root.named("remove-8"));
            wait(100);
            compare(root.pageLoader.item.notes["row:8"], undefined);
            verify(!root.pageLoader.item.busy);
        }

        function test_clearing_sends_the_state_and_the_files_choice() {
            root.press(root.named("clear-failed"));
            tryVerify(() => root.runs("clear-history").length === 1);
            compare(root.runs("clear-history")[0][2], ["failed"]);
            tryVerify(() => !root.pageLoader.item.busy);
            const box = root.named("clear-with-files");
            box.checked = true;
            root.press(root.named("clear-completed"));
            tryVerify(() => root.runs("clear-history-files").length === 1);
            compare(root.runs("clear-history-files")[0][2], ["completed"]);
            tryVerify(() => root.showing("deleted 3 files (9 MiB)") !== null);
        }

        function test_retention_asks_with_the_plan_when_something_would_leave() {
            root.choose(root.named("retention"), 2);
            tryVerify(() => root.runs("config-set-confirmed").length === 1);
            compare(root.runs("preview-config")[0][2], ["history_days", "30"]);
            compare(root.runs("config-set-confirmed")[0][2], ["history_days", "30"]);
            compare(root.runs("config-set").length, 0);
        }

        function test_retention_saves_at_once_when_nothing_would_leave() {
            root.fakeCtx.commands["preview-config"] = root.ok({ summary: "Nothing is older than 90 days.", changes: [], would_refuse: false, reason: "" });
            root.choose(root.named("retention"), 3);
            tryVerify(() => root.runs("config-set").length === 1);
            compare(root.runs("config-set")[0][2], ["history_days", "90"]);
            compare(root.runs("config-set-confirmed").length, 0, "no sheet for a change that removes nothing");
        }

        function test_keeping_for_ever_needs_no_plan() {
            root.fakeCtx.commands.config = root.config({ history_days: 30 });
            root.fakeCtx.change("config");
            tryVerify(() => root.named("retention").currentIndex === 2);
            root.choose(root.named("retention"), 0);
            tryVerify(() => root.runs("config-set").length === 1);
            compare(root.runs("config-set")[0][2], ["history_days", "0"]);
            compare(root.runs("preview-config").length, 0);
        }

        function test_a_refused_retention_says_why_and_the_box_shows_what_is_saved() {
            root.fakeCtx.commands["preview-config"] = root.ok({ summary: "history_days: must be 0 to 36500", changes: [], would_refuse: true, reason: "history_days: must be 0 to 36500" });
            root.choose(root.named("retention"), 4);
            tryVerify(() => root.showing("must be 0 to 36500") !== null);
            compare(root.runs("config-set").length + root.runs("config-set-confirmed").length, 0);
        }

        function test_a_value_not_in_the_presets_is_still_shown() {
            root.fakeCtx.commands.config = root.config({ history_days: 45 });
            root.fakeCtx.change("config");
            tryVerify(() => root.named("retention").currentIndex === 5);
            compare(root.named("retention").model[5], "45 days");
        }

        function test_an_empty_history_says_so() {
            root.fakeCtx.commands.history = root.ok({ ok: true, tasks: [] });
            root.fakeCtx.change("history");
            tryVerify(() => root.shown("Nothing here yet.") !== null);
        }

        function test_a_failed_read_says_so() {
            root.fakeCtx.commands.history = { code: 1, error: "exit code 1", output: { ok: false, error: "bad message from haulerd", code: "error" } };
            root.fakeCtx.change("history");
            tryVerify(() => root.shown("Couldn't read the history") !== null);
            verify(root.showing("bad message from haulerd") !== null);
        }

        function test_names_from_the_server_are_plain_text() {
            root.fakeCtx.commands.history = root.history(tasks => {
                tasks[0].name = "<a href='http://evil'>click</a>.zip";
            });
            root.fakeCtx.change("history");
            tryVerify(() => root.shown("<a href='http://evil'>click</a>.zip") !== null);
        }

        function test_a_threat_is_marked_on_its_row_and_the_page_warns() {
            root.tab("History");
            verify(root.showing("oysterAV found Win.Test.EICAR_HDB-1. Don't open it") !== null);
            verify(root.showing("oysterAV found no threats.") !== null);
            verify(root.named("scan-threats") !== null, "the banner");
            // Scanned rows offer a rescan, unscanned ones a scan, and a row whose file is gone neither.
            verify(root.named("rescan-3") !== null);
            verify(root.named("rescan-1") !== null);
            verify(root.named("scan-8") !== null);
            verify(root.named("scan-2") === null && root.named("rescan-2") === null);
        }

        function test_scanning_one_download_sends_its_id() {
            root.tab("History");
            root.press(root.named("scan-8"));
            tryVerify(() => root.runs("scan-one").length === 1);
            compare(root.runs("scan-one")[0][2], ["8"]);
            tryVerify(() => root.showing("Asked oysterAV to scan it.") !== null);
        }
    }

    Component.onCompleted: root.tc = tc
}
