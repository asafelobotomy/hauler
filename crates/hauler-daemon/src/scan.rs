//! Scanning a finished download with oysterAV (`oyst-cli scan <file> --ndjson`).
//!
//! The contract (see oysterAV's `docs/cli/gui-contract.md`, "Apps that save files"): progress lines
//! `{"event":"progress",…}`, then one `{"event":"result","clean":…,"findings":[{"threat_name":…}],
//! "pack_errors":[{"error":…}]}`; exit 0 clean, 1 threats found, 3 another oysterAV job is running
//! (ask again later), anything else a failure. Nothing is moved or deleted: quarantining a file is
//! the person's choice, made in oysterAV.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hauler_ipc::{ScanReport, ScanState};
use serde_json::Value;
use tokio::process::Command;

const DEFAULT_CLI: &str = "/usr/bin/oyst-cli";
/// oysterAV runs one job at a time; when it is busy, ask again this often, this many times.
const BUSY_DELAY: Duration = Duration::from_secs(10);
const BUSY_TRIES: u32 = 30;

/// `$HAULER_OYST_CLI` (for tests and odd installs), else `/usr/bin/oyst-cli`.
pub fn oyst_cli() -> PathBuf {
    std::env::var_os("HAULER_OYST_CLI").map(PathBuf::from).unwrap_or_else(|| DEFAULT_CLI.into())
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn report(state: ScanState, detail: impl Into<String>) -> ScanReport {
    ScanReport { state, detail: detail.into(), at: now() }
}

/// What one `oyst-cli` run said.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Done(ScanState, String),
    /// Another oysterAV job is running.
    Busy,
}

/// Reads the result line (the last `event: result`) and the exit code.
fn read_answer(code: Option<i32>, stdout: &str, stderr: &str) -> Answer {
    if code == Some(3) {
        return Answer::Busy;
    }
    let result: Option<Value> = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .rfind(|v| v.get("event").and_then(Value::as_str) == Some("result"));
    let Some(result) = result else {
        let why = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("oysterAV gave no result");
        return Answer::Done(ScanState::Error, why.trim().chars().take(300).collect());
    };
    let mut threats: Vec<&str> = Vec::new();
    for f in result.get("findings").and_then(Value::as_array).into_iter().flatten() {
        let name = f.get("threat_name").and_then(Value::as_str).unwrap_or("unknown threat");
        if !threats.contains(&name) {
            threats.push(name);
        }
    }
    if !threats.is_empty() {
        return Answer::Done(ScanState::Threat, threats.join(", "));
    }
    let problem = result
        .get("pack_errors")
        .and_then(Value::as_array)
        .and_then(|e| e.first())
        .and_then(|e| e.get("error").and_then(Value::as_str));
    match (problem, result.get("clean").and_then(Value::as_bool), code) {
        (Some(p), _, _) => Answer::Done(ScanState::Error, p.chars().take(300).collect()),
        (None, Some(true), Some(0)) => Answer::Done(ScanState::Clean, String::new()),
        _ => Answer::Done(ScanState::Error, "oysterAV could not finish the scan".into()),
    }
}

/// Scan one file. Waits (up to about five minutes) while oysterAV is busy with another job.
pub async fn scan_file(path: &Path) -> ScanReport {
    scan_with(&oyst_cli(), path, BUSY_DELAY, BUSY_TRIES).await
}

pub async fn scan_with(cli: &Path, path: &Path, busy_delay: Duration, tries: u32) -> ScanReport {
    if !path.is_absolute() {
        return report(ScanState::Error, "the file's place is not known");
    }
    if !path.is_file() {
        return report(ScanState::Error, "the file is no longer there");
    }
    if !cli.exists() {
        return report(ScanState::Error, "oysterAV is not installed (oyst-cli not found)");
    }
    for attempt in 0..tries.max(1) {
        let run = Command::new(cli)
            .args(["scan", "--ndjson"])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await;
        match run {
            Err(e) => return report(ScanState::Error, format!("could not start oysterAV: {e}")),
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                match read_answer(out.status.code(), &stdout, &stderr) {
                    Answer::Done(state, detail) => return report(state, detail),
                    Answer::Busy if attempt + 1 < tries.max(1) => tokio::time::sleep(busy_delay).await,
                    Answer::Busy => {}
                }
            }
        }
    }
    report(ScanState::Error, "oysterAV stayed busy with another job")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: &str = "{\"event\":\"progress\",\"stage\":\"clamav\",\"percent\":100.0}\n{\"event\":\"result\",\"clean\":true,\"findings\":[],\"pack_errors\":[]}\n";

    #[test]
    fn clean_threat_error_and_busy() {
        assert_eq!(read_answer(Some(0), CLEAN, ""), Answer::Done(ScanState::Clean, String::new()));
        let threat = "{\"event\":\"result\",\"clean\":false,\"findings\":[{\"threat_name\":\"Win.Test\"},{\"threat_name\":\"Win.Test\"},{\"threat_name\":\"Other\"}],\"pack_errors\":[]}\n";
        assert_eq!(read_answer(Some(1), threat, ""), Answer::Done(ScanState::Threat, "Win.Test, Other".into()));
        let broken = "{\"event\":\"result\",\"clean\":false,\"findings\":[],\"pack_errors\":[{\"pack\":\"clamav\",\"error\":\"no signatures\"}]}\n";
        assert_eq!(read_answer(Some(2), broken, ""), Answer::Done(ScanState::Error, "no signatures".into()));
        assert_eq!(read_answer(Some(3), "", "busy"), Answer::Busy);
        assert_eq!(read_answer(Some(2), "", "\nboom\n"), Answer::Done(ScanState::Error, "boom".into()));
        assert_eq!(
            read_answer(Some(0), "not json\n", ""),
            Answer::Done(ScanState::Error, "oysterAV gave no result".into())
        );
        // A clean result with a failing exit code is not a clean file.
        assert!(matches!(read_answer(Some(2), CLEAN, ""), Answer::Done(ScanState::Error, _)));
    }

    #[tokio::test]
    async fn missing_scanner_and_file_are_errors_not_panics() {
        let r = scan_with(Path::new("/nonexistent/oyst-cli"), Path::new("/etc/hostname"), Duration::ZERO, 1).await;
        assert_eq!(r.state, ScanState::Error);
        assert!(r.detail.contains("not installed"), "{r:?}");
        let r = scan_with(Path::new("/bin/true"), Path::new("/nonexistent/file"), Duration::ZERO, 1).await;
        assert!(r.detail.contains("no longer there"), "{r:?}");
        let r = scan_with(Path::new("/bin/true"), Path::new("relative"), Duration::ZERO, 1).await;
        assert_eq!(r.state, ScanState::Error);
    }
}
