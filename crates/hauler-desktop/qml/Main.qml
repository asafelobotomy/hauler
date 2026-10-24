import QtQuick
import QtQuick.Controls as QQC2
import Qt.labs.platform as Platform
import org.kde.kirigami as Kirigami
import org.hauler.desktop

Kirigami.ApplicationWindow {
    id: root

    property bool quitting: false
    readonly property var settings: backend.settingsJson ? JSON.parse(backend.settingsJson) : null
    readonly property var network: backend.networkJson ? JSON.parse(backend.networkJson) : null
    readonly property int runningCount: tasks.runningCount
    readonly property real totalSpeed: tasks.totalSpeed

    title: "Hauler"
    // With --minimized, start in the tray (if there is one).
    visible: !(backend.startHidden && tray.available)
    width: Kirigami.Units.gridUnit * 48
    height: Kirigami.Units.gridUnit * 32
    minimumWidth: Kirigami.Units.gridUnit * 22
    minimumHeight: Kirigami.Units.gridUnit * 16

    function showWindow() {
        root.show();
        root.raise();
        root.requestActivate();
    }

    function openAdd(text) {
        addDialog.openWith(text ?? "");
    }

    function openSettings() {
        if (pageStack.depth === 1)
            pageStack.push(settingsPage, { backend: backend, settings: root.settings });
    }

    function quit() {
        root.quitting = true;
        Qt.quit();
    }

    // Closing the window keeps downloads going; the tray brings it back.
    onClosing: close => {
        if (tray.available && !root.quitting) {
            close.accepted = false;
            root.hide();
        } else {
            Qt.quit();
        }
    }

    Backend {
        id: backend

        onTasksReset: json => tasks.reset(JSON.parse(json))
        onTaskChanged: json => tasks.upsert(JSON.parse(json))
        onTaskRemoved: id => tasks.removeId(id)
        onProgress: json => tasks.applyProgress(JSON.parse(json))
        onFailed: message => root.showPassiveNotification(message, "long")
    }

    // Development aid: with HAULER_DESKTOP_DEMO=<prefix>, save the main view,
    // a details dialog, the add dialog and settings as <prefix>-1…4.png, then
    // the main view and details at a narrow width as -5 and -6, then quit.
    // Run with QT_QPA_PLATFORM=offscreen to keep it off the screen.
    Timer {
        property int step: 0

        interval: 2500
        repeat: true
        running: backend.demoPrefix !== ""
        onTriggered: {
            step++;
            const shot = () => backend.grabWindow(backend.demoPrefix + "-" + step + ".png");
            if (step === 1) {
                shot();
                // Smoke test: links given on the command line get submitted.
                if (backend.initialUrls !== "")
                    addDialog.submit();
                // Prefer a running download: its segment map is the interesting one.
                let pick = -1;
                for (let i = 0; i < tasks.count && pick < 0; ++i) {
                    if (tasks.get(i).status === "running")
                        pick = i;
                }
                if (tasks.count > 0)
                    root.pageStack.initialPage.showDetails(tasks.get(Math.max(pick, 0)).taskId);
            } else if (step === 2) {
                shot();
                root.pageStack.initialPage.closeDetails();
                root.openAdd("https://example.com/files/big-file.iso");
            } else if (step === 3) {
                shot();
                addDialog.close();
                root.openSettings();
            } else if (step === 4) {
                shot();
                root.pageStack.pop();
                root.width = root.minimumWidth;
            } else if (step === 5) {
                shot();
                if (tasks.count > 0)
                    root.pageStack.initialPage.showDetails(tasks.get(0).taskId);
            } else if (step === 6) {
                shot();
            } else {
                root.quit();
            }
        }
    }

    TaskModel {
        id: tasks
    }

    pageStack.initialPage: TasksPage {
        model: tasks
        backend: backend
        connected: backend.connected
        network: root.network
        totalSpeed: root.totalSpeed
        runningCount: root.runningCount
        onAddRequested: root.openAdd("")
        onSettingsRequested: root.openSettings()
    }
    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.ToolBar

    Component {
        id: settingsPage

        SettingsPage {}
    }

    // Links passed on the command line (the .desktop file's %U).
    Component.onCompleted: {
        if (backend.initialUrls !== "") {
            showWindow();
            openAdd(backend.initialUrls);
        }
    }

    AddDialog {
        id: addDialog

        backend: backend
        settings: root.settings
    }

    // Drop links or text from a browser to queue them.
    DropArea {
        anchors.fill: parent
        onDropped: drop => {
            const text = drop.hasUrls ? drop.urls.map(u => u.toString()).join("\n") : drop.text;
            if (text)
                root.openAdd(text);
        }
    }

    Shortcut {
        sequences: [StandardKey.New]
        onActivated: root.openAdd("")
    }

    Shortcut {
        sequences: [StandardKey.Quit]
        onActivated: root.quit()
    }

    Platform.SystemTrayIcon {
        id: tray

        visible: true
        icon.name: "folder-download"
        tooltip: root.runningCount > 0
            ? qsTr("Hauler: %1 downloading, %2").arg(root.runningCount).arg(Fmt.speed(root.totalSpeed))
            : qsTr("Hauler")

        onActivated: reason => {
            if (reason === Platform.SystemTrayIcon.Context)
                return;
            if (root.visible && root.active)
                root.hide();
            else
                root.showWindow();
        }

        menu: Platform.Menu {
            Platform.MenuItem {
                text: root.visible ? qsTr("Hide Window") : qsTr("Show Window")
                onTriggered: root.visible ? root.hide() : root.showWindow()
            }
            Platform.MenuItem {
                text: qsTr("Add Download…")
                onTriggered: {
                    root.showWindow();
                    root.openAdd("");
                }
            }
            Platform.MenuSeparator {}
            Platform.MenuItem {
                text: qsTr("Pause All")
                enabled: backend.connected
                onTriggered: backend.pauseAll()
            }
            Platform.MenuItem {
                text: qsTr("Resume All")
                enabled: backend.connected
                onTriggered: backend.resumeAll()
            }
            Platform.MenuSeparator {}
            Platform.MenuItem {
                text: qsTr("Quit")
                onTriggered: root.quit()
            }
        }
    }
}
