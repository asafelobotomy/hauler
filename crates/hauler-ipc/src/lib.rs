//! The local API between `haulerd` and its clients (CLI, desktop UI, browser
//! host): JSON-RPC 2.0, one JSON object per line, over a Unix socket.
//!
//! The socket lives in `$XDG_RUNTIME_DIR/hauler/`, which only the user can
//! open, so file permissions are the authentication.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod client;

pub use client::{Client, ClientError, Events};

pub type TaskId = i64;

/// Where the daemon listens: `$HAULER_SOCKET`, else
/// `$XDG_RUNTIME_DIR/hauler/hauler.sock`, else `/tmp/hauler-<uid>/hauler.sock`.
pub fn default_socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("HAULER_SOCKET") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("hauler-{}", uid())));
    base.join("hauler").join("hauler.sock")
}

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// The socket's directory must belong to this user and be writable by no one
/// else; otherwise another user could swap the socket for their own and read
/// every URL and cookie sent to it. Matters most for the `/tmp` fallback.
pub fn check_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != uid() || meta.mode() & 0o022 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} must be a directory you own that others cannot write to", dir.display()),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Data types

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Waiting for a free slot.
    Queued,
    Running,
    /// Paused by the user.
    Paused,
    /// Stopped because the network is down or metered; resumes by itself.
    Waiting,
    Completed,
    Failed,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskState::Queued => "queued",
            TaskState::Running => "running",
            TaskState::Paused => "paused",
            TaskState::Waiting => "waiting",
            TaskState::Completed => "completed",
            TaskState::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => TaskState::Queued,
            "running" => TaskState::Running,
            "paused" => TaskState::Paused,
            "waiting" => TaskState::Waiting,
            "completed" => TaskState::Completed,
            "failed" => TaskState::Failed,
            _ => return None,
        })
    }

    pub fn is_finished(self) -> bool {
        matches!(self, TaskState::Completed | TaskState::Failed)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: TaskId,
    pub url: String,
    pub dir: String,
    /// File name once known (after the first probe).
    pub name: Option<String>,
    pub state: TaskState,
    pub size: Option<u64>,
    pub received: u64,
    /// Bytes per second, smoothed.
    pub speed: u64,
    pub connections: u32,
    /// Per-task limit in bytes per second; 0 means none.
    pub limit: u64,
    pub priority: i32,
    /// Whether a pause keeps progress. Unknown until the first probe.
    pub resumable: Option<bool>,
    pub error: Option<String>,
    /// Unix seconds.
    pub added_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AddParams {
    pub url: String,
    /// Defaults to the daemon's download directory.
    pub dir: Option<String>,
    pub filename: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// `sha256:<hex>`
    pub checksum: Option<String>,
    pub connections: Option<u16>,
    pub limit: Option<u64>,
    pub priority: Option<i32>,
    /// Add without starting.
    #[serde(default)]
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Downloads running at once.
    pub max_active: u32,
    /// Default connections per download.
    pub connections: u16,
    /// Limit across all downloads in bytes per second; 0 means none.
    pub global_limit: u64,
    pub download_dir: String,
    /// Pause downloads while the connection is metered.
    pub pause_on_metered: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SettingsPatch {
    pub max_active: Option<u32>,
    pub connections: Option<u16>,
    pub global_limit: Option<u64>,
    pub download_dir: Option<String>,
    pub pause_on_metered: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkState {
    pub online: bool,
    pub metered: bool,
    /// Downloads may run.
    pub open: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
    pub version: String,
    pub running: u32,
    pub queued: u32,
    /// Bytes per second across all downloads.
    pub speed: u64,
    pub network: NetworkState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProgressUpdate {
    pub id: TaskId,
    pub received: u64,
    pub speed: u64,
    pub connections: u32,
}

/// One piece of the file, for drawing a progress map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    /// Bytes done, counted from `start`'s offset: the segment is complete
    /// when `done == end`.
    pub done: u64,
    /// A connection is working on it right now.
    pub active: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentMap {
    pub size: Option<u64>,
    /// In file order; empty when nothing is known yet.
    pub segments: Vec<Segment>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    TaskAdded {
        task: TaskInfo,
    },
    TaskUpdated {
        task: TaskInfo,
    },
    TaskRemoved {
        id: TaskId,
    },
    /// Sent about once a second while anything is running.
    Progress {
        updates: Vec<ProgressUpdate>,
    },
    Settings {
        settings: Settings,
    },
    Network {
        network: NetworkState,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdParams {
    pub id: TaskId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoveParams {
    pub id: TaskId,
    #[serde(default)]
    pub delete_files: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LimitParams {
    pub id: TaskId,
    pub bytes_per_sec: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PriorityParams {
    pub id: TaskId,
    pub priority: i32,
}

/// Method names.
pub mod method {
    pub const ADD: &str = "add";
    pub const LIST: &str = "list";
    pub const GET: &str = "get";
    pub const PAUSE: &str = "pause";
    pub const RESUME: &str = "resume";
    pub const REMOVE: &str = "remove";
    pub const SET_LIMIT: &str = "set_limit";
    pub const SET_PRIORITY: &str = "set_priority";
    pub const PAUSE_ALL: &str = "pause_all";
    pub const RESUME_ALL: &str = "resume_all";
    pub const GET_SETTINGS: &str = "get_settings";
    pub const SET_SETTINGS: &str = "set_settings";
    pub const STATUS: &str = "status";
    pub const SEGMENTS: &str = "segments";
    /// After the reply, the connection carries `event` notifications only.
    pub const SUBSCRIBE: &str = "subscribe";
    pub const EVENT: &str = "event";
}

// ---------------------------------------------------------------------------
// JSON-RPC envelopes

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    /// Absent for notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message} (code {code})")]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub const PARSE: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL: i64 = -32603;
    pub const NOT_FOUND: i64 = 1;
    pub const INVALID_STATE: i64 = 2;

    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub fn not_found(id: TaskId) -> Self {
        Self::new(Self::NOT_FOUND, format!("no download with id {id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_dir_must_be_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("hauler-ipc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(check_private_dir(&dir).is_ok());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(check_private_dir(&dir).is_err());
        // Someone else's directory is refused too (unless we are root, as in
        // CI containers, where / is ours).
        if uid() != 0 {
            assert!(check_private_dir(std::path::Path::new("/")).is_err());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn events_are_tagged() {
        let json = serde_json::to_value(Event::TaskRemoved { id: 7 }).unwrap();
        assert_eq!(json, serde_json::json!({"kind": "task_removed", "id": 7}));
    }

    #[test]
    fn states_round_trip() {
        for s in [TaskState::Queued, TaskState::Running, TaskState::Paused, TaskState::Waiting, TaskState::Completed] {
            assert_eq!(TaskState::parse(s.as_str()), Some(s));
        }
    }
}
