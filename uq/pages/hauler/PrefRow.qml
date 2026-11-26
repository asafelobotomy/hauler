// One preference: a switch or a field for a number or a size, with Hauler's own refusal under it. The settings are
// followed, so after a change the row shows what Hauler holds.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.uq.settings
import "logic.js" as Logic
import "edit.js" as Edit

ColumnLayout {
    id: row

    property var host
    /// {key, label, kind, min, max, help} from Logic.PREFS.
    property var def
    property string settingId: ""
    readonly property var saved: row.host ? row.host.setting(row.def.key) : undefined
    readonly property string shown: row.saved === undefined ? "" : Edit.prefText(row.def, row.saved)
    readonly property int rev: row.host ? row.host.configRev : 0
    readonly property string noteKey: "cfg:" + row.def.key

    /// Saves what was typed; a value that isn't one is refused here, in words.
    function commit(text) {
        const why = Edit.prefProblem(row.def, text);
        if (why !== "") {
            row.host.setNote(row.noteKey, { message: why, failed: true });
            return false;
        }
        return row.host.setConfig(row.def.key, Edit.prefValue(row.def, text), false);
    }

    // After a save (accepted or refused) the field shows what Hauler holds, not what was typed.
    onRevChanged: field.text = Qt.binding(() => row.shown)

    spacing: 2

    SettingRow {
        settingId: row.settingId
        label: row.def.label
        highlight: row.host ? row.host.highlight : ""
        enabled: !!row.host && !row.host.busy && row.saved !== undefined
        Layout.fillWidth: true

        QQC2.Switch {
            objectName: "switch-" + row.def.key
            visible: row.def.kind === "bool"
            checked: row.saved === true
            onToggled: {
                const want = checked;
                checked = Qt.binding(() => row.saved === true);
                row.host.setConfig(row.def.key, want, false);
            }
        }
        QQC2.TextField {
            id: field
            objectName: "field-" + row.def.key
            visible: row.def.kind !== "bool"
            text: row.shown
            implicitWidth: 110
            onEditingFinished: {
                if (field.text === row.shown)
                    return;
                if (!row.commit(field.text))
                    field.text = Qt.binding(() => row.shown);
            }
        }
    }
    UqLabel {
        visible: row.def.help !== ""
        text: row.def.help
        opacity: 0.7
        wrapMode: Text.Wrap
        Layout.fillWidth: true
    }
    Outcome {
        host: row.host
        noteKey: row.noteKey
        Layout.fillWidth: true
    }
}
