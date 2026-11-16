// The Security tab: what Hauler refuses, and what it keeps. Only https can be required, saved logins can be
// forgotten, and the safeguards that are always on are listed so nobody has to take them on trust.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.uq.settings
import "logic.js" as Logic

SettingsTab {
    id: tab

    property var host
    readonly property bool configured: !!tab.host && tab.host.config !== null && tab.host.config.ok
    readonly property int holders: Logic.headerHolders(tab.host ? tab.host.history : null)

    title: "Security"
    iconName: "security-medium"

    SettingsCard {
        title: "Connections"
        iconName: "security-high"
        help: "Applies to new downloads and to waiting ones when they start."
        Layout.fillWidth: true

        Loading {
            loading: !!tab.host && tab.host.config === null
            problem: tab.host && tab.host.config !== null && !tab.host.config.ok ? tab.host.config.problem : ""
            what: "Hauler's settings"
            Layout.fillWidth: true
        }
        SwitchRow {
            objectName: "https-only"
            settingId: "https-only"
            label: "Only allow https"
            highlight: tab.host ? tab.host.highlight : ""
            visible: tab.configured
            enabled: !!tab.host && !tab.host.busy
            value: tab.configured && tab.host.setting("https_only") === true
            onChosen: v => tab.host.setConfig("https_only", v, v)
            Layout.fillWidth: true
        }
        UqLabel {
            text: "Refuses a link that starts with http://, and a download whose server redirects to http. A download that was already waiting fails when it starts."
            visible: tab.configured
            opacity: 0.7
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        Outcome {
            host: tab.host
            noteKey: "cfg:https_only"
            Layout.fillWidth: true
        }
    }
    SettingsCard {
        title: "Saved cookies and logins"
        iconName: "preferences-system-privacy"
        help: "Cookies and headers are what lets a download from a page you are signed in to keep going after a pause. Hauler keeps them in a database only you can read."
        Layout.fillWidth: true

        SwitchRow {
            objectName: "scrub-headers"
            settingId: "saved-logins"
            label: "Forget them when a download completes"
            highlight: tab.host ? tab.host.highlight : ""
            visible: tab.configured
            enabled: !!tab.host && !tab.host.busy
            value: tab.configured && tab.host.setting("scrub_headers") === true
            onChosen: v => tab.host.setConfig("scrub_headers", v, v)
            Layout.fillWidth: true
        }
        UqLabel {
            objectName: "holders"
            text: tab.holders === 0 ? "No finished download holds saved cookies or headers." : Logic.plural(tab.holders, "finished download still holds", "finished downloads still hold") + " saved cookies or headers."
            visible: !!tab.host && tab.host.history !== null && tab.host.history.ok
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        QQC2.Button {
            objectName: "forget-logins"
            text: "Forget them now"
            visible: tab.holders > 0
            enabled: !!tab.host && !tab.host.busy
            onClicked: tab.host.forgetLogins()
        }
        Outcome {
            host: tab.host
            noteKey: "forget"
            Layout.fillWidth: true
        }
        Outcome {
            host: tab.host
            noteKey: "cfg:scrub_headers"
            Layout.fillWidth: true
        }
    }
    SettingsCard {
        title: "Always on"
        iconName: "emblem-ok-symbolic"
        help: "These are how Hauler is built; there is nothing to switch."
        wide: true
        Layout.fillWidth: true

        Repeater {
            model: [
                "Only you can reach Hauler: its socket and its database are readable by your account alone.",
                "Cookies and passwords are sent only to the site they were given for. A redirect to another site drops them.",
                "Hauler only ever deletes files it made: the finished file, or its own .part and .hauler files.",
                "A file that changed on the server is downloaded again instead of being mixed with the old one; a checksum that fails starts over.",
                "Only http and https links are accepted. This page shows addresses without their login, query or fragment."
            ]

            UqLabel {
                required property string modelData

                text: "• " + modelData
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
        }
    }
}
