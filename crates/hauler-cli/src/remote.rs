//! Subcommands that talk to `haulerd` over its socket.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{ArgAction, Args};
use hauler_ipc::{AddParams, Client, Event, SettingsPatch, TaskId, TaskInfo, TaskState};
use indicatif::HumanBytes;

use crate::{parse_header, parse_size};

async fn connect() -> anyhow::Result<Client> {
    let path = hauler_ipc::default_socket_path();
    Client::connect(&path).await.with_context(|| {
        format!(
            "is haulerd running? Start it with `systemctl --user start hauler.socket` or run `haulerd` ({})",
            path.display()
        )
    })
}

/// Queue a download in the daemon.
#[derive(Args)]
pub struct AddArgs {
    /// URLs to download.
    #[arg(required = true)]
    urls: Vec<String>,
    /// Directory to save into (default: the daemon's download directory).
    #[arg(short = 'd', long)]
    dir: Option<PathBuf>,
    /// Save under this file name instead of the server's.
    #[arg(short = 'o', long)]
    output: Option<String>,
    /// Parallel connections.
    #[arg(short = 'x', long, value_parser = clap::value_parser!(u16).range(1..=32))]
    connections: Option<u16>,
    /// Speed limit in bytes per second, e.g. 2M.
    #[arg(long, value_parser = parse_size)]
    limit: Option<u64>,
    /// Higher runs first.
    #[arg(short, long, allow_negative_numbers = true)]
    priority: Option<i32>,
    /// Verify the result, e.g. sha256:<hex>.
    #[arg(long)]
    checksum: Option<String>,
    /// Extra request header, e.g. -H 'Cookie: a=b'. Repeatable.
    #[arg(short = 'H', long = "header", value_parser = parse_header)]
    headers: Vec<(hauler_engine::header::HeaderName, hauler_engine::header::HeaderValue)>,
    /// Add without starting.
    #[arg(long)]
    paused: bool,
}

pub async fn add(args: AddArgs) -> anyhow::Result<ExitCode> {
    if args.output.is_some() && args.urls.len() > 1 {
        anyhow::bail!("--output only works with a single URL");
    }
    let dir = match args.dir {
        Some(d) => Some(std::path::absolute(&d)?.to_string_lossy().into_owned()),
        None => None,
    };
    let headers: Vec<_> =
        args.headers.iter().map(|(n, v)| (n.to_string(), v.to_str().unwrap_or_default().to_owned())).collect();
    let mut client = connect().await?;
    for url in args.urls {
        let task = client
            .add(&AddParams {
                url,
                dir: dir.clone(),
                filename: args.output.clone(),
                headers: headers.clone(),
                checksum: args.checksum.clone(),
                connections: args.connections,
                limit: args.limit,
                priority: args.priority,
                paused: args.paused,
            })
            .await?;
        println!("{}\t{}\t{}", task.id, task.state.as_str(), task.url);
    }
    Ok(ExitCode::SUCCESS)
}

/// List downloads.
#[derive(Args)]
pub struct ListArgs {
    /// Hide completed downloads.
    #[arg(short, long)]
    active: bool,
}

pub async fn list(args: ListArgs) -> anyhow::Result<ExitCode> {
    let mut tasks = connect().await?.list().await?;
    if args.active {
        tasks.retain(|t| t.state != TaskState::Completed);
    }
    print_table(&tasks);
    Ok(ExitCode::SUCCESS)
}

fn print_table(tasks: &[TaskInfo]) {
    println!("{:>4}  {:<9} {:>6} {:>11} {:>11}  NAME", "ID", "STATE", "DONE", "SIZE", "SPEED");
    for t in tasks {
        let done = match t.size {
            Some(s) if s > 0 => format!("{:.0}%", t.received as f64 * 100.0 / s as f64),
            _ => "-".into(),
        };
        let size = t.size.map(|s| HumanBytes(s).to_string()).unwrap_or_else(|| "?".into());
        let speed = if t.state == TaskState::Running { format!("{}/s", HumanBytes(t.speed)) } else { String::new() };
        let name = t.name.clone().unwrap_or_else(|| t.url.clone());
        let note = t.error.as_deref().map(|e| format!("  ({e})")).unwrap_or_default();
        println!("{:>4}  {:<9} {:>6} {:>11} {:>11}  {name}{note}", t.id, t.state.as_str(), done, size, speed);
    }
}

/// Act on downloads by id, or on all of them.
#[derive(Args)]
pub struct IdsArgs {
    #[arg(required_unless_present = "all")]
    ids: Vec<TaskId>,
    #[arg(long, conflicts_with = "ids")]
    all: bool,
}

pub async fn pause(args: IdsArgs) -> anyhow::Result<ExitCode> {
    let mut client = connect().await?;
    if args.all {
        println!("paused {}", client.pause_all().await?);
    }
    for id in args.ids {
        let t = client.pause(id).await?;
        println!("{}\t{}", t.id, t.state.as_str());
    }
    Ok(ExitCode::SUCCESS)
}

