//! The QML-facing `Backend` object. It is a thin bridge: QML calls
//! invokables, which queue commands to a tokio thread that talks to haulerd;
//! daemon events come back as signals carrying JSON. All state lives in the
//! daemon, so the UI can close and reopen at any time.

use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;
use hauler_ipc::{AddParams, Client, ClientError, Event, Events, SettingsPatch, TaskId};
use tokio::sync::mpsc;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, connected)]
        #[qproperty(QString, settings_json)]
        #[qproperty(QString, network_json)]
        /// Development aid: `$HAULER_DESKTOP_DEMO`, a path prefix for screenshots.
        #[qproperty(QString, demo_prefix)]
        /// http(s) links from the command line, one per line.
        #[qproperty(QString, initial_urls)]
        /// Started with `--minimized`: stay in the tray until asked.
        #[qproperty(bool, start_hidden)]
        type Backend = super::BackendRust;

        /// `params_json` is an `AddParams` object.
        #[qinvokable]
        fn add(self: &Backend, params_json: &QString);
        #[qinvokable]
        fn pause(self: &Backend, id: i64);
        #[qinvokable]
        fn resume(self: &Backend, id: i64);
        #[qinvokable]
        fn remove(self: &Backend, id: i64, delete_files: bool);
        #[qinvokable]
        fn set_limit(self: &Backend, id: i64, bytes_per_sec: f64);
        #[qinvokable]
        fn set_priority(self: &Backend, id: i64, priority: i32);
        #[qinvokable]
        fn pause_all(self: &Backend);
        #[qinvokable]
        fn resume_all(self: &Backend);
        /// `patch_json` is a `SettingsPatch` object.
        #[qinvokable]
        fn save_settings(self: &Backend, patch_json: &QString);
        /// Answered by `segments`.
        #[qinvokable]
        fn request_segments(self: &Backend, id: i64);
        /// Save the window as a PNG (development aid).
        #[qinvokable]
        fn grab_window(self: &Backend, path: &QString) -> bool;

        /// The full task list, after (re)connecting.
        #[qsignal]
        fn tasks_reset(self: Pin<&mut Backend>, json: QString);
        #[qsignal]
        fn task_changed(self: Pin<&mut Backend>, json: QString);
        #[qsignal]
        fn task_removed(self: Pin<&mut Backend>, id: i64);
        #[qsignal]
        fn progress(self: Pin<&mut Backend>, json: QString);
        #[qsignal]
        fn segments(self: Pin<&mut Backend>, id: i64, json: QString);
        #[qsignal]
        fn failed(self: Pin<&mut Backend>, message: QString);
    }

    impl cxx_qt::Threading for Backend {}
    impl cxx_qt::Initialize for Backend {}
}

use qobject::Backend;

#[derive(Default)]
pub struct BackendRust {
    connected: bool,
    settings_json: QString,
    network_json: QString,
    demo_prefix: QString,
    start_hidden: bool,
    initial_urls: QString,
    commands: Option<mpsc::UnboundedSender<Command>>,
}

#[derive(Debug)]
enum Command {
    Add(AddParams),
    Pause(TaskId),
    Resume(TaskId),
    Remove(TaskId, bool),
    Limit(TaskId, u64),
    Priority(TaskId, i32),
    PauseAll,
    ResumeAll,
    Settings(SettingsPatch),
    Segments(TaskId),
}

impl cxx_qt::Initialize for Backend {
    fn initialize(mut self: Pin<&mut Self>) {
        let (tx, rx) = mpsc::unbounded_channel();
        self.as_mut().rust_mut().commands = Some(tx);
        if std::env::args().any(|a| a == "--minimized") {
            self.as_mut().set_start_hidden(true);
        }
        let urls: Vec<String> =
            std::env::args().skip(1).filter(|a| a.starts_with("http://") || a.starts_with("https://")).collect();
        if !urls.is_empty() {
            self.as_mut().set_initial_urls(QString::from(&urls.join("\n")));
        }
        if let Ok(prefix) = std::env::var("HAULER_DESKTOP_DEMO") {
            self.as_mut().set_demo_prefix(QString::from(&prefix));
        }
        let qt = self.qt_thread();
        std::thread::Builder::new()
            .name("hauler-ipc".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                runtime.block_on(worker(qt, rx));
            })
            .expect("spawning the IPC thread");
    }
}

impl Backend {
    fn send(&self, cmd: Command) {
        if let Some(tx) = &self.rust().commands {
            let _ = tx.send(cmd);
        }
    }

