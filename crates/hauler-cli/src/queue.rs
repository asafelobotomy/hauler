//! Reading and tidying the list: `list`, `status`, `remove`, `clear` and
//! `forget-headers`, each with `--json`, and a `--dry-run --json` plan for
//! every one that deletes something.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use hauler_ipc::{TaskInfo, TaskState};
use indicatif::HumanBytes;
use serde::Serialize;
use serde_json::json;

use crate::json::{self, Change, Plan};
use crate::remote::{connect, print_table};

/// List downloads.
#[derive(Args)]
pub struct ListArgs {
    /// Hide completed downloads.
    #[arg(short, long, conflicts_with = "finished")]
    active: bool,
    /// Only completed and failed downloads: the history.
    #[arg(short, long)]
    finished: bool,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Serialize)]
struct Row {
    #[serde(flatten)]
    task: TaskInfo,
    /// The address without its login, query or fragment: safe to show.
    shown_url: String,
    /// Where a completed download's file is, and whether it is still there.
    path: Option<String>,
    file_exists: bool,
}

fn row(task: TaskInfo) -> Row {
    let path = (task.state == TaskState::Completed).then(|| task_file(&task)).flatten();
    let file_exists = path.as_deref().is_some_and(Path::exists);
    Row {
        shown_url: json::shown_url(&task.url),
        path: path.map(|p| p.to_string_lossy().into_owned()),
        file_exists,
        task,
    }
}

/// A plain file name only, so joining it to the folder stays in it.
fn plain(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!((parts.next(), parts.next()), (Some(std::path::Component::Normal(_)), None))
}

pub(crate) fn task_file(task: &TaskInfo) -> Option<PathBuf> {
    task.name.as_deref().filter(|n| plain(n)).map(|n| Path::new(&task.dir).join(n))
}

pub async fn list(args: ListArgs) -> anyhow::Result<ExitCode> {
    json::run(args.json, async {
        let mut tasks = connect().await?.list().await?;
        if args.active {
            tasks.retain(|t| t.state != TaskState::Completed);
        }
        if args.finished {
            tasks.retain(|t| t.state.is_finished());
        }
        if args.json {
            let rows: Vec<_> = tasks.into_iter().map(row).collect();
            return Ok(json::ok(json!({ "tasks": rows })));
        }
        print_table(&tasks);
        Ok(ExitCode::SUCCESS)
    })
    .await
}

/// Show what the daemon is doing.
#[derive(Args)]
pub struct StatusArgs {
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

pub async fn status(args: StatusArgs) -> anyhow::Result<ExitCode> {
    json::run(args.json, async {
        let mut client = connect().await?;
        let s = client.status().await?;
        let tasks = client.list().await?;
        if args.json {
            let count = |states: &[TaskState]| tasks.iter().filter(|t| states.contains(&t.state)).count();
            return Ok(json::ok(json!({
                "version": s.version,
                "running": s.running,
                "queued": s.queued,
                "speed": s.speed,
                "network": s.network,
                "counts": {
                    "total": tasks.len(),
                    "running": count(&[TaskState::Running]),
                    "queued": count(&[TaskState::Queued, TaskState::Waiting]),
                    "paused": count(&[TaskState::Paused]),
                    "completed": count(&[TaskState::Completed]),
                    "failed": count(&[TaskState::Failed]),
                },
            })));
        }
        println!("haulerd {}", s.version);
        println!("running {}, queued {}, {}/s", s.running, s.queued, HumanBytes(s.speed));
        let net = match (s.network.online, s.network.metered, s.network.open) {
            (false, _, _) => "offline, downloads held",
            (true, true, false) => "metered, downloads held",
            (true, true, true) => "metered, downloading anyway",
            _ => "online",
        };
        println!("network {net}");
        Ok(ExitCode::SUCCESS)
    })
    .await
}

/// Drop the saved cookies and headers of completed downloads.
#[derive(Args)]
pub struct ForgetArgs {
    /// Print what would be dropped, and drop nothing.
    #[arg(long)]
    dry_run: bool,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

pub async fn forget_headers(args: ForgetArgs) -> anyhow::Result<ExitCode> {
    json::run(args.json, async {
        let mut client = connect().await?;
        if args.dry_run {
            let tasks = client.list().await?;
            let holding: Vec<_> = tasks.iter().filter(|t| t.state == TaskState::Completed && t.has_headers).collect();
            let changes: Vec<_> = holding
                .iter()
                .map(|t| Change {
                    group: "Saved cookies and headers dropped".into(),
                    label: json::label(t),
                    detail: json::shown_url(&t.url),
                    bytes: None,
                    reversible: Some(false),
                })
                .collect();
            let n = changes.len();
            let summary = match n {
                0 => "No finished download holds saved cookies or headers.".into(),
                _ => format!("{n} finished download{} forget their saved cookies and headers.", crate::tidy::plural(n)),
            };
            Plan { summary, changes, would_refuse: false, reason: String::new() }.print(args.json);
            return Ok(ExitCode::SUCCESS);
        }
        let n = client.forget_headers().await?;
        if args.json {
            Ok(json::ok(json!({ "forgotten": n })))
        } else {
            println!("dropped the saved headers of {n} downloads");
            Ok(ExitCode::SUCCESS)
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(dir: &Path, name: &str, state: TaskState) -> TaskInfo {
        crate::tidy::tests::task(dir, name, state)
    }

    #[test]
    fn rows_carry_a_safe_address_and_where_the_file_is() {
        let mut t = task(Path::new("/nonexistent"), "a.bin", TaskState::Completed);
        t.url = "https://u:p@h.example:8443/a.bin?token=secret".into();
        let r = serde_json::to_value(row(t)).unwrap();
        assert_eq!(r["shown_url"], "https://h.example:8443/a.bin");
        assert_eq!(r["path"], "/nonexistent/a.bin");
        assert_eq!(r["file_exists"], false);
        assert_eq!(r["state"], "completed");
    }
}