pub async fn resume(args: IdsArgs) -> anyhow::Result<ExitCode> {
    let mut client = connect().await?;
    if args.all {
        println!("resumed {}", client.resume_all().await?);
    }
    for id in args.ids {
        let t = client.resume(id).await?;
        println!("{}\t{}", t.id, t.state.as_str());
    }
    Ok(ExitCode::SUCCESS)
}

/// Remove downloads from the list.
#[derive(Args)]
pub struct RemoveArgs {
    #[arg(required = true)]
    ids: Vec<TaskId>,
    /// Also delete the downloaded and partial files.
    #[arg(long)]
    delete: bool,
}

pub async fn remove(args: RemoveArgs) -> anyhow::Result<ExitCode> {
    let mut client = connect().await?;
    for id in args.ids {
        client.remove(id, args.delete).await?;
        println!("removed {id}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Set a speed limit for one download, or for all together.
#[derive(Args)]
pub struct LimitArgs {
    /// Bytes per second, e.g. 500K or 2M; 0 removes the limit.
    #[arg(value_parser = parse_size)]
    rate: u64,
    /// Limit only this download.
    #[arg(long)]
    id: Option<TaskId>,
}

pub async fn limit(args: LimitArgs) -> anyhow::Result<ExitCode> {
    let mut client = connect().await?;
    match args.id {
        Some(id) => {
            client.set_limit(id, args.rate).await?;
        }
        None => {
            client.set_settings(&SettingsPatch { global_limit: Some(args.rate), ..Default::default() }).await?;
        }
    }
    let what = args.id.map(|id| format!("download {id}")).unwrap_or_else(|| "all downloads".into());
    if args.rate == 0 {
        println!("no limit on {what}");
    } else {
        println!("{what} limited to {}/s", HumanBytes(args.rate));
    }
    Ok(ExitCode::SUCCESS)
}

/// Change a download's priority (higher runs first).
#[derive(Args)]
pub struct PriorityArgs {
    id: TaskId,
    #[arg(allow_negative_numbers = true)]
    priority: i32,
}

pub async fn priority(args: PriorityArgs) -> anyhow::Result<ExitCode> {
    let t = connect().await?.set_priority(args.id, args.priority).await?;
    println!("{}\tpriority {}", t.id, t.priority);
    Ok(ExitCode::SUCCESS)
}

pub async fn status() -> anyhow::Result<ExitCode> {
    let s = connect().await?.status().await?;
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
}

/// Show or change daemon settings.
#[derive(Args)]
pub struct ConfigArgs {
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

pub async fn config(args: ConfigArgs) -> anyhow::Result<ExitCode> {
    let mut client = connect().await?;
    let patch = SettingsPatch {
        max_active: args.max_active,
        connections: args.connections,
        download_dir: args.dir.map(|d| std::path::absolute(d).map(|p| p.to_string_lossy().into_owned())).transpose()?,
        global_limit: None,
        pause_on_metered: args.pause_on_metered,
    };
    let any = patch.max_active.is_some()
        || patch.connections.is_some()
        || patch.download_dir.is_some()
        || patch.pause_on_metered.is_some();
    let s = if any { client.set_settings(&patch).await? } else { client.settings().await? };
    println!("max_active        {}", s.max_active);
    println!("connections       {}", s.connections);
    let limit = if s.global_limit == 0 { "none".into() } else { format!("{}/s", HumanBytes(s.global_limit)) };
    println!("global_limit      {limit}");
    println!("download_dir      {}", s.download_dir);
    println!("pause_on_metered  {}", s.pause_on_metered);
    Ok(ExitCode::SUCCESS)
}

/// Follow downloads live until Ctrl-C.
pub async fn watch() -> anyhow::Result<ExitCode> {
    let mut client = connect().await?;
    let mut tasks: BTreeMap<TaskId, TaskInfo> = client.list().await?.into_iter().map(|t| (t.id, t)).collect();
    let mut events = client.subscribe().await?;
    redraw(&tasks);
    loop {
        let event = tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(ExitCode::SUCCESS),
            e = events.next() => e?,
        };
        let Some(event) = event else {
            eprintln!("haulerd stopped");
            return Ok(ExitCode::FAILURE);
        };
        match event {
            Event::TaskAdded { task } | Event::TaskUpdated { task } => {
                tasks.insert(task.id, task);
            }
            Event::TaskRemoved { id } => {
                tasks.remove(&id);
            }
            Event::Progress { updates } => {
                for u in updates {
                    if let Some(t) = tasks.get_mut(&u.id) {
                        t.received = u.received;
                        t.speed = u.speed;
                        t.connections = u.connections;
                    }
                }
            }
            Event::Settings { .. } | Event::Network { .. } => {}
        }
        redraw(&tasks);
    }
}

fn redraw(tasks: &BTreeMap<TaskId, TaskInfo>) {
    // Clear the screen and home the cursor.
    print!("\x1b[2J\x1b[H");
    let list: Vec<_> = tasks.values().cloned().collect();
    print_table(&list);
    let _ = std::io::stdout().flush();
}