    fn add(&self, params_json: &QString) {
        match serde_json::from_str::<AddParams>(&params_json.to_string()) {
            Ok(p) => self.send(Command::Add(p)),
            Err(e) => eprintln!("bad add parameters from QML: {e}"),
        }
    }

    fn pause(&self, id: i64) {
        self.send(Command::Pause(id));
    }

    fn resume(&self, id: i64) {
        self.send(Command::Resume(id));
    }

    fn remove(&self, id: i64, delete_files: bool) {
        self.send(Command::Remove(id, delete_files));
    }

    fn set_limit(&self, id: i64, bytes_per_sec: f64) {
        self.send(Command::Limit(id, bytes_per_sec.max(0.0) as u64));
    }

    fn set_priority(&self, id: i64, priority: i32) {
        self.send(Command::Priority(id, priority));
    }

    fn pause_all(&self) {
        self.send(Command::PauseAll);
    }

    fn resume_all(&self) {
        self.send(Command::ResumeAll);
    }

    fn save_settings(&self, patch_json: &QString) {
        match serde_json::from_str::<SettingsPatch>(&patch_json.to_string()) {
            Ok(p) => self.send(Command::Settings(p)),
            Err(e) => eprintln!("bad settings from QML: {e}"),
        }
    }

    fn request_segments(&self, id: i64) {
        self.send(Command::Segments(id));
    }

    fn grab_window(&self, path: &QString) -> bool {
        crate::app::ffi::hauler_grab_window(&path.to_string())
    }
}

/// Run `f` on the Qt thread against the Backend object. Fails quietly once
/// the object is gone.
fn on_qt(qt: &CxxQtThread<Backend>, f: impl FnOnce(Pin<&mut Backend>) + Send + 'static) {
    let _ = qt.queue(f);
}

fn emit_failed(qt: &CxxQtThread<Backend>, message: String) {
    on_qt(qt, move |b| b.failed(QString::from(&message)));
}

fn json<T: serde::Serialize>(v: &T) -> QString {
    QString::from(&serde_json::to_string(v).unwrap_or_default())
}

async fn worker(qt: CxxQtThread<Backend>, mut commands: mpsc::UnboundedReceiver<Command>) {
    let mut tried_spawn = false;
    loop {
        match connect().await {
            Ok((client, events)) => {
                if !session(&qt, client, events, &mut commands).await {
                    return; // the UI is gone
                }
                on_qt(&qt, |b| b.set_connected(false));
            }
            Err(_) if !tried_spawn => {
                tried_spawn = true;
                spawn_daemon();
            }
            Err(_) => {}
        }
        // Retry, answering commands with an error meanwhile.
        let retry = tokio::time::sleep(Duration::from_secs(1));
        tokio::pin!(retry);
        loop {
            tokio::select! {
                _ = &mut retry => break,
                cmd = commands.recv() => match cmd {
                    None => return,
                    Some(_) => emit_failed(&qt, "Not connected to haulerd".into()),
                },
            }
        }
    }
}

async fn connect() -> Result<(Client, Events), ClientError> {
    let mut client = Client::connect_default().await?;
    let events = Client::connect_default().await?.subscribe().await?;
    // Make sure the request connection works before calling it a session.
    client.status().await?;
    Ok((client, events))
}

/// Start `haulerd` when socket activation is not set up: next to this
/// binary first (a development build), then from `$PATH`.
///
/// Its output goes where ours does, so a failure to start is visible, and a
/// thread waits on it so it never lingers as a zombie if it exits early.
fn spawn_daemon() {
    let sibling = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("haulerd")));
    let program = sibling.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("haulerd"));
    let mut command = std::process::Command::new(&program);
    command.stdin(std::process::Stdio::null());
    // If this app dies without stopping it (a crash, a kill), the kernel
    // sends the daemon SIGTERM so it saves progress and exits instead of
    // running on orphaned, possibly from an unmounted AppImage.
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        use std::os::unix::process::CommandExt;
        let parent = libc::getpid();
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have died before prctl took effect.
            if libc::getppid() != parent {
                libc::raise(libc::SIGTERM);
            }
            Ok(())
        });
    }
    match command.spawn() {
        Ok(mut child) => {
            let pid = child.id();
            let reaper = std::thread::spawn(move || {
                if let Ok(status) = child.wait()
                    && !status.success()
                {
                    eprintln!("haulerd exited: {status}");
                }
            });
            *SPAWNED.lock().unwrap() = Some((pid, reaper));
        }
        Err(e) => eprintln!("could not start {}: {e}", program.display()),
    }
}

