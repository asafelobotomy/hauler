//! Scanning finished downloads with oysterAV, after they complete (when the setting is on) or on request.

use std::path::PathBuf;
use std::sync::Arc;

use hauler_ipc::{RpcError, ScanReport, ScanState, TaskId, TaskInfo, TaskState};
use tracing::info;

use super::{Manager, now};

impl Manager {
    /// After a download ended: scan its file when `scan_downloads` is on and it completed.
    pub(super) fn scan_if_enabled(self: &Arc<Self>, id: TaskId) {
        let wanted = {
            let inner = self.inner.lock().unwrap();
            inner.settings.scan_downloads && inner.tasks.get(&id).is_some_and(|e| e.info.state == TaskState::Completed)
        };
        if wanted {
            let _ = self.scan(id);
        }
    }

    /// Scan a completed download's file now; answers the task, marked as being scanned. The answer
    /// arrives as a task update. A scan already under way is left to finish.
    pub fn scan(self: &Arc<Self>, id: TaskId) -> Result<TaskInfo, RpcError> {
        let (info, path) = {
            let mut inner = self.inner.lock().unwrap();
            let entry = inner.tasks.get_mut(&id).ok_or_else(|| RpcError::not_found(id))?;
            if entry.info.state != TaskState::Completed {
                return Err(RpcError::new(RpcError::INVALID_STATE, "only a completed download can be scanned"));
            }
            let name = entry.info.name.clone().filter(|n| super::is_plain_name(n));
            let Some(name) = name else {
                return Err(RpcError::new(RpcError::INVALID_STATE, "this download has no file to scan"));
            };
            if entry.info.scan.as_ref().is_some_and(|r| r.state == ScanState::Scanning) {
                return Ok(entry.info.clone());
            }
            entry.info.scan = Some(ScanReport { state: ScanState::Scanning, detail: String::new(), at: now() });
            let info = entry.info.clone();
            let path = PathBuf::from(&entry.info.dir).join(name);
            self.changed(&inner, id);
            (info, path)
        };
        let m = self.clone();
        tokio::spawn(async move {
            let report = crate::scan::scan_file(&path).await;
            info!("scan of download {id}: {}", report.state.as_str());
            m.finish_scan(id, report);
        });
        Ok(info)
    }

    /// Keep what oysterAV said, unless the download is gone or a newer scan has started.
    pub(super) fn finish_scan(&self, id: TaskId, report: ScanReport) {
        let mut inner = self.inner.lock().unwrap();
        let Some(entry) = inner.tasks.get_mut(&id) else { return };
        entry.info.scan = Some(report);
        self.changed(&inner, id);
    }
}
