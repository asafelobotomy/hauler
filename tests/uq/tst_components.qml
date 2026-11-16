// The stub org.uq.settings module loads and each component has the documented API. A page's
// repo can drop this file; it tests the stub, not the page.
import QtQuick
import QtQuick.Layouts
import QtTest
import org.uq.settings

Item {
    width: 600
    height: 400

    ColumnLayout {
        anchors.fill: parent

        UqLabel {
            id: label
            text: "<b>x</b>"
        }
        UqComboBox {
            id: combo
            model: ["One", "Two"]
        }
        SettingRow {
            id: row
            settingId: "a"
            label: "A"
            UqLabel {
                text: "in the row"
            }
        }
        SwitchRow {
            id: switchRow
            label: "On"
            value: true
        }
        ValueSlider {
            id: slider
            label: "Level"
            value: 0.5
        }
        ErrorCard {
            id: card
            title: "Failed"
            detail: "why"
        }
        // uq_api = 2
        SettingsPage {
            id: tabbed
            title: "Storage"
            Layout.fillWidth: true
            Layout.preferredHeight: 200
            SettingsTab {
                id: first
                title: "Overview"
                SettingsCard {
                    id: settingsCard
                    title: "Space"
                    help: "What you can free"
                    UqAppIcon {
                        id: appIcon
                        name: "/etc/passwd"
                        label: "steam"
                    }
                }
            }
            SettingsTab {
                id: second
                title: "Apps"
            }
        }
    }

    TestCase {
        name: "StubComponents"
        when: windowShown

        function test_api() {
            compare(label.textFormat, Text.PlainText);
            compare(combo.currentText, "One");
            verify(!row.flashing);
            row.highlight = "a";
            verify(row.flashing);
            compare(switchRow.toggle.checked, true);
            compare(slider.slider.value, 0.5);
            compare(card.title, "Failed");
        }

        function test_api2_layout() {
            compare(tabbed.tabList.length, 2);
            compare(tabbed.currentIndex, 0);
            second.activate();
            compare(tabbed.currentIndex, 1);
            tabbed.show(first);
            compare(tabbed.currentIndex, 0);
            compare(settingsCard.title, "Space");
            verify(!appIcon.plainName, "a path is never looked up as an icon");
            verify(!appIcon.found);
        }

        function test_switch_snaps_back_until_the_page_agrees() {
            let chosen = null;
            switchRow.chosen.connect(v => chosen = v);
            switchRow.toggle.toggle();
            switchRow.toggle.toggled();
            compare(chosen, false);
            compare(switchRow.toggle.checked, true, "shows value again");
        }
    }
}
