//! Where downloads go and what they take: `storage` reads the numbers,
//! `cleanup` removes unfinished downloads that nothing is working on.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use hauler_engine::{Partial, free_space, partials};
use hauler_ipc::{TaskInfo, TaskState};
use indicatif::HumanBytes;
use serde_json::json;

use crate::json::{self, Change, Plan};
use crate::remote::connect;

/// Show the download folder, its free space and what hauler keeps there.
#[derive(Args)]
pub struct StorageArgs {
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

/// Unfinished download data nothing in the queue owns: left by `hauler get`
/// or by a download removed without deleting its files.
fn leftovers(dirs: &BTreeSet<PathBuf>, tasks: &[TaskInfo]) -> Vec<(PathBuf, Partial)> {
    let owned =
        |dir: &Path, name: &str| tasks.iter().any(|t| Path::new(&t.dir) == dir && t.name.as_deref() == Some(name));
    let mut out = Vec::new();
    for dir in dirs {
        for p in partials(dir).unwrap_or_default() {
            if !owned(dir, &p.name) {
                out.push((dir.clone(), p));
            }
        }
    }
    out
}

/// The download folder, and every folder a download of the queue lives in.
fn folders(tasks: &[TaskInfo], download_dir: &str) -> BTreeSet<PathBuf> {
    let mut dirs: BTreeSet<_> = tasks.iter().map(|t| PathBuf::from(&t.dir)).collect();
    dirs.insert(PathBuf::from(download_dir));
    dirs.retain(|d| d.is_dir());
    dirs
}

fn file_bytes(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub async fn storage(args: StorageArgs) -> anyhow::Result<ExitCode> {
    json::run(args.json, async {
        let mut client = connect().await?;
        let settings = client.settings().await?;
        let tasks = client.list().await?;
        let dir = Path::new(&settings.download_dir);
        let dir_ok = dir.is_dir();
        let free = free_space(dir).ok();
        let total = rustix::fs::statvfs(dir).ok().map(|s| s.f_blocks.saturating_mul(s.f_frsize));

        // What the queue's own downloads take.
        let finished: Vec<_> = tasks.iter().filter(|t| t.state == TaskState::Completed).collect();
        let finished_bytes: u64 = finished
            .iter()
            .filter_map(|t| t.name.as_deref().map(|n| Path::new(&t.dir).join(n)))
            .map(|p| file_bytes(&p))
            .sum();
        let mut partial_count = 0u64;
        let mut partial_bytes = 0u64;
        for t in tasks.iter().filter(|t| !t.state.is_finished()) {
            if let Some(n) = t.name.as_deref() {
                let part = Path::new(&t.dir).join(format!("{n}.part"));
                if part.exists() {
                    partial_count += 1;
                    partial_bytes += file_bytes(&part);
                }
            }
        }
        let loose = leftovers(&folders(&tasks, &settings.download_dir), &tasks);
        let loose_bytes: u64 = loose.iter().map(|(_, p)| p.bytes).sum();
        if args.json {
            let shown: Vec<_> =
                loose.iter().take(50).map(|(dir, p)| json!({ "name": p.name, "dir": dir, "bytes": p.bytes })).collect();
            return Ok(json::ok(json!({
                "dir": settings.download_dir,
                "dir_ok": dir_ok,
                "free_bytes": free,
                "total_bytes": total,
                "min_free": settings.min_free,
                "finished": { "count": finished.len(), "bytes": finished_bytes },
                "unfinished": { "count": partial_count, "bytes": partial_bytes },
                "leftovers": { "count": loose.len(), "bytes": loose_bytes, "files": shown },
            })));
        }
        println!("folder     {}{}", settings.download_dir, if dir_ok { "" } else { "  (missing)" });
        if let (Some(free), Some(total)) = (free, total) {
            println!("free       {} of {}", HumanBytes(free), HumanBytes(total));
        }
        println!("finished   {} files, {}", finished.len(), HumanBytes(finished_bytes));
        println!("unfinished {} downloads, {}", partial_count, HumanBytes(partial_bytes));
        println!("leftovers  {} unfinished, {}", loose.len(), HumanBytes(loose_bytes));
        Ok(ExitCode::SUCCESS)
    })
    .await
}

/// Delete unfinished downloads nothing is working on and the queue does not own.
#[derive(Args)]
pub struct CleanupArgs {
    /// Print what would be deleted, and delete nothing.
    #[arg(long)]
    dry_run: bool,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

pub async fn cleanup(args: CleanupArgs) -> anyhow::Result<ExitCode> {
    json::run(args.json, async {
        let mut client = connect().await?;
        let settings = client.settings().await?;
        let tasks = client.list().await?;
        let loose = leftovers(&folders(&tasks, &settings.download_dir), &tasks);
        if args.dry_run {
            let changes: Vec<_> = loose
                .iter()
                .map(|(dir, p)| Change {
                    group: "Unfinished downloads nothing is working on".into(),
                    label: p.name.clone(),
                    detail: dir.display().to_string(),
                    bytes: Some(p.bytes),
                    reversible: Some(false),
                })
                .collect();
            let bytes: u64 = loose.iter().map(|(_, p)| p.bytes).sum();
            let summary = match loose.len() {
                0 => "Nothing to clean up.".into(),
                n => format!(
                    "{n} unfinished download{} deleted, {} freed.",
                    if n == 1 { "" } else { "s" },
                    HumanBytes(bytes)
                ),
            };
            Plan { summary, changes, would_refuse: false, reason: String::new() }.print(args.json);
            return Ok(ExitCode::SUCCESS);
        }
        let (mut removed, mut bytes) = (0u64, 0u64);
        for (_, p) in &loose {
            // Only the pair `partials` found: the data and its state file.
            let sized = p.bytes;
            let a = std::fs::remove_file(&p.part);
            let b = std::fs::remove_file(&p.state);
            if a.is_ok() || b.is_ok() {
                removed += 1;
                bytes += sized;
            }
        }
        if args.json {
            Ok(json::ok(json!({ "removed": removed, "bytes": bytes })))
        } else {
            println!("deleted {removed} unfinished downloads ({})", HumanBytes(bytes));
            Ok(ExitCode::SUCCESS)
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queues_own_unfinished_downloads_are_not_leftovers() {
        let dir = std::env::temp_dir().join(format!("hauler-cli-storage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state =
            r#"{"version":1,"url":"u","final_url":"u","size":4,"etag":null,"last_modified":null,"segments":[]}"#;
        for name in ["mine.bin", "stray.bin"] {
            std::fs::write(dir.join(format!("{name}.part")), b"1234").unwrap();
            std::fs::write(dir.join(format!("{name}.hauler")), state).unwrap();
        }
        let task = TaskInfo {
            id: 1,
            url: "https://h.example/mine.bin".into(),
            dir: dir.to_string_lossy().into_owned(),
            name: Some("mine.bin".into()),
            state: TaskState::Paused,
            size: Some(4),
            received: 2,
            speed: 0,
            connections: 0,
            limit: 0,
            priority: 0,
            resumable: Some(true),
            error: None,
            added_at: 0,
            finished_at: None,
            has_headers: false,
            scan: None,
        };
        let dirs = folders(std::slice::from_ref(&task), "/not/a/folder");
        assert_eq!(dirs.len(), 1, "a missing folder is skipped");
        let loose = leftovers(&dirs, std::slice::from_ref(&task));
        assert_eq!(loose.len(), 1);
        assert_eq!(loose[0].1.name, "stray.bin");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