/// The daemon this app started, if any: its pid and the thread waiting on it.
static SPAWNED: std::sync::Mutex<Option<(u32, std::thread::JoinHandle<()>)>> = std::sync::Mutex::new(None);

/// Stop the daemon this app started, letting it save its downloads' progress.
/// Run from an AppImage, its binary disappears when the app exits, so it must
/// not outlive us. A daemon from systemd or started elsewhere is left alone.
pub fn stop_spawned_daemon() {
    let Some((pid, reaper)) = SPAWNED.lock().unwrap().take() else { return };
    // SAFETY: plain kill(2) on a pid we spawned and have not reaped yet.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    // haulerd allows downloads up to 10 s to flush; wait a little longer.
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    while !reaper.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Serve one daemon connection. Returns false when the UI has gone away.
async fn session(
    qt: &CxxQtThread<Backend>,
    mut client: Client,
    mut events: Events,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> bool {
    let (Ok(tasks), Ok(settings), Ok(status)) = (client.list().await, client.settings().await, client.status().await)
    else {
        return true;
    };
    let (tasks, settings, network) = (json(&tasks), json(&settings), json(&status.network));
    on_qt(qt, move |mut b| {
        b.as_mut().set_settings_json(settings);
        b.as_mut().set_network_json(network);
        b.as_mut().tasks_reset(tasks);
        b.set_connected(true);
    });

    loop {
        tokio::select! {
            cmd = commands.recv() => {
                let Some(cmd) = cmd else { return false };
                match execute(qt, &mut client, cmd).await {
                    Ok(()) => {}
                    Err(ClientError::Rpc(e)) => emit_failed(qt, e.message),
                    Err(e) => {
                        emit_failed(qt, e.to_string());
                        return true;
                    }
                }
            }
            event = events.next() => match event {
                Ok(Some(e)) => forward(qt, e),
                _ => return true,
            },
        }
    }
}

async fn execute(qt: &CxxQtThread<Backend>, client: &mut Client, cmd: Command) -> Result<(), ClientError> {
    match cmd {
        Command::Add(p) => drop(client.add(&p).await?),
        Command::Pause(id) => drop(client.pause(id).await?),
        Command::Resume(id) => drop(client.resume(id).await?),
        Command::Remove(id, delete) => client.remove(id, delete).await?,
        Command::Limit(id, rate) => drop(client.set_limit(id, rate).await?),
        Command::Priority(id, p) => drop(client.set_priority(id, p).await?),
        Command::PauseAll => drop(client.pause_all().await?),
        Command::ResumeAll => drop(client.resume_all().await?),
        Command::Settings(patch) => drop(client.set_settings(&patch).await?),
        Command::Segments(id) => {
            let map = json(&client.segments(id).await?);
            on_qt(qt, move |b| b.segments(id, map));
        }
    }
    Ok(())
}

fn forward(qt: &CxxQtThread<Backend>, event: Event) {
    match event {
        Event::TaskAdded { task } | Event::TaskUpdated { task } => {
            let j = json(&task);
            on_qt(qt, move |b| b.task_changed(j));
        }
        Event::TaskRemoved { id } => on_qt(qt, move |b| b.task_removed(id)),
        Event::Progress { updates } => {
            let j = json(&updates);
            on_qt(qt, move |b| b.progress(j));
        }
        Event::Settings { settings } => {
            let j = json(&settings);
            on_qt(qt, move |b| b.set_settings_json(j));
        }
        Event::Network { network } => {
            let j = json(&network);
            on_qt(qt, move |b| b.set_network_json(j));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The exact shapes AddDialog.qml and SettingsPage.qml send.
    #[test]
    fn qml_payloads_parse() {
        let add: AddParams = serde_json::from_str(
            r#"{"url":"https://example.com/a.iso","dir":null,"filename":null,"connections":8,"limit":null,"priority":0,"paused":false}"#,
        )
        .unwrap();
        assert_eq!(add.connections, Some(8));
        assert!(add.headers.is_empty());

        let patch: SettingsPatch = serde_json::from_str(
            r#"{"max_active":3,"connections":8,"global_limit":0,"download_dir":"/tmp","pause_on_metered":true}"#,
        )
        .unwrap();
        assert_eq!(patch.max_active, Some(3));
        let held: SettingsPatch = serde_json::from_str(r#"{"pause_on_metered":false}"#).unwrap();
        assert_eq!(held.pause_on_metered, Some(false));
    }
}
