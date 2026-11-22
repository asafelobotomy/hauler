//! What the daemon keeps about finished downloads: how long the list is
//! kept, and the request headers (cookies, tokens) saved with each one.

use hauler_ipc::{Event, TaskState};
use tracing::warn;

use super::{Entry, Manager, now};

const DAY: i64 = 24 * 60 * 60;

impl Manager {
    /// Forget finished and failed downloads older than `history_days`. Only
    /// the list entry goes: the files stay where they are.
    pub(super) fn prune_history(&self) {
        let removed: Vec<_> = {
            let mut inner = self.inner.lock().unwrap();
            let days = i64::from(inner.settings.history_days);
            if days == 0 {
                return;
            }
            let cutoff = now() - days * DAY;
            let ids: Vec<_> = inner
                .tasks
                .values()
                .filter(|e| e.run.is_none() && e.info.state.is_finished())
                .filter(|e| e.info.finished_at.is_some_and(|t| t < cutoff))
                .map(|e| e.info.id)
                .collect();
            for id in &ids {
                inner.tasks.remove(id);
            }
            ids
        };
        for id in removed {
            if let Err(e) = self.store.delete(id) {
                warn!("[HAULER-003] could not forget download {id}: {e}");
            }
            self.emit(Event::TaskRemoved { id });
        }
    }

    /// Drop the saved headers of one entry, in memory and in the database.
    pub(super) fn scrub(&self, entry: &mut Entry) {
        if entry.spec.headers.is_empty() {
            return;
        }
        entry.spec.headers.clear();
        entry.info.has_headers = false;
        if let Err(e) = self.store.clear_headers(entry.info.id) {
            warn!("[HAULER-003] could not drop the saved headers of download {}: {e}", entry.info.id);
        }
    }

    /// Drop the saved headers of every completed download; answers how many
    /// held any. Unfinished and failed ones keep theirs, since a retry needs
    /// them.
    pub fn forget_headers(&self) -> u32 {
        let mut inner = self.inner.lock().unwrap();
        let mut changed = Vec::new();
        for entry in inner.tasks.values_mut() {
            if entry.info.state == TaskState::Completed && entry.run.is_none() && entry.info.has_headers {
                self.scrub(entry);
                changed.push(entry.info.clone());
            }
        }
        drop(inner);
        let n = changed.len() as u32;
        for task in changed {
            self.emit(Event::TaskUpdated { task });
        }
        n
    }
}
