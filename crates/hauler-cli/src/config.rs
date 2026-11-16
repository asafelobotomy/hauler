//! `hauler config`: show or change the daemon's settings.
//!
//! `config show [--json]` prints them with what each accepts; `config set
//! <key> <value> [--dry-run] [--json]` changes one, checking the value as the
//! daemon does. The old flag form (`config --max-active 5`) still works.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{ArgAction, Args, Subcommand};
use hauler_ipc::{Settings, SettingsPatch, TaskInfo, TaskState};
use indicatif::HumanBytes;
use serde_json::{Value, json};

use crate::json::{self, Change, Plan};
use crate::parse_size;
use crate::remote::connect;

/// Show or change daemon settings.
#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct ConfigArgs {
    #[command(subcommand)]
    command: Option<ConfigCommand>,
    /// Downloads running at once.
    #[arg(long)]
    max_active: Option<u32>,
    /// Default connections per download.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=32))]
    connections: Option<u16>,
    /// Default download directory.
    #[arg(long)]
    dir: Option<PathBuf>,
    /// Pause while on a metered connection.
    #[arg(long, action = ArgAction::Set)]
    pause_on_metered: Option<bool>,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print every setting.
    Show {
        #[arg(long)]
        json: bool,
    },
    /// Change one setting.
    Set {
        /// One of: max_active, connections, global_limit, download_dir, pause_on_metered, history_days,
        /// scrub_headers, https_only, min_free, scan_downloads.
        key: String,
        value: String,
        /// Print what would change, and change nothing.
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
}

/// Each setting: name, kind, smallest and largest value (for numbers).
const SCHEMA: &[(&str, &str, Option<u64>, Option<u64>)] = &[
    ("max_active", "int", Some(1), None),
    ("connections", "int", Some(1), Some(32)),
    ("global_limit", "size", Some(0), None),
    ("download_dir", "dir", None, None),
    ("pause_on_metered", "bool", None, None),
    ("history_days", "int", Some(0), Some(36_500)),
    ("scrub_headers", "bool", None, None),
    ("https_only", "bool", None, None),
    ("min_free", "size", Some(0), None),
    ("scan_downloads", "bool", None, None),
];

pub async fn run(args: ConfigArgs) -> anyhow::Result<ExitCode> {
    match args.command {
        Some(ConfigCommand::Show { json }) => json::run(json, show(json)).await,
        Some(ConfigCommand::Set { key, value, dry_run, json }) => json::run(json, set(key, value, dry_run, json)).await,
        None => legacy(args.max_active, args.connections, args.dir, args.pause_on_metered).await,
    }
}

fn values(s: &Settings) -> Value {
    json!({
        "max_active": s.max_active,
        "connections": s.connections,
        "global_limit": s.global_limit,
        "download_dir": s.download_dir,
        "pause_on_metered": s.pause_on_metered,
        "history_days": s.history_days,
        "scrub_headers": s.scrub_headers,
        "https_only": s.https_only,
        "min_free": s.min_free,
        "scan_downloads": s.scan_downloads,
    })
}

fn schema() -> Value {
    SCHEMA.iter().map(|(key, kind, min, max)| json!({ "key": key, "kind": kind, "min": min, "max": max })).collect()
}

async fn show(as_json: bool) -> anyhow::Result<ExitCode> {
    let s = connect().await?.settings().await?;
    if as_json {
        return Ok(json::ok(json!({ "settings": values(&s), "schema": schema() })));
    }
    let size = |n: u64| if n == 0 { "none".to_string() } else { format!("{}", HumanBytes(n)) };
    println!("max_active        {}", s.max_active);
    println!("connections       {}", s.connections);
    println!(
        "global_limit      {}",
        if s.global_limit == 0 { "none".into() } else { format!("{}/s", HumanBytes(s.global_limit)) }
    );
    println!("download_dir      {}", s.download_dir);
    println!("pause_on_metered  {}", s.pause_on_metered);
    println!("history_days      {}", if s.history_days == 0 { "forever".into() } else { s.history_days.to_string() });
    println!("scrub_headers     {}", s.scrub_headers);
    println!("https_only        {}", s.https_only);
    println!("min_free          {}", size(s.min_free));
    println!("scan_downloads    {}", s.scan_downloads);
    Ok(ExitCode::SUCCESS)
}

fn parse_bool(text: &str) -> anyhow::Result<bool> {
    match text.to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => bail!("{text:?} is not true or false"),
    }
}

fn whole(text: &str, min: u64, max: Option<u64>) -> anyhow::Result<u64> {
    let n: u64 = text.parse().map_err(|_| anyhow::anyhow!("{text:?} is not a whole number"))?;
    if n < min || max.is_some_and(|m| n > m) {
        match max {
            Some(m) => bail!("must be {min} to {m}"),
            None => bail!("must be at least {min}"),
        }
    }
    Ok(n)
}

