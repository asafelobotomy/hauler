# hauler

Fast, resumable downloads over parallel connections. Linux first.

This is Phase 0: the download engine and a command-line client. A daemon, desktop UI and browser extension come later.

## Usage

```sh
cargo build --release
target/release/hauler https://example.com/big.iso
target/release/hauler -x 16 -d ~/Downloads --checksum sha256:<hex> https://example.com/big.iso
```

Press Ctrl-C to pause, then run the same command again to resume. Run `hauler --help` for all options.
Set `HAULER_LOG=debug` to see splits, restarts and retries.

## How it works

- **Probe.** A `Range: bytes=0-0` request gets the size, confirms range support, and records a strong `ETag` or `Last-Modified`.
- **Dynamic splitting.** The file starts as up to N equal segments. When a connection runs out of work, it takes the upper half of the segment with the most bytes left. The connection that owned that segment stops at the new boundary.
- **Slow-connection restarts.** A connection running below 0.3× the mean speed for 5 s gets reopened, because a new connection often takes a faster path through the CDN.
- **Crash-safe resume.** Data goes straight into one preallocated `<name>.part` file with positional writes. Every 2 s the engine syncs that data, then atomically replaces `<name>.hauler` with the progress of each segment. A crash, including `kill -9`, can lose a few seconds of progress, but it never records bytes as done before they are on disk.
- **Safe resume.** Every range request sends `If-Range`. If the file changed on the server, the engine notices and starts over instead of mixing two versions.
- **Retries.** Exponential backoff with jitter, honouring `Retry-After`, for timeouts, 408, 429 and 5xx responses.
- **No ranges?** Falls back to one connection, which cannot be resumed.

## Layout

| Crate | What it is |
| --- | --- |
| `crates/hauler-engine` | The library: probe, segment planner, downloader, resume state |
| `crates/hauler-cli` | The `hauler` binary |
| `crates/hauler-testserver` | A throttled HTTP server with Range/ETag support, used by the tests and for local benchmarks |

## Testing

```sh
cargo test
cargo run --release -p hauler-testserver -- --size-mib 256 --kib-per-sec 4096 --slow-every 3
```

The tests cover byte-exact downloads, checksum checks, servers without range support, pause and resume, a file that changes on the server, and splitting around slow connections.
