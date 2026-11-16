//! `remove` and `clear`: take downloads off the list, with their files if
//! asked, after a `--dry-run --json` plan of exactly what goes.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, ValueEnum};
use hauler_ipc::{TaskId, TaskInfo, TaskState};
use indicatif::HumanBytes;
use serde_json::json;

use crate::json::{self, Change, Plan};
use crate::queue::task_file;
use crate::remote::connect;

/// Remove downloads from the list.
#[derive(Args)]
pub struct RemoveArgs {
    #[arg(required = true)]
    ids: Vec<TaskId>,
    /// Also delete the downloaded and partial files.
    #[arg(long)]
    delete: bool,
    /// Print what would be removed, and remove nothing.
    #[arg(long)]
    dry_run: bool,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

pub async fn remove(args: RemoveArgs) -> anyhow::Result<ExitCode> {
    json::run(args.json, async {
        let mut client = connect().await?;
        let all = client.list().await?;
        let mut chosen = Vec::new();
        for id in &args.ids {
            match all.iter().find(|t| t.id == *id) {
                Some(t) => chosen.push(t.clone()),
                None if args.dry_run => return refuse(args.json, format!("no download with id {id}")),
                None => return Err(hauler_ipc::RpcError::not_found(*id).into()),
            }
        }
        finish(&mut client, chosen, args.delete, args.dry_run, args.json, "Removed").await
    })
    .await
}

fn refuse(as_json: bool, reason: String) -> anyhow::Result<ExitCode> {
    Plan::refuse(reason).print(as_json);
    Ok(ExitCode::SUCCESS)
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Which {
    /// Completed and failed downloads.
    Finished,
    Completed,
    Failed,
}

/// Remove finished downloads from the list in one go.
#[derive(Args)]
pub struct ClearArgs {
    #[arg(long, value_enum, default_value = "finished")]
    state: Which,
    /// Only those that finished more than this many days ago.
    #[arg(long, value_name = "DAYS")]
    older_than: Option<u32>,
    /// Also delete the downloaded files.
    #[arg(long)]
    delete_files: bool,
    /// Print what would be removed, and remove nothing.
    #[arg(long)]
    dry_run: bool,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

pub async fn clear(args: ClearArgs) -> anyhow::Result<ExitCode> {
    json::run(args.json, async {
        let mut client = connect().await?;
        let cutoff = args.older_than.map(|d| json::now() - i64::from(d) * 86_400);
        let chosen: Vec<_> = client
            .list()
            .await?
            .into_iter()
            .filter(|t| match args.state {
                Which::Finished => t.state.is_finished(),
                Which::Completed => t.state == TaskState::Completed,
                Which::Failed => t.state == TaskState::Failed,
            })
            .filter(|t| cutoff.is_none_or(|c| t.finished_at.is_some_and(|at| at < c)))
            .collect();
        finish(&mut client, chosen, args.delete_files, args.dry_run, args.json, "Cleared").await
    })
    .await
}

/// The files the daemon deletes with a removed download, and their sizes:
/// the finished file, or an unfinished download's own `.part` and `.hauler`.
fn doomed_files(task: &TaskInfo) -> Vec<(PathBuf, u64)> {
    let suffixes: &[&str] = match task.state {
        TaskState::Completed => &[""],
        _ if task.resumable.is_some() => &[".part", ".hauler"],
        _ => &[],
    };
    let Some(base) = task_file(task) else { return Vec::new() };
    suffixes
        .iter()
        .filter_map(|s| {
            let mut p = base.clone().into_os_string();
            p.push(s);
            let p = PathBuf::from(p);
            std::fs::metadata(&p).ok().filter(|m| m.is_file()).map(|m| (p, m.len()))
        })
        .collect()
}

fn plan_removal(tasks: &[TaskInfo], delete: bool, now: i64) -> Plan {
    let mut changes = Vec::new();
    let mut bytes = 0u64;
    let mut files = 0usize;
    for t in tasks {
        let mine = if delete { doomed_files(t) } else { Vec::new() };
        let size: u64 = mine.iter().map(|(_, n)| n).sum();
        bytes += size;
        files += mine.len();
        let when = t
            .finished_at
            .map(|at| format!("finished {}", json::days_ago(now, at)))
            .unwrap_or_else(|| t.state.as_str().into());
        let detail = match (delete, mine.is_empty()) {
            (true, false) => {
                format!("{when}; deletes {}", task_file(t).map(|p| p.display().to_string()).unwrap_or_default())
            }
            (true, true) => format!("{when}; no file to delete"),
            (false, _) => when,
        };
        changes.push(Change {
            group: if delete { "Removed with their files" } else { "Removed from the list" }.into(),
            label: json::label(t),
            detail,
            bytes: (size > 0).then_some(size),
            // A history entry can't be put back, and neither can a file.
            reversible: Some(false),
        });
    }
    let n = tasks.len();
    let summary = match (n, delete) {
        (0, _) => "Nothing to remove.".to_string(),
        (_, true) => {
            format!("{n} download{} removed, {files} file{} deleted ({}).", plural(n), plural(files), HumanBytes(bytes))
        }
        (_, false) => format!("{n} download{} leave the list. Files stay.", plural(n)),
    };
    Plan { summary, changes, would_refuse: false, reason: String::new() }
}

pub(crate) fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

async fn finish(
    client: &mut hauler_ipc::Client,
    tasks: Vec<TaskInfo>,
    delete: bool,
    dry_run: bool,
    as_json: bool,
    verb: &str,
) -> anyhow::Result<ExitCode> {
    if dry_run {
        plan_removal(&tasks, delete, json::now()).print(as_json);
        return Ok(ExitCode::SUCCESS);
    }
    let mut bytes = 0u64;
    let mut files = 0usize;
    for t in &tasks {
        if delete {
            let mine = doomed_files(t);
            bytes += mine.iter().map(|(_, n)| n).sum::<u64>();
            files += mine.len();
        }
        client.remove(t.id, delete).await?;
    }
    if as_json {
        Ok(json::ok(json!({ "removed": tasks.len(), "deleted_files": files, "bytes": bytes })))
    } else {
        println!("{verb} {}", tasks.len());
        Ok(ExitCode::SUCCESS)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use std::path::Path;

    pub(crate) fn task(dir: &Path, name: &str, state: TaskState) -> TaskInfo {
        TaskInfo {
            id: 1,
            url: "https://h.example/f".into(),
            dir: dir.to_string_lossy().into_owned(),
            name: Some(name.into()),
            state,
            size: Some(5),
            received: 5,
            speed: 0,
            connections: 0,
            limit: 0,
            priority: 0,
            resumable: Some(true),
            error: None,
            added_at: 0,
            finished_at: Some(0),
            has_headers: false,
            scan: None,
        }
    }

    #[test]
    fn a_removal_plan_counts_only_files_the_daemon_would_delete() {
        let dir = std::env::temp_dir().join(format!("hauler-cli-queue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("done.bin"), b"12345").unwrap();
        std::fs::write(dir.join("half.bin.part"), b"123").unwrap();
        std::fs::write(dir.join("half.bin.hauler"), b"{}").unwrap();
        std::fs::write(dir.join("other.bin"), b"keep").unwrap();
        let tasks = [task(&dir, "done.bin", TaskState::Completed), task(&dir, "half.bin", TaskState::Paused)];
        let keep = plan_removal(&tasks, false, 100);
        assert!(keep.changes.iter().all(|c| c.bytes.is_none()), "no files without --delete");
        let gone = plan_removal(&tasks, true, 100);
        assert_eq!(gone.changes.iter().filter_map(|c| c.bytes).sum::<u64>(), 5 + 3 + 2);
        assert!(gone.summary.contains("3 files"), "{}", gone.summary);
        // A name with directory parts never reaches outside the folder.
        assert!(doomed_files(&task(&dir, "../other.bin", TaskState::Completed)).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
