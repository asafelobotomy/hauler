//! Session-bus integration. On Plasma every running download appears in the
//! notification area's job tracker, with pause, resume and cancel wired back
//! to the queue. Where there is no job tracker (GNOME and others), finished
//! and failed downloads raise a notification instead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::StreamExt;
use hauler_engine::Url;
use hauler_ipc::{Event, TaskId, TaskInfo, TaskState};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::manager::Manager;

/// The `.desktop` file Plasma uses for the job's icon and app name.
pub const DESKTOP_ENTRY: &str = "hauler";

/// KJob capabilities.
const KILLABLE: i32 = 1;
const SUSPENDABLE: i32 = 2;

#[zbus::proxy(
    interface = "org.kde.JobViewServerV2",
    default_service = "org.kde.JobViewServer",
    default_path = "/JobViewServer"
)]
trait JobViewServer {
    #[zbus(name = "requestView")]
    fn request_view(
        &self,
        desktop_entry: &str,
        capabilities: i32,
        hints: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;
}

/// Plasma implements most setters as one-way calls that never reply.
#[zbus::proxy(interface = "org.kde.JobViewV2", default_service = "org.kde.JobViewServer")]
trait JobView {
    #[zbus(name = "terminate", no_reply)]
    fn terminate(&self, error_message: &str) -> zbus::Result<()>;
    #[zbus(name = "setSuspended", no_reply)]
    fn set_suspended(&self, suspended: bool) -> zbus::Result<()>;
    #[zbus(name = "setTotalAmount", no_reply)]
    fn set_total_amount(&self, amount: u64, unit: &str) -> zbus::Result<()>;
    #[zbus(name = "setProcessedAmount", no_reply)]
    fn set_processed_amount(&self, amount: u64, unit: &str) -> zbus::Result<()>;
    #[zbus(name = "setPercent", no_reply)]
    fn set_percent(&self, percent: u32) -> zbus::Result<()>;
    #[zbus(name = "setSpeed", no_reply)]
    fn set_speed(&self, bytes_per_second: u64) -> zbus::Result<()>;
    #[zbus(name = "setInfoMessage", no_reply)]
    fn set_info_message(&self, message: &str) -> zbus::Result<()>;
    #[zbus(name = "setDescriptionField")]
    fn set_description_field(&self, number: u32, name: &str, value: &str) -> zbus::Result<bool>;
    #[zbus(name = "setDestUrl", no_reply)]
    fn set_dest_url(&self, url: &Value<'_>) -> zbus::Result<()>;
    #[zbus(signal, name = "cancelRequested")]
    fn cancel_requested(&self) -> zbus::Result<()>;
    #[zbus(signal, name = "suspendRequested")]
    fn suspend_requested(&self) -> zbus::Result<()>;
    #[zbus(signal, name = "resumeRequested")]
    fn resume_requested(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.FileManager1",
    default_service = "org.freedesktop.FileManager1",
    default_path = "/org/freedesktop/FileManager1"
)]
trait FileManager {
    fn show_items(&self, uris: &[&str], startup_id: &str) -> zbus::Result<()>;
}

struct View {
    proxy: JobViewProxy<'static>,
    signals: JoinHandle<()>,
    suspended: bool,
    described: Option<String>,
}

fn target(task: &TaskInfo) -> Option<PathBuf> {
    task.name.as_ref().map(|n| Path::new(&task.dir).join(n))
}

fn file_url(path: &Path) -> String {
    Url::from_file_path(path).map(|u| u.to_string()).unwrap_or_else(|_| path.display().to_string())
}

pub async fn run(manager: Arc<Manager>, shutdown: CancellationToken) {
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => return info!("no session bus, desktop integration off: {e}"),
    };
    let mut events = manager.subscribe();
    let tracker = match job_tracker(&conn).await {
        Some(t) => {
            info!("reporting downloads to the Plasma job tracker");
            Some(t)
        }
        None => {
            info!("no job tracker; using notifications");
            None
        }
    };
    let notifier = NotificationsProxy::new(&conn).await.ok();
    let mut opened: HashMap<u32, PathBuf> = HashMap::new();
    let mut actions = match &notifier {
        Some(n) => n.receive_action_invoked().await.ok(),
        None => None,
    };

