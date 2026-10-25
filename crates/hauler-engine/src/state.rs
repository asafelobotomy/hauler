//! Crash-safe resume state, stored beside the `.part` file.
//!
//! Callers must make the data durable (`sync_data`) before saving a snapshot
//! of `written` positions, so the state never claims bytes that are not on
//! disk. The file is replaced atomically: write a temp file, fsync, rename.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentState {
    pub start: u64,
    pub end: u64,
    pub written: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResumeState {
    pub version: u32,
    pub url: String,
    pub final_url: String,
    pub size: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub segments: Vec<SegmentState>,
}

impl ResumeState {
    pub fn new(url: String, final_url: String, size: u64, etag: Option<String>, last_modified: Option<String>) -> Self {
        Self { version: VERSION, url, final_url, size, etag, last_modified, segments: Vec::new() }
    }

    pub fn load(path: &Path) -> Result<Option<Self>> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let state: Self = serde_json::from_slice(&bytes).map_err(|e| Error::State(e.to_string()))?;
        if state.version != VERSION {
            return Err(Error::State(format!("unsupported version {}", state.version)));
        }
        Ok(Some(state))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("hauler.tmp");
        let json = serde_json::to_vec(self).map_err(|e| Error::State(e.to_string()))?;
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            // Persist the rename itself; not every platform can open a directory.
            if let Ok(d) = fs::File::open(dir) {
                let _ = d.sync_all();
            }
        }
        Ok(())
    }

    /// Whether the segments tile `[0, size)` with every `written` inside its
    /// segment, so they can be resumed. A state with no segments asks for a
    /// fresh start into the same file.
    pub fn is_resumable(&self) -> bool {
        let mut segs: Vec<_> = self.segments.iter().collect();
        segs.sort_by_key(|s| s.start);
        let mut next = 0;
        for s in &segs {
            if s.start != next || s.start >= s.end || !(s.start..=s.end).contains(&s.written) {
                return false;
            }
            next = s.end;
        }
        !segs.is_empty() && next == self.size
    }

    /// Whether this state describes the same remote file as a fresh probe.
    pub fn matches(&self, size: u64, etag: Option<&str>, last_modified: Option<&str>) -> bool {
        if self.size != size {
            return false;
        }
        match (self.etag.as_deref(), etag) {
            (Some(a), Some(b)) => a == b,
            _ => match (self.last_modified.as_deref(), last_modified) {
                (Some(a), Some(b)) => a == b,
                // No validator on either side: size is all we can check.
                _ => self.etag.is_none() && etag.is_none(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_segments_that_tile_the_file_are_resumable() {
        let seg = |start, end, written| SegmentState { start, end, written };
        let state =
            |segments| ResumeState { segments, ..ResumeState::new(String::new(), String::new(), 10, None, None) };
        assert!(state(vec![seg(5, 10, 7), seg(0, 5, 5)]).is_resumable());
        assert!(!state(vec![]).is_resumable(), "nothing to resume");
        assert!(!state(vec![seg(0, 4, 4), seg(5, 10, 5)]).is_resumable(), "a gap");
        assert!(!state(vec![seg(0, 6, 6), seg(5, 10, 5)]).is_resumable(), "an overlap");
        assert!(!state(vec![seg(0, 10, 11)]).is_resumable(), "written past the end");
        assert!(!state(vec![seg(2, 10, 1)]).is_resumable(), "written before the start");
        assert!(!state(vec![seg(0, 8, 8)]).is_resumable(), "short of the size");
    }
}
