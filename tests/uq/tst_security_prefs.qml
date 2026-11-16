// The Security and Preferences tabs.
import QtQuick
import QtTest

PageHarness {
    id: root

    TestCase {
        id: tc
        name: "SecurityAndPreferences"
        when: windowShown

        function init() {
            root.reset();
        }

        function test_https_only_shows_the_plan_when_it_would_break_waiting_downloads() {
            root.tab("Security");
            verify(!root.named("https-only").toggle.checked);
            root.flip(root.named("https-only").toggle);
            tryVerify(() => root.runs("config-set-confirmed").length === 1);
            compare(root.runs("preview-config")[0][2], ["https_only", "true"]);
            compare(root.runs("config-set-confirmed")[0][2], ["https_only", "true"]);
        }

        function test_https_only_saves_at_once_when_nothing_waits_on_http() {
            root.fakeCtx.commands["preview-config"] = root.ok({ summary: "Every waiting download is https.", changes: [], would_refuse: false, reason: "" });
            root.tab("Security");
            root.flip(root.named("https-only").toggle);
            tryVerify(() => root.runs("config-set").length === 1);
            compare(root.runs("config-set")[0][2], ["https_only", "true"]);
        }

        function test_turning_https_only_off_needs_no_plan() {
            root.fakeCtx.commands.config = root.config({ https_only: true });
            root.fakeCtx.change("config");
            root.tab("Security");
            tryVerify(() => root.named("https-only").toggle.checked);
            root.flip(root.named("https-only").toggle);
            tryVerify(() => root.runs("config-set").length === 1);
            compare(root.runs("config-set")[0][2], ["https_only", "false"]);
            compare(root.runs("preview-config").length, 0);
        }

        function test_saved_logins_are_counted_and_can_be_forgotten_after_a_plan() {
            root.tab("Security");
            verify(root.showing("1 finished download still holds saved cookies or headers.") !== null);
            root.press(root.named("forget-logins"));
            tryVerify(() => root.runs("forget-headers").length === 1);
            tryVerify(() => root.showing("Forgot the saved logins of 1 download.") !== null);
        }

        function test_nothing_to_forget_hides_the_button() {
            root.fakeCtx.commands.history = root.history(tasks => tasks.forEach(t => { t.has_headers = false; }));
            root.fakeCtx.change("history");
            root.tab("Security");
            tryVerify(() => root.named("holders").text.indexOf("No finished download holds") === 0);
            verify(root.named("forget-logins") === null);
        }

        function test_scrubbing_on_completion_is_a_switch_with_a_plan() {
            root.tab("Security");
            root.flip(root.named("scrub-headers").toggle);
            tryVerify(() => root.runs("config-set-confirmed").length === 1);
            compare(root.runs("config-set-confirmed")[0][2], ["scrub_headers", "true"]);
        }

        function test_the_always_on_safeguards_are_listed() {
            root.tab("Security");
            verify(root.showing("Hauler only ever deletes files it made") !== null);
            verify(root.showing("Only you can reach Hauler") !== null);
        }

        function test_preferences_show_what_is_saved() {
            root.tab("Preferences");
            compare(root.named("field-max_active").text, "3");
            compare(root.named("field-connections").text, "8");
            compare(root.named("field-global_limit").text, "0");
            verify(root.named("switch-pause_on_metered").checked);
        }

        function test_a_number_is_checked_here_first_and_then_by_hauler() {
            root.tab("Preferences");
            root.type(root.named("field-max_active"), "abc");
            tryVerify(() => root.showing("Use a whole number.") !== null);
            compare(root.runs("config-set").length, 0);
            compare(root.named("field-max_active").text, "3", "the field shows what is saved");
            root.type(root.named("field-connections"), "40");
            tryVerify(() => root.showing("Use a number from 1 to 32.") !== null);
            compare(root.runs("config-set").length, 0);
            root.type(root.named("field-max_active"), "5");
            tryVerify(() => root.runs("config-set").length === 1);
            compare(root.runs("config-set")[0][2], ["max_active", "5"]);
        }

        function test_a_speed_limit_is_sent_in_bytes() {
            root.tab("Preferences");
            root.type(root.named("field-global_limit"), "lots");
            tryVerify(() => root.showing("Use a number, then K, M or G") !== null);
            compare(root.runs("config-set").length, 0);
            root.type(root.named("field-global_limit"), "2M");
            tryVerify(() => root.runs("config-set").length === 1);
            compare(root.runs("config-set")[0][2], ["global_limit", "2097152"]);
        }

        function test_a_refusal_by_hauler_is_shown_and_the_field_reverts() {
            root.fakeCtx.commands["config-set"] = root.refused("max_active: must be at least 1");
            root.tab("Preferences");
            root.type(root.named("field-max_active"), "2");
            tryVerify(() => root.showing("must be at least 1") !== null);
            tryVerify(() => root.named("field-max_active").text === "3");
        }

        function test_the_metered_switch_sends_its_new_value() {
            root.tab("Preferences");
            root.flip(root.named("switch-pause_on_metered"));
            tryVerify(() => root.runs("config-set").length === 1);
            compare(root.runs("config-set")[0][2], ["pause_on_metered", "false"]);
        }

        function test_a_change_made_in_the_app_shows_up() {
            root.tab("Preferences");
            root.fakeCtx.commands.config = root.config({ max_active: 7 });
            root.fakeCtx.change("config");
            tryVerify(() => root.named("field-max_active").text === "7");
        }

        function test_a_failed_read_says_so() {
            root.fakeCtx.commands.config = { code: 1, error: "exit code 1", output: { ok: false, error: "cannot reach haulerd", code: "error" } };
            root.open();
            root.tab("Preferences");
            tryVerify(() => root.shown("Couldn't read Hauler's settings") !== null);
        }

        function test_the_antivirus_switch_saves_at_once_and_the_scan_button_scans_everything() {
            root.tab("Security");
            verify(!root.named("scan-downloads").toggle.checked);
            root.flip(root.named("scan-downloads").toggle);
            tryVerify(() => root.runs("config-set").length === 1);
            compare(root.runs("config-set")[0][2], ["scan_downloads", "true"]);
            verify(root.showing("1 download has a threat") !== null);
            tryVerify(() => root.named("scan-all").enabled);
            root.press(root.named("scan-all"));
            tryVerify(() => root.runs("scan-all").length === 1);
            tryVerify(() => root.showing("Asked oysterAV to scan 2 downloads") !== null);
        }
    }

    Component.onCompleted: root.tc = tc
}