    let mut tasks: HashMap<TaskId, TaskInfo> = manager.list().into_iter().map(|t| (t.id, t)).collect();
    let mut views: HashMap<TaskId, View> = HashMap::new();
    for task in tasks.values() {
        if let Some(t) = &tracker
            && task.state == TaskState::Running
        {
            open_view(&conn, t, &manager, task, &mut views).await;
        }
    }

    loop {
        let event = tokio::select! {
            _ = shutdown.cancelled() => break,
            Some(signal) = async { actions.as_mut()?.next().await } => {
                if let Ok(args) = signal.args()
                    && let Some(path) = opened.get(&args.id)
                {
                    open_path(&conn, path, &args.action_key).await;
                }
                continue;
            }
            e = events.recv() => match e {
                Ok(e) => e,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
        };
        match event {
            Event::TaskAdded { task } | Event::TaskUpdated { task } => {
                let before = tasks.insert(task.id, task.clone()).map(|t| t.state);
                if let Some(t) = &tracker {
                    update_view(&conn, t, &manager, &task, &mut views).await;
                } else if before != Some(task.state)
                    && let Some(n) = &notifier
                    && let Some((id, path)) = notify_finished(n, &task).await
                {
                    opened.insert(id, path);
                }
            }
            Event::TaskRemoved { id } => {
                tasks.remove(&id);
                if let Some(v) = views.remove(&id) {
                    close_view(v, "").await;
                }
            }
            Event::Progress { updates } => {
                for u in updates {
                    let Some(v) = views.get(&u.id) else { continue };
                    let size = tasks.get(&u.id).and_then(|t| t.size);
                    if let Some(size) = size.filter(|s| *s > 0) {
                        let _ = v.proxy.set_percent((u.received * 100 / size) as u32).await;
                    }
                    let _ = v.proxy.set_processed_amount(u.received, "bytes").await;
                    let _ = v.proxy.set_speed(u.speed).await;
                }
            }
            Event::Settings { .. } | Event::Network { .. } => {}
        }
    }
    for (_, v) in views.drain() {
        close_view(v, "").await;
    }
}

/// Plasma's job tracker, if this session has one.
async fn job_tracker(conn: &zbus::Connection) -> Option<JobViewServerProxy<'static>> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = "org.kde.JobViewServer".try_into().ok()?;
    if !dbus.name_has_owner(name).await.ok()? {
        return None;
    }
    JobViewServerProxy::new(conn).await.ok()
}

async fn open_view(
    conn: &zbus::Connection,
    tracker: &JobViewServerProxy<'static>,
    manager: &Arc<Manager>,
    task: &TaskInfo,
    views: &mut HashMap<TaskId, View>,
) {
    let path = match tracker.request_view(DESKTOP_ENTRY, KILLABLE | SUSPENDABLE, HashMap::new()).await {
        Ok(p) => p,
        Err(e) => return debug!("job tracker refused a view: {e}"),
    };
    let proxy = match JobViewProxy::builder(conn).path(path).map(|b| b.build()) {
        Ok(fut) => match fut.await {
            Ok(p) => p,
            Err(e) => return debug!("job view proxy: {e}"),
        },
        Err(e) => return debug!("job view path: {e}"),
    };
    let signals = tokio::spawn(forward_controls(proxy.clone(), manager.clone(), task.id));
    views.insert(task.id, View { proxy, signals, suspended: false, described: None });
}

/// Map the job tracker's buttons onto the queue.
async fn forward_controls(proxy: JobViewProxy<'static>, manager: Arc<Manager>, id: TaskId) {
    let (Ok(mut cancel), Ok(mut suspend), Ok(mut resume)) = (
        proxy.receive_cancel_requested().await,
        proxy.receive_suspend_requested().await,
        proxy.receive_resume_requested().await,
    ) else {
        return;
    };
    loop {
        tokio::select! {
            Some(_) = cancel.next() => {
                // Cancel means stop and discard the partial download.
                let _ = manager.remove(id, true).await;
                return;
            }
            Some(_) = suspend.next() => { let _ = manager.pause(id); }
            Some(_) = resume.next() => { let _ = manager.resume(id); }
            else => return,
        }
    }
}

async fn update_view(
    conn: &zbus::Connection,
    tracker: &JobViewServerProxy<'static>,
    manager: &Arc<Manager>,
    task: &TaskInfo,
    views: &mut HashMap<TaskId, View>,
) {
    if task.state == TaskState::Running && !views.contains_key(&task.id) {
        open_view(conn, tracker, manager, task, views).await;
    }
    let Some(view) = views.get_mut(&task.id) else { return };
    let p = &view.proxy;

    if let Some(path) = target(task)
        && view.described.as_deref() != task.name.as_deref()
    {
        let name = task.name.clone().unwrap_or_default();
        let _ = p.set_info_message(&format!("Downloading {name}")).await;
        let _ = p.set_description_field(0, "Source", &task.url).await;
        let _ = p.set_description_field(1, "Destination", &path.display().to_string()).await;
        let _ = p.set_dest_url(&Value::from(file_url(&path))).await;
        if let Some(size) = task.size {
            let _ = p.set_total_amount(size, "bytes").await;
        }
        view.described = task.name.clone();
    }

    match task.state {
        TaskState::Running | TaskState::Paused | TaskState::Waiting => {
            let suspended = task.state != TaskState::Running;
            if suspended != view.suspended {
                let _ = p.set_suspended(suspended).await;
                view.suspended = suspended;
            }
        }
        TaskState::Completed => {
            let _ = p.set_processed_amount(task.received, "bytes").await;
            let _ = p.set_percent(100).await;
            let view = views.remove(&task.id).unwrap();
            close_view(view, "").await;
        }
        TaskState::Failed => {
            let error = task.error.clone().unwrap_or_else(|| "Download failed".into());
            let view = views.remove(&task.id).unwrap();
            close_view(view, &error).await;
        }
        // Back in the queue (daemon stopping): the next run opens a new view.
        TaskState::Queued => {
            let view = views.remove(&task.id).unwrap();
            close_view(view, "").await;
        }
    }
}

async fn close_view(view: View, error: &str) {
    view.signals.abort();
    let _ = view.proxy.terminate(error).await;
}

/// Notify on completion or failure; returns the notification id and the file
/// its actions refer to.
async fn notify_finished(n: &NotificationsProxy<'_>, task: &TaskInfo) -> Option<(u32, PathBuf)> {
    let name = task.name.clone().unwrap_or_else(|| task.url.clone());
    let path = target(task)?;
    let (summary, body, actions): (_, _, &[&str]) = match task.state {
        TaskState::Completed => {
            ("Download finished", name, &["default", "Open", "open", "Open", "folder", "Show in Folder"])
        }
        TaskState::Failed => {
            ("Download failed", format!("{name}: {}", task.error.as_deref().unwrap_or("unknown error")), &[])
        }
        _ => return None,
    };
    let icon = if task.state == TaskState::Completed { "folder-download" } else { "dialog-error" };
    let id = n.notify("Hauler", 0, icon, summary, &body, actions, HashMap::new(), -1).await.ok()?;
    Some((id, path))
}

async fn open_path(conn: &zbus::Connection, path: &Path, action: &str) {
    if action == "folder" {
        if let Ok(fm) = FileManagerProxy::new(conn).await
            && fm.show_items(&[&file_url(path)], "").await.is_ok()
        {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    } else {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}
