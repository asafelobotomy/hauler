// The Storage tab: the download folder, free space, the reserve, and leftovers.
import QtQuick
import QtTest

PageHarness {
    id: root

    TestCase {
        id: tc
        name: "Storage"
        when: windowShown

        function init() {
            root.reset();
            root.tab("Storage");
        }

        function test_shows_the_folder_the_space_and_what_hauler_keeps_there() {
            compare(root.named("folder-field").text, "/home/user/Downloads");
            verify(root.showing("GiB free of 252 GiB") !== null);
            verify(root.showing("Finished downloads: 4 files, 9 MiB.") !== null);
            verify(root.showing("Unfinished downloads: 1 download, 64 MiB reserved on disk.") !== null);
            verify(root.named("space-bar").value > 0 && root.named("space-bar").value < 1);
        }

        function test_a_typed_folder_is_checked_here_then_saved() {
            root.type(root.named("folder-field"), "Downloads");
            tryVerify(() => root.showing("Use a full path, starting with /.") !== null);
            compare(root.runs("config-set").length, 0);
            compare(root.named("folder-field").text, "/home/user/Downloads", "the field shows what is saved");
            root.type(root.named("folder-field"), "/data/downloads");
            tryVerify(() => root.runs("config-set").length === 1);
            compare(root.runs("config-set")[0][2], ["download_dir", "/data/downloads"]);
        }

        function test_a_folder_hauler_refuses_is_explained() {
            root.fakeCtx.commands["config-set"] = root.refused("download_dir: /nope is not an existing folder");
            root.type(root.named("folder-field"), "/nope");
            tryVerify(() => root.showing("is not an existing folder") !== null);
            tryVerify(() => root.named("folder-field").text === "/home/user/Downloads");
        }

        function test_a_missing_folder_is_called_out() {
            root.fakeCtx.commands.storage = root.storage({ dir_ok: false });
            root.fakeCtx.change("storage");
            tryVerify(() => root.named("folder-missing") !== null);
        }

        function test_the_reserve_is_chosen_from_sizes() {
            compare(root.named("reserve").currentIndex, 0);
            root.choose(root.named("reserve"), 2);
            tryVerify(() => root.runs("config-set").length === 1);
            compare(root.runs("config-set")[0][2], ["min_free", "5368709120"]);
        }

        function test_a_reserve_set_elsewhere_is_shown() {
            root.fakeCtx.commands.storage = root.storage({ min_free: 2147483648 });
            root.fakeCtx.change("storage");
            tryVerify(() => root.named("reserve").model.indexOf("2 GiB") >= 0);
            compare(root.named("reserve").currentIndex, root.named("reserve").model.indexOf("2 GiB"));
        }

        function test_leftovers_are_listed_and_cleaned_after_a_plan() {
            verify(root.showing("1 unfinished download, 64 MiB") !== null);
            verify(root.showing("stray-copy.iso") !== null);
            root.press(root.named("clean-leftovers"));
            tryVerify(() => root.runs("cleanup").length === 1);
            tryVerify(() => root.showing("Deleted 1 unfinished download, freeing 64 MiB.") !== null);
        }

        function test_no_leftovers_no_card() {
            root.fakeCtx.commands.storage = root.storage({ leftovers: { bytes: 0, count: 0, files: [] } });
            root.fakeCtx.change("storage");
            tryVerify(() => root.named("clean-leftovers") === null);
        }

        function test_declining_the_cleanup_deletes_nothing() {
            root.fakeCtx.confirms = false;
            root.press(root.named("clean-leftovers"));
            wait(100);
            verify(!root.pageLoader.item.busy);
            compare(root.pageLoader.item.notes["leftovers"], undefined);
        }

        function test_a_failed_read_says_so_and_keeps_the_tab() {
            root.fakeCtx.commands.storage = { code: 1, error: "exit code 1", output: { ok: false, error: "cannot reach haulerd", code: "error" } };
            root.fakeCtx.change("storage");
            tryVerify(() => root.shown("Couldn't read the download folder") !== null);
            verify(root.showing("cannot reach haulerd") !== null);
        }
    }

    Component.onCompleted: root.tc = tc
}
