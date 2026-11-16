// The Preferences tab: how many downloads run, with how many connections, at what speed, and what to do on a
// metered connection. The settings are followed, so a change made in Hauler's own window shows here.
import QtQuick
import QtQuick.Layouts
import org.uq.settings
import "logic.js" as Logic

SettingsTab {
    id: tab

    property var host

    title: "Preferences"
    iconName: "configure"

    SettingsCard {
        title: "Downloads"
        iconName: "configure"
        help: "Hauler checks every value before it saves it, and says why when it refuses."
        wide: true
        Layout.fillWidth: true

        Loading {
            loading: !!tab.host && tab.host.config === null
            problem: tab.host && tab.host.config !== null && !tab.host.config.ok ? tab.host.config.problem : ""
            what: "Hauler's settings"
            Layout.fillWidth: true
        }
        Repeater {
            model: tab.host && tab.host.config !== null && tab.host.config.ok ? Logic.PREFS : []

            PrefRow {
                required property var modelData
                required property int index

                host: tab.host
                def: modelData
                settingId: index === 0 ? "preferences" : ""
                Layout.fillWidth: true
            }
        }
    }
}
