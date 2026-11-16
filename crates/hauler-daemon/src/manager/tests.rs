//! State transitions that are hard to time through a real download.

use std::sync::Arc;
use std::time::Duration;

use hauler_engine::{CancellationToken, Error};
use hauler_ipc::{AddParams, TaskId, TaskState};

use super::schedule::{MAX_TURNED_AWAY, turned_away_delay};
use super::{Manager, Running};
use crate::store::Store;

fn manager() -> (Arc<Manager>, TaskId) {
    let m = Manager::new(Store::in_memory().unwrap(), &std::env::temp_dir()).unwrap();
    let url = "http://127.0.0.1:9/file.bin".into();
    let id = m.add(AddParams { url, paused: true, ..Default::default() }).unwrap().id;
    (m, id)
}

/// Make `id` look like the engine is running it, in `state`, about to come
/// to rest as `after_stop`.
fn pretend_running(m: &Manager, id: TaskId, state: TaskState, after_stop: TaskState) {
    let mut inner = m.inner.lock().unwrap();
    let e = inner.tasks.get_mut(&id).unwrap();
    e.info.state = state;
    e.run =
        Some(Running { cancel: CancellationToken::new(), progress: None, after_stop, handle: None, last_received: 0 });
}

#[tokio::test]
async fn a_turned_away_download_waits_in_the_queue_then_fails() {
    let (m, id) = manager();
    let busy = || Err(Error::Refused("http://127.0.0.1:9/busy.html".into()));
    for n in 1..=MAX_TURNED_AWAY {
        pretend_running(&m, id, TaskState::Running, TaskState::Queued);
        m.on_finished(id, busy());
        let t = m.get(id).unwrap();
        assert_eq!(t.state, TaskState::Queued, "attempt {n}");
        assert!(t.error.as_deref().is_some_and(|e| e.contains("busy.html")), "{t:?}");
        // Not started again until its time comes.
        m.schedule();
        assert!(m.inner.lock().unwrap().tasks[&id].run.is_none(), "restarted at once");
    }
    pretend_running(&m, id, TaskState::Running, TaskState::Queued);
    m.on_finished(id, busy());
    assert_eq!(m.get(id).unwrap().state, TaskState::Failed);

    // Retrying by hand starts afresh.
    m.resume(id).unwrap();
    let inner = m.inner.lock().unwrap();
    assert_eq!((inner.tasks[&id].turned_away, inner.tasks[&id].retry_at), (0, None));
}

#[test]
fn turned_away_delays_back_off_to_five_minutes() {
    let secs: Vec<_> = (1..=7).map(|n| turned_away_delay(n).as_secs()).collect();
    assert_eq!(secs, [30, 60, 120, 240, 300, 300, 300]);
    assert!(turned_away_delay(u32::MAX) <= Duration::from_secs(300));
}

#[tokio::test]
async fn pausing_while_winding_down_stays_paused() {
    for state in [TaskState::Queued, TaskState::Waiting] {
        let (m, id) = manager();
        // Stopped for the network, or resumed while a pause was settling.
        pretend_running(&m, id, state, state);
        m.pause(id).unwrap();
        m.on_finished(id, Ok(hauler_engine::Outcome::Paused));
        assert_eq!(m.get(id).unwrap().state, TaskState::Paused, "from {state:?}");
    }
}

#[test]
fn only_http_and_https_urls_are_accepted() {
    use super::check_url;
    assert_eq!(check_url("https://example.com/a.iso").unwrap().as_str(), "https://example.com/a.iso");
    assert_eq!(check_url("HTTP://example.com").unwrap().as_str(), "http://example.com/");
    for bad in ["", "example.com/a.iso", "https://", "ftp://example.com/a", "file:///etc/passwd", "data:,x"] {
        assert!(check_url(bad).is_err(), "{bad:?}");
    }
    assert!(check_url("ftp://example.com/a").unwrap_err().contains("only http and https"));
    // The RPC `add` (what the CLI sends) refuses the same way.
    let (m, _) = manager();
    let err = m.add(AddParams { url: "ftp://example.com/a".into(), ..Default::default() }).unwrap_err();
    assert_eq!(err.code, hauler_ipc::RpcError::INVALID_PARAMS);
}

#[test]
fn only_completed_downloads_forget_their_headers() {
    let m = Manager::new(Store::in_memory().unwrap(), &std::env::temp_dir()).unwrap();
    let add = |name: &str| {
        let url = format!("http://127.0.0.1:9/{name}");
        let headers = vec![("Cookie".to_string(), "a=b".to_string())];
        m.add(AddParams { url, headers, paused: true, ..Default::default() }).unwrap().id
    };
    let (done, waiting) = (add("done.bin"), add("waiting.bin"));
    {
        let mut inner = m.inner.lock().unwrap();
        inner.tasks.get_mut(&done).unwrap().info.state = TaskState::Completed;
    }
    assert!(m.get(done).unwrap().has_headers);
    assert_eq!(m.forget_headers(), 1);
    assert!(!m.get(done).unwrap().has_headers);
    assert!(m.get(waiting).unwrap().has_headers, "a download still to finish keeps its cookie");
    assert_eq!(m.forget_headers(), 0);
    // The database forgot too: a restart does not bring them back.
    let rows = m.store.load_all().unwrap();
    let row = |id| rows.iter().find(|(i, _)| i.id == id).unwrap();
    assert!(row(done).1.headers.is_empty());
    assert_eq!(row(waiting).1.headers.len(), 1);
}

#[test]
fn old_finished_downloads_leave_the_list_but_recent_and_unfinished_stay() {
    let m = Manager::new(Store::in_memory().unwrap(), &std::env::temp_dir()).unwrap();
    let add = |name: &str, state: TaskState, age_days: i64| {
        let url = format!("http://127.0.0.1:9/{name}");
        let id = m.add(AddParams { url, paused: true, ..Default::default() }).unwrap().id;
        let mut inner = m.inner.lock().unwrap();
        let e = inner.tasks.get_mut(&id).unwrap();
        e.info.state = state;
        e.info.finished_at = state.is_finished().then(|| super::now() - age_days * 86_400);
        id
    };
    let old = add("old.bin", TaskState::Completed, 40);
    let old_failed = add("old-failed.bin", TaskState::Failed, 31);
    let recent = add("recent.bin", TaskState::Completed, 2);
    let paused = add("paused.bin", TaskState::Paused, 0);
    m.prune_history();
    assert_eq!(m.list().len(), 4, "0 keeps everything");
    m.set_settings(hauler_ipc::SettingsPatch { history_days: Some(30), ..Default::default() }).unwrap();
    let left: Vec<_> = m.list().iter().map(|t| t.id).collect();
    assert_eq!(left, vec![recent, paused]);
    assert!(m.get(old).is_err() && m.get(old_failed).is_err());
    assert!(m.set_settings(hauler_ipc::SettingsPatch { history_days: Some(40_000), ..Default::default() }).is_err());
}

#[test]
fn https_only_refuses_plain_http_when_adding() {
    let m = Manager::new(Store::in_memory().unwrap(), &std::env::temp_dir()).unwrap();
    m.set_settings(hauler_ipc::SettingsPatch { https_only: Some(true), ..Default::default() }).unwrap();
    let plain = m.add(AddParams { url: "http://example.com/a.bin".into(), paused: true, ..Default::default() });
    assert!(plain.unwrap_err().message.contains("https"));
    assert!(m.add(AddParams { url: "https://example.com/a.bin".into(), paused: true, ..Default::default() }).is_ok());
}