/// The patch that sets `key` to `value`, or why it can't.
pub fn patch_for(key: &str, value: &str) -> anyhow::Result<SettingsPatch> {
    let mut p = SettingsPatch::default();
    let (_, _, min, max) =
        *SCHEMA.iter().find(|(k, ..)| *k == key).with_context(|| format!("no setting called {key}"))?;
    let min = min.unwrap_or(0);
    match key {
        "max_active" => p.max_active = Some(whole(value, min, max)? as u32),
        "connections" => p.connections = Some(whole(value, min, max)? as u16),
        "history_days" => p.history_days = Some(whole(value, min, max)? as u32),
        "global_limit" => p.global_limit = Some(parse_size(value).map_err(anyhow::Error::msg)?),
        "min_free" => p.min_free = Some(parse_size(value).map_err(anyhow::Error::msg)?),
        "pause_on_metered" => p.pause_on_metered = Some(parse_bool(value)?),
        "scrub_headers" => p.scrub_headers = Some(parse_bool(value)?),
        "https_only" => p.https_only = Some(parse_bool(value)?),
        "scan_downloads" => p.scan_downloads = Some(parse_bool(value)?),
        "download_dir" => {
            let dir = std::path::absolute(value)?;
            if !dir.is_dir() {
                bail!("{} is not an existing folder", dir.display());
            }
            p.download_dir = Some(dir.to_string_lossy().into_owned());
        }
        _ => unreachable!("checked against SCHEMA"),
    }
    Ok(p)
}

async fn set(key: String, value: String, dry_run: bool, as_json: bool) -> anyhow::Result<ExitCode> {
    let patch = match patch_for(&key, &value) {
        Ok(p) => p,
        Err(e) if dry_run => {
            Plan::refuse(format!("{key}: {e}")).print(as_json);
            return Ok(ExitCode::SUCCESS);
        }
        Err(e) => return Err(e.context(key)),
    };
    let mut client = connect().await?;
    if dry_run {
        let tasks = client.list().await?;
        plan(&key, &patch, &tasks, json::now()).print(as_json);
        return Ok(ExitCode::SUCCESS);
    }
    let s = client.set_settings(&patch).await?;
    if as_json {
        Ok(json::ok(json!({ "settings": values(&s) })))
    } else {
        println!("{key} set");
        Ok(ExitCode::SUCCESS)
    }
}

fn change(group: &str, task: &TaskInfo, detail: String, reversible: Option<bool>) -> Change {
    Change { group: group.into(), label: json::label(task), detail, bytes: None, reversible }
}

/// What setting `key` would do to the downloads that exist, from `now`.
fn plan(key: &str, patch: &SettingsPatch, tasks: &[TaskInfo], now: i64) -> Plan {
    let mut changes = Vec::new();
    let summary: String = match key {
        "history_days" => {
            let days = i64::from(patch.history_days.unwrap_or(0));
            if days > 0 {
                for t in tasks.iter().filter(|t| t.state.is_finished()) {
                    if let Some(at) = t.finished_at.filter(|at| *at < now - days * 86_400) {
                        changes.push(change(
                            "Leave the history",
                            t,
                            format!("finished {}", json::days_ago(now, at)),
                            Some(false),
                        ));
                    }
                }
            }
            match (days, changes.len()) {
                (0, _) => "The history is kept for ever.".into(),
                (_, 0) => format!("Nothing is older than {days} days."),
                (_, n) => format!("{n} download{} leave the history. Their files stay.", if n == 1 { "" } else { "s" }),
            }
        }
        "scrub_headers" if patch.scrub_headers == Some(true) => {
            for t in tasks.iter().filter(|t| t.state == TaskState::Completed && t.has_headers) {
                changes.push(change("Saved cookies and headers dropped", t, json::shown_url(&t.url), Some(false)));
            }
            match changes.len() {
                0 => "No finished download holds saved cookies or headers.".into(),
                n => format!(
                    "{n} finished download{} forget their saved cookies and headers.",
                    if n == 1 { "" } else { "s" }
                ),
            }
        }
        "https_only" if patch.https_only == Some(true) => {
            for t in tasks.iter().filter(|t| !t.state.is_finished() && t.url.starts_with("http://")) {
                changes.push(change(
                    "Would fail to start",
                    t,
                    format!("{} is plain http", json::shown_url(&t.url)),
                    Some(true),
                ));
            }
            match changes.len() {
                0 => "Every waiting download is https.".into(),
                n => format!("{n} waiting download{} use plain http and will fail.", if n == 1 { "" } else { "s" }),
            }
        }
        _ => "Nothing is removed.".into(),
    };
    Plan { summary, changes, would_refuse: false, reason: String::new() }
}

