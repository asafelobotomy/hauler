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
