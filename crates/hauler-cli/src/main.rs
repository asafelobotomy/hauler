use std::process::ExitCode;

use clap::{Parser, Subcommand};
use hauler_engine::header::{HeaderName, HeaderValue};

mod get;
mod remote;

/// Fast, resumable downloads over parallel connections.
#[derive(Parser)]
#[command(name = "hauler", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Get(get::GetArgs),
    Add(remote::AddArgs),
    #[command(alias = "ls")]
    List(remote::ListArgs),
    /// Pause downloads.
    Pause(remote::IdsArgs),
    /// Resume paused or failed downloads.
    Resume(remote::IdsArgs),
    #[command(alias = "rm")]
    Remove(remote::RemoveArgs),
    Limit(remote::LimitArgs),
    Priority(remote::PriorityArgs),
    /// Follow downloads live.
    Watch,
    /// Show what the daemon is doing.
    Status,
    Config(remote::ConfigArgs),
}

pub(crate) fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num, mult) = match s.char_indices().last() {
        Some((i, c)) if c.eq_ignore_ascii_case(&'k') => (&s[..i], 1024),
        Some((i, c)) if c.eq_ignore_ascii_case(&'m') => (&s[..i], 1024 * 1024),
        Some((i, c)) if c.eq_ignore_ascii_case(&'g') => (&s[..i], 1024 * 1024 * 1024),
        _ => (s, 1),
    };
    num.parse::<u64>().map(|n| n * mult).map_err(|_| format!("not a size: {s}"))
}

pub(crate) fn parse_header(s: &str) -> Result<(HeaderName, HeaderValue), String> {
    let (name, value) = s.split_once(':').ok_or("expected 'Name: value'")?;
    let name = HeaderName::from_bytes(name.trim().as_bytes()).map_err(|e| e.to_string())?;
    let value = HeaderValue::from_str(value.trim()).map_err(|e| e.to_string())?;
    Ok((name, value))
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    // Exit quietly when piped into `head` and the like, as other CLI tools do.
    // SAFETY: called before any other threads touch signal handling.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("HAULER_LOG").unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Command::Get(args) => get::run(args).await,
        Command::Add(args) => remote::add(args).await,
        Command::List(args) => remote::list(args).await,
        Command::Pause(args) => remote::pause(args).await,
        Command::Resume(args) => remote::resume(args).await,
        Command::Remove(args) => remote::remove(args).await,
        Command::Limit(args) => remote::limit(args).await,
        Command::Priority(args) => remote::priority(args).await,
        Command::Watch => remote::watch().await,
        Command::Status => remote::status().await,
        Command::Config(args) => remote::config(args).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("512K"), Ok(512 * 1024));
        assert_eq!(parse_size("4m"), Ok(4 * 1024 * 1024));
        assert_eq!(parse_size("100"), Ok(100));
        assert!(parse_size("lots").is_err());
    }

    #[test]
    fn headers() {
        let (n, v) = parse_header("Cookie: a=b; c=d").unwrap();
        assert_eq!(n.as_str(), "cookie");
        assert_eq!(v, "a=b; c=d");
        assert!(parse_header("nocolon").is_err());
    }
}
