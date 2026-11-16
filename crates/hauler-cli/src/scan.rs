//! `hauler scan`: have oysterAV look at finished downloads.
//!
//! `scan <id>…` or `scan --all` (every completed download whose file is still there) asks the daemon to scan,
//! and answers at once; the verdict shows in `hauler list` and in the windows as it arrives. With `--wait` the
//! command stays until every scan it started has ended, and exits 1 when oysterAV found something. Nothing is
//! moved or deleted: that is for the person to decide, in oysterAV.

use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use hauler_ipc::{ScanState, TaskId, TaskInfo, TaskState};
use serde_json::json;

use crate::json;
use crate::queue::task_file;
use crate::remote::connect;

/// Scan finished downloads with oysterAV.
#[derive(Args)]
pub struct ScanArgs {
    /// Downloads to scan.
    ids: Vec<TaskId>,
    /// Every completed download whose file is still there.
    #[arg(long, conflicts_with = "ids")]
    all: bool,
    /// Stay until the scans have ended; exit 1 when something was found.
    #[arg(long)]
    wait: bool,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

fn summary(task: &TaskInfo) -> serde_json::Value {
    let scan = task.scan.as_ref();
    json!({
        "id": task.id,
        "name": json::label(task),
        "scan": scan.map(|r| r.state.as_str()),
        "detail": scan.map(|r| r.detail.as_str()),
    })
}

pub async fn run(args: ScanArgs) -> anyhow::Result<ExitCode> {
    json::run(args.json, async {
        let mut client = connect().await?;
        let tasks = client.list().await?;
        let chosen: Vec<TaskId> = if args.all {
            tasks
                .iter()
                .filter(|t| t.state == TaskState::Completed && task_file(t).is_some_and(|p| p.exists()))
                .map(|t| t.id)
                .collect()
        } else if args.ids.is_empty() {
            anyhow::bail!("say which downloads to scan: their ids, or --all");
        } else {
            args.ids.clone()
        };
        let mut started = Vec::new();
        let mut refused = Vec::new();
        for id in chosen {
            match client.scan(id).await {
                Ok(_) => started.push(id),
                Err(e) if args.all || args.ids.len() > 1 => refused.push(json!({ "id": id, "reason": e.to_string() })),
                Err(e) => return Err(e.into()),
            }
        }
        let mut threats = false;
        let mut results = Vec::new();
        if args.wait {
            for id in &started {
                loop {
                    let t = client.get(*id).await?;
                    if t.scan.as_ref().is_none_or(|r| r.state != ScanState::Scanning) {
                        threats |= t.scan.as_ref().is_some_and(|r| r.state == ScanState::Threat);
                        results.push(t);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
            }
        }
        if args.json {
            let mut doc = json!({ "started": started, "refused": refused });
            if args.wait {
                doc["results"] = results.iter().map(summary).collect();
            }
            json::ok(doc);
        } else if args.wait {
            for t in &results {
                let r = t.scan.as_ref();
                let verdict = r.map(|r| r.state.as_str()).unwrap_or("not scanned");
                let detail = r.filter(|r| !r.detail.is_empty()).map(|r| format!(": {}", r.detail)).unwrap_or_default();
                println!("{}  {verdict}{detail}", json::label(t));
            }
        } else {
            println!("{} scanning; see `hauler list` for what oysterAV says", started.len());
        }
        for r in &refused {
            eprintln!("download {}: {}", r["id"], r["reason"].as_str().unwrap_or(""));
        }
        Ok(if threats { ExitCode::FAILURE } else { ExitCode::SUCCESS })
    })
    .await
}
