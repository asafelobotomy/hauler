//! `hauler get`: download one URL in this process, without the daemon.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Context, bail};
use clap::Args;
use hauler_engine::header::{HeaderMap, HeaderName, HeaderValue};
use hauler_engine::{CancellationToken, Checksum, Error, Options, Outcome, Progress, client, prepare};
use indicatif::{HumanBytes, ProgressBar, ProgressStyle};

use crate::{parse_header, parse_size};

/// Download a URL directly. Ctrl-C pauses; run it again to resume.
#[derive(Args)]
pub struct GetArgs {
    /// URL to download.
    url: String,
    /// Directory to save into.
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,
    /// Save under this file name instead of the server's.
    #[arg(short = 'o', long)]
    output: Option<String>,
    /// Parallel connections.
    #[arg(short = 'x', long, default_value_t = 8, value_parser = clap::value_parser!(u16).range(1..=32))]
    connections: u16,
    /// Smallest piece a segment is split into, e.g. 512K or 4M.
    #[arg(long, default_value = "1M", value_parser = parse_size)]
    min_split: u64,
    /// Speed limit in bytes per second, e.g. 2M.
    #[arg(long, value_parser = parse_size)]
    limit: Option<u64>,
    /// Verify the result, e.g. sha256:<hex>.
    #[arg(long)]
    checksum: Option<Checksum>,
    /// Extra request header, e.g. -H 'Cookie: a=b'. Repeatable.
    #[arg(short = 'H', long = "header", value_parser = parse_header)]
    headers: Vec<(HeaderName, HeaderValue)>,
    /// Discard any saved progress and start over.
    #[arg(long)]
    restart: bool,
    /// No progress bar.
    #[arg(short, long)]
    quiet: bool,
}

pub async fn run(args: GetArgs) -> anyhow::Result<ExitCode> {
    if !args.dir.is_dir() {
        bail!("{} is not a directory", args.dir.display());
    }

    let cancel = CancellationToken::new();
    let on_signal = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        on_signal.cancel();
        // A second Ctrl-C means stop now, without saving.
        let _ = tokio::signal::ctrl_c().await;
        std::process::exit(130);
    });

    let mut headers = HeaderMap::new();
    for (name, value) in args.headers {
        headers.append(name, value);
    }
    let mut opts = Options {
        connections: args.connections.into(),
        min_split: args.min_split,
        headers,
        filename: args.output,
        checksum: args.checksum,
        restart: args.restart,
        limiter: args.limit.map(hauler_engine::Limiter::new).into_iter().collect(),
        ..Options::default()
    };
    let http = client()?;

    loop {
        let task =
            prepare(http.clone(), &args.url, &args.dir, opts.clone()).await.context("could not start download")?;
        let resumable = task.resumable();
        if !args.quiet {
            let size = task.size().map(|s| HumanBytes(s).to_string()).unwrap_or_else(|| "unknown size".into());
            eprintln!("Saving {} ({size})", task.target().display());
            if task.resumed_bytes() > 0 {
                eprintln!("Resuming with {} already on disk", HumanBytes(task.resumed_bytes()));
            }
            if !resumable {
                eprintln!("The server does not support ranges: one connection, and no resume after a pause");
            }
        }

        let bar = (!args.quiet).then(|| progress_bar(task.size(), task.progress()));
        let result = task.run(cancel.clone()).await;
        if let Some(bar) = bar {
            bar.abandon();
        }

        match result {
            Ok(Outcome::Completed(path)) => {
                if !args.quiet {
                    eprintln!("Done: {}", path.display());
                }
                return Ok(ExitCode::SUCCESS);
            }
            Ok(Outcome::Paused) => {
                if resumable {
                    eprintln!("Paused. Run the same command again to resume.");
                } else {
                    eprintln!("Stopped. This download cannot be resumed.");
                }
                return Ok(ExitCode::from(130));
            }
            Err(Error::Changed) if !opts.restart => {
                eprintln!("The file changed on the server; starting over.");
                opts.restart = true;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn progress_bar(size: Option<u64>, progress: Arc<Progress>) -> ProgressBar {
    let bar = match size {
        Some(size) => ProgressBar::new(size).with_style(
            ProgressStyle::with_template(
                "{bar:30.cyan/blue} {bytes:>10}/{total_bytes} {bytes_per_sec:>12} eta {eta:>4}  {msg}",
            )
            .unwrap()
            .progress_chars("=> "),
        ),
        None => ProgressBar::no_length()
            .with_style(ProgressStyle::with_template("{spinner} {bytes:>10} {bytes_per_sec:>12}  {msg}").unwrap()),
    };
    bar.set_position(progress.received.load(Ordering::Relaxed));
    bar.reset_eta();
    let ticker = bar.clone();
    tokio::spawn(async move {
        while !ticker.is_finished() {
            ticker.set_position(progress.received.load(Ordering::Relaxed));
            ticker.set_message(format!(
                "{} conn  {} splits  {} restarts",
                progress.connections.load(Ordering::Relaxed),
                progress.splits.load(Ordering::Relaxed),
                progress.restarts.load(Ordering::Relaxed),
            ));
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    bar
}
