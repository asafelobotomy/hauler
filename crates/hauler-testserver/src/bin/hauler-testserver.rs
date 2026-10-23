use clap::Parser;
use hauler_testserver::{Config, spawn_on};

/// Serve a synthetic file with Range, ETag and per-connection throttling.
#[derive(Parser)]
struct Args {
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: std::net::SocketAddr,
    /// File size in MiB.
    #[arg(long, default_value_t = 256)]
    size_mib: u64,
    /// Per-connection cap in KiB/s (0 = unlimited).
    #[arg(long, default_value_t = 0)]
    kib_per_sec: u64,
    /// Make every Nth request 10x slower.
    #[arg(long)]
    slow_every: Option<u64>,
    /// Disable Range support.
    #[arg(long)]
    no_ranges: bool,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let config = Config {
        size: args.size_mib * 1024 * 1024,
        ranges: !args.no_ranges,
        bytes_per_sec: (args.kib_per_sec > 0).then_some(args.kib_per_sec * 1024),
        slow_every: args.slow_every.map(|n| (n, 10)),
        filename: None,
    };
    let server = spawn_on(args.listen, config).await?;
    println!("serving {}", server.url());
    std::future::pending::<()>().await;
    Ok(())
}