async fn legacy(
    max_active: Option<u32>,
    connections: Option<u16>,
    dir: Option<PathBuf>,
    pause_on_metered: Option<bool>,
) -> anyhow::Result<ExitCode> {
    let patch = SettingsPatch {
        max_active,
        connections,
        download_dir: dir.map(|d| std::path::absolute(d).map(|p| p.to_string_lossy().into_owned())).transpose()?,
        pause_on_metered,
        ..Default::default()
    };
    let any = patch.max_active.is_some()
        || patch.connections.is_some()
        || patch.download_dir.is_some()
        || patch.pause_on_metered.is_some();
    if any {
        connect().await?.set_settings(&patch).await?;
    }
    show(false).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: i64, state: TaskState, finished_days_ago: Option<i64>, url: &str, headers: bool) -> TaskInfo {
        TaskInfo {
            id,
            url: url.into(),
            dir: "/tmp".into(),
            name: Some(format!("f{id}.bin")),
            state,
            size: Some(10),
            received: 10,
            speed: 0,
            connections: 0,
            limit: 0,
            priority: 0,
            resumable: Some(true),
            error: None,
            added_at: 0,
            finished_at: finished_days_ago.map(|d| 1_000_000 - d * 86_400),
            has_headers: headers,
            scan: None,
        }
    }

    #[test]
    fn values_are_checked_the_way_the_daemon_checks_them() {
        assert_eq!(patch_for("max_active", "5").unwrap().max_active, Some(5));
        assert!(patch_for("max_active", "0").is_err());
        assert!(patch_for("connections", "33").is_err());
        assert_eq!(patch_for("min_free", "2G").unwrap().min_free, Some(2 << 30));
        assert_eq!(patch_for("global_limit", "0").unwrap().global_limit, Some(0));
        assert_eq!(patch_for("https_only", "on").unwrap().https_only, Some(true));
        assert!(patch_for("https_only", "maybe").is_err());
        assert!(patch_for("history_days", "36501").is_err());
        assert!(patch_for("history_days", "-1").is_err());
        assert!(patch_for("download_dir", "/definitely/not/here").is_err());
        assert!(patch_for("colour", "red").is_err());
    }

    #[test]
    fn the_schema_covers_every_setting_the_daemon_has() {
        let s = Settings {
            max_active: 3,
            connections: 8,
            global_limit: 0,
            download_dir: "/tmp".into(),
            pause_on_metered: true,
            history_days: 0,
            scrub_headers: true,
            https_only: false,
            min_free: 0,
            scan_downloads: false,
        };
        let shown = values(&s);
        let mut keys: Vec<_> = shown.as_object().unwrap().keys().cloned().collect();
        let mut schema: Vec<_> = SCHEMA.iter().map(|(k, ..)| k.to_string()).collect();
        keys.sort();
        schema.sort();
        assert_eq!(keys, schema);
        assert_eq!(
            serde_json::to_value(&s).unwrap().as_object().unwrap().len(),
            schema.len(),
            "a field was added to Settings"
        );
    }

    #[test]
    fn retention_lists_only_what_would_leave() {
        let tasks = [
            task(1, TaskState::Completed, Some(40), "https://a.example/x?token=secret", false),
            task(2, TaskState::Failed, Some(31), "https://a.example/y", false),
            task(3, TaskState::Completed, Some(2), "https://a.example/z", false),
            task(4, TaskState::Paused, None, "https://a.example/w", false),
        ];
        let patch = patch_for("history_days", "30").unwrap();
        let p = plan("history_days", &patch, &tasks, 1_000_000);
        assert_eq!(p.changes.len(), 2, "{p:?}");
        assert!(p.changes.iter().all(|c| c.reversible == Some(false) && c.group == "Leave the history"));
        assert!(p.summary.contains("2 downloads"));
        let keep = plan("history_days", &patch_for("history_days", "0").unwrap(), &tasks, 1_000_000);
        assert!(keep.changes.is_empty());
    }

    #[test]
    fn a_plan_never_shows_a_secret_in_an_address() {
        let tasks = [TaskInfo {
            name: None,
            ..task(1, TaskState::Completed, Some(1), "https://u:pw@h.example/p?sig=abc#frag", true)
        }];
        let p = plan("scrub_headers", &patch_for("scrub_headers", "true").unwrap(), &tasks, 1_000_000);
        let text = serde_json::to_string(&p).unwrap();
        assert!(text.contains("https://h.example/p") && !text.contains("abc") && !text.contains("pw"), "{text}");
    }

    #[test]
    fn https_only_names_the_waiting_downloads_it_would_break() {
        let tasks = [
            task(1, TaskState::Queued, None, "http://a.example/x", false),
            task(2, TaskState::Queued, None, "https://a.example/y", false),
            task(3, TaskState::Completed, Some(1), "http://a.example/z", false),
        ];
        let p = plan("https_only", &patch_for("https_only", "true").unwrap(), &tasks, 1_000_000);
        assert_eq!(p.changes.len(), 1);
        assert!(plan("https_only", &patch_for("https_only", "false").unwrap(), &tasks, 1_000_000).changes.is_empty());
    }
}
