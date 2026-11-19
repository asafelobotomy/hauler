//! What `--json` prints, so a settings page (and any script) can rely on it.
//!
//! A read prints one JSON document with `"ok": true`. A refusal or a failure
//! prints `{"ok": false, "error": "…", "code": "…"}` and exits 1, where `code`
//! is `daemon_down` when `haulerd` can't be reached and `refused` when it
//! said no. A `--dry-run --json` prints one plan document, the shape uQ's
//! confirmation sheet shows: `{summary, changes[{group, label, detail, bytes,
//! reversible}], would_refuse, reason}`.

use std::process::ExitCode;

use hauler_ipc::{ClientError, TaskInfo};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Serialize, Default, Debug)]
pub struct Plan {
    pub summary: String,
    pub changes: Vec<Change>,
    pub would_refuse: bool,
    pub reason: String,
}

#[derive(Serialize, Debug)]
pub struct Change {
    pub group: String,
    pub label: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// `Some(false)`: can't be undone. Left out when it doesn't apply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reversible: Option<bool>,
}

impl Plan {
    pub fn refuse(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Plan { summary: reason.clone(), changes: Vec::new(), would_refuse: true, reason }
    }

    pub fn print(&self, json: bool) {
        if json {
            println!("{}", serde_json::to_string(self).unwrap());
            return;
        }
        println!("{}", self.summary);
        for c in &self.changes {
            let size = c.bytes.map(|b| format!(" ({})", indicatif::HumanBytes(b))).unwrap_or_default();
            println!("  {}: {}{size}  {}", c.group, c.label, c.detail);
        }
    }
}

/// Print `value` as the answer of a successful read.
pub fn ok(mut value: Value) -> ExitCode {
    if let Value::Object(map) = &mut value {
        map.insert("ok".into(), Value::Bool(true));
    }
    println!("{value}");
    ExitCode::SUCCESS
}

pub fn failure(error: &anyhow::Error) -> ExitCode {
    let code = match error.downcast_ref::<ClientError>() {
        Some(ClientError::Connect { .. }) => "daemon_down",
        Some(ClientError::Rpc(_)) => "refused",
        _ => "error",
    };
    let message = match code {
        "daemon_down" => {
            "haulerd isn't running and couldn't be started. Start it with `systemctl --user start hauler.socket`."
                .to_string()
        }
        _ => format!("{error:#}"),
    };
    println!("{}", json!({ "ok": false, "error": message, "code": code }));
    ExitCode::FAILURE
}

/// Run `f`; with `json`, a failure prints the failure document instead of
/// bubbling up to a plain stderr message.
pub async fn run<F>(json: bool, f: F) -> anyhow::Result<ExitCode>
where
    F: std::future::Future<Output = anyhow::Result<ExitCode>>,
{
    match f.await {
        Err(e) if json => Ok(failure(&e)),
        other => other,
    }
}

/// What to call a download: its file name, else its address.
pub fn label(task: &TaskInfo) -> String {
    task.name.clone().unwrap_or_else(|| shown_url(&task.url))
}

/// An address without what may be secret in it: no login, query or fragment.
pub fn shown_url(url: &str) -> String {
    match hauler_engine::Url::parse(url) {
        Ok(u) => {
            let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
            format!("{}://{}{port}{}", u.scheme(), u.host_str().unwrap_or(""), u.path())
        }
        Err(_) => "(invalid address)".into(),
    }
}

pub fn days_ago(now: i64, then: i64) -> String {
    let days = (now - then).max(0) / 86_400;
    match days {
        0 => "today".into(),
        1 => "1 day ago".into(),
        n => format!("{n} days ago"),
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
