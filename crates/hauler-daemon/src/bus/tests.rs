//! Progress aggregation, queue order and the provider's URL offer.

use std::collections::HashMap;

use hauler_ipc::{TaskInfo, TaskState};

use super::hauler1::{QUEUE_MAX, queue_order};
use super::provider::{LauncherProvider, MAX_ID_BYTES, offer};
use super::{Totals, aggregate, newly_finished};
use crate::manager::Manager;
use crate::store::Store;

fn task(id: i64, state: TaskState, received: u64, size: Option<u64>) -> TaskInfo {
    TaskInfo {
        id,
        url: format!("https://example.com/{id}.bin"),
        dir: "/tmp".into(),
        name: Some(format!("{id}.bin")),
        state,
        size,
        received,
        speed: 0,
        connections: 0,
        limit: 0,
        priority: 0,
        resumable: None,
        error: None,
        added_at: 0,
        finished_at: None,
    }
}

#[test]
fn nothing_to_do_is_all_zero() {
    assert_eq!(aggregate(&[]), Totals::default());
    let done = [task(1, TaskState::Completed, 10, Some(10)), task(2, TaskState::Failed, 3, Some(9))];
    assert_eq!(aggregate(&done), Totals::default());
}

#[test]
fn running_and_queued_downloads_add_up() {
    let tasks = [
        task(1, TaskState::Running, 100, Some(1000)),
        task(2, TaskState::Running, 50, Some(500)),
        task(3, TaskState::Queued, 0, Some(200)),
        task(4, TaskState::Waiting, 20, Some(40)),
        // Left out: paused and finished downloads.
        task(5, TaskState::Paused, 7, Some(70)),
        task(6, TaskState::Completed, 9, Some(9)),
    ];
    assert_eq!(aggregate(&tasks), Totals { active: 2, queued: 2, bytes_done: 170, bytes_total: 1740 });
}

#[test]
fn an_unknown_size_makes_the_total_unknown() {
    let tasks = [task(1, TaskState::Running, 100, Some(1000)), task(2, TaskState::Queued, 0, None)];
    assert_eq!(aggregate(&tasks), Totals { active: 1, queued: 1, bytes_done: 100, bytes_total: 0 });
    // A server that says 0 bytes is no better than none.
    let tasks = [task(1, TaskState::Running, 5, Some(0))];
    assert_eq!(aggregate(&tasks).bytes_total, 0);
}

#[test]
fn sums_saturate() {
    let tasks = [task(1, TaskState::Running, u64::MAX, Some(u64::MAX)), task(2, TaskState::Running, 1, Some(1))];
    let t = aggregate(&tasks);
    assert_eq!((t.bytes_done, t.bytes_total), (u64::MAX, u64::MAX));
}

#[test]
fn only_changed_properties_are_sent() {
    let a = Totals { active: 1, queued: 2, bytes_done: 3, bytes_total: 4 };
    assert!(a.changes(&a).is_empty());
    let b = Totals { bytes_done: 30, ..a };
    let changed = b.changes(&a);
    assert_eq!(changed.keys().copied().collect::<Vec<_>>(), ["BytesDone"]);
    let all = Totals::default().changes(&a);
    let mut keys: Vec<_> = all.keys().copied().collect();
    keys.sort();
    assert_eq!(keys, ["ActiveCount", "BytesDone", "BytesTotal", "QueuedCount"]);
}

#[test]
fn finished_is_reported_once_per_transition() {
    let mut states = HashMap::new();
    assert!(newly_finished(&mut states, &[task(1, TaskState::Running, 0, None)]).is_empty());
    let done = newly_finished(&mut states, &[task(1, TaskState::Completed, 9, Some(9))]);
    assert_eq!(done.iter().map(|t| t.id).collect::<Vec<_>>(), [1]);
    // Another update of a finished download is not a second finish.
    assert!(newly_finished(&mut states, &[task(1, TaskState::Completed, 9, Some(9))]).is_empty());
    // A download first seen finished (loaded from the database) never ran here.
    assert!(newly_finished(&mut states, &[task(2, TaskState::Failed, 0, None)]).is_empty());
    // Retried and failed again: a new finish.
    newly_finished(&mut states, &[task(2, TaskState::Queued, 0, None)]);
    assert_eq!(newly_finished(&mut states, &[task(2, TaskState::Failed, 0, None)]).len(), 1);
}

#[test]
fn the_queue_lists_unfinished_downloads_in_order() {
    let mut high = task(5, TaskState::Queued, 0, None);
    high.priority = 3;
    let tasks = vec![
        task(1, TaskState::Completed, 0, None),
        task(2, TaskState::Paused, 0, None),
        task(3, TaskState::Queued, 0, None),
        task(4, TaskState::Running, 0, None),
        high,
        task(6, TaskState::Failed, 0, None),
        task(7, TaskState::Waiting, 0, None),
    ];
    let ids: Vec<_> = queue_order(tasks).iter().map(|t| t.id).collect();
    assert_eq!(ids, [4, 7, 5, 3, 2, 6]);
    let many: Vec<_> = (0..200).map(|i| task(i, TaskState::Queued, 0, None)).collect();
    assert_eq!(queue_order(many).len(), QUEUE_MAX);
}

#[test]
fn only_a_whole_http_url_is_offered() {
    assert_eq!(offer("https://example.com/a.iso").as_deref(), Some("https://example.com/a.iso"));
    assert_eq!(offer("  HTTP://Example.com  ").as_deref(), Some("http://example.com/"));
    for q in [
        "",
        "   ",
        "firefox",
        "example.com/a.iso",
        "ftp://example.com/a.iso",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "https://",
        "get https://example.com/a.iso",
        "https://example.com/a b.iso",
    ] {
        assert_eq!(offer(q), None, "{q:?}");
    }
}

#[tokio::test]
async fn long_urls_get_a_token_that_activates_them() {
    let m = Manager::new(Store::in_memory().unwrap(), &std::env::temp_dir()).unwrap();
    let p = LauncherProvider::new(m.clone());
    let long = format!("https://example.com/{}", "a".repeat(MAX_ID_BYTES));
    let rows = p.search(long.clone());
    assert_eq!(rows.len(), 1);
    let id = rows[0].0.clone();
    assert!(id.len() <= MAX_ID_BYTES && id != long, "{id}");
    assert_eq!(p.search(long.clone())[0].0, id, "the same URL keeps its token");
    p.activate(id, "download".into()).unwrap();
    assert_eq!(m.list().iter().map(|t| t.url.clone()).collect::<Vec<_>>(), [long]);
    assert!(p.activate("hauler-url:999".into(), String::new()).is_err());
    assert!(p.activate("https://example.com/".into(), "open".into()).is_err());
}
