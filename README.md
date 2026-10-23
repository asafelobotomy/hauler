# hauler

Fast, resumable downloads over parallel connections. Linux first.

Status: Phase 0 (engine and CLI) and Phase 1 (daemon, API and queue) are done. The desktop UI and browser extension come next.

## Install for your user

```sh
scripts/install-user.sh
```

This builds release binaries into `~/.local/bin` and installs systemd user units. `hauler.socket` starts `haulerd` the first time a client connects.

## Usage

Through the daemon, with a queue that survives restarts:

```sh
hauler add https://example.com/big.iso            # queue it; prints the id
hauler add -x 16 --limit 2M -p 5 <url> <url> ...  # 16 connections, 2 MiB/s cap, priority 5
hauler ls                                         # list downloads (alias of `list`)
hauler watch                                      # live view
hauler pause 3 | hauler resume 3 | hauler pause --all
hauler rm 3 --delete                              # remove, and delete its files
hauler limit 5M                                   # cap all downloads together; 0 removes it
hauler limit 1M --id 3                            # cap one download
hauler priority 3 10                              # higher runs first
hauler config --max-active 5 --dir ~/Downloads --pause-on-metered false
hauler status
```

Standalone, without the daemon:

```sh
hauler get -x 16 --checksum sha256:<hex> https://example.com/big.iso
```

Ctrl-C pauses `get`; running the same command again resumes it. Set `HAULER_LOG=debug` to see splits, restarts and retries.

## How downloads work

- **Probe.** A `Range: bytes=0-0` request gets the size, confirms range support, and records a strong `ETag` or `Last-Modified`.
- **Dynamic splitting.** The file starts as up to N equal segments. When a connection runs out of work, it takes the upper half of the segment with the most bytes left. The connection that owned that segment stops at the new boundary.
- **Slow-connection restarts.** A connection running below 0.3× the mean speed for 5 s gets reopened, because a new connection often takes a faster path through the CDN.
- **Crash-safe resume.** Data goes straight into one preallocated `<name>.part` file with positional writes. Every 2 s the engine syncs that data, then atomically replaces `<name>.hauler` with the progress of each segment. A crash, including `kill -9`, can lose a few seconds of progress, but it never records bytes as done before they are on disk.
- **Safe resume.** Every range request sends `If-Range`. If the file changed on the server, the engine notices and starts over instead of mixing two versions.
- **Speed limits.** Token buckets for each download and one shared bucket, all changeable while running. A limited connection stops reading, so TCP slows the server down too.
- **Retries.** Exponential backoff with jitter, honouring `Retry-After`, for timeouts, 408, 429 and 5xx responses.
- **No ranges?** Falls back to one connection, which cannot be resumed.

## The daemon

`haulerd` owns the queue and runs downloads with the engine.

- **API.** JSON-RPC 2.0, one JSON object per line, on `$XDG_RUNTIME_DIR/hauler/hauler.sock` (mode 0600). File permissions are the authentication. Methods and types are in `crates/hauler-ipc`. A `subscribe` call turns the connection into a stream of `event` notifications: task added, updated or removed, progress about once a second, settings, network.
- **Queue.** Up to `max_active` downloads run at once. Waiting downloads start by priority, then by the order they were added. Downloads that were running when the daemon stopped go back in the queue and resume on the next start.
- **Storage.** SQLite in WAL mode at `$XDG_STATE_HOME/hauler/hauler.db`. Per-segment progress stays in each download's `.hauler` file.
- **Sleep.** While anything downloads, the daemon holds a logind sleep inhibitor, visible in `systemd-inhibit --list`.
- **Network.** It follows NetworkManager. Downloads are held while offline, or on a metered connection unless `pause_on_metered` is off, and they resume by themselves.
- **Idle cost.** About 11 MB of memory, and no timers run while nothing is downloading.

## Benchmarks

Measured with `scripts/bench.sh` against the local test server, which caps each connection's speed the way many servers and CDNs do. 16 connections each, Intel i5-8300H laptop, NVMe disk.

| Scenario | hauler | aria2c 1.37 |
| --- | --- | --- |
| 1 GiB, every connection 8 MiB/s | 13.2 s | 13.1 s |
| 1 GiB, every 3rd connection 10× slower | 17.9 s | 25.5 s |
| 10 GiB, 16 MiB/s, every 3rd connection 10× slower | 120.8 s | 259.3 s |
| 1 GiB, no cap (localhost, CPU-bound) | 1.8 s | 7.0 s |

With every connection equal, both tools are limited by the server. With uneven connections, hauler's dynamic splitting keeps every connection busy until the end. Real-world numbers depend on the server; these are a reproducible baseline, not a promise.

## Layout

| Crate | What it is |
| --- | --- |
| `crates/hauler-engine` | The library: probe, segment planner, downloader, resume state, speed limits |
| `crates/hauler-ipc` | API types and the async client for the daemon socket |
| `crates/hauler-daemon` | `haulerd`: queue, scheduler, SQLite store, RPC server, D-Bus integration |
| `crates/hauler-cli` | The `hauler` command |
| `crates/hauler-testserver` | A throttled HTTP server with Range/ETag support, for tests and benchmarks |

## Testing

```sh
cargo test
scripts/bench.sh 1024 8192 3 16    # size MiB, KiB/s per connection, slow every Nth, connections
```

The tests cover:
- byte-exact downloads and checksum checks
- servers without range support
- pause and resume
- a file that changes on the server
- splitting around slow connections
- 50 downloads running in the daemon at once
- pause, resume and restarting the daemon mid-download
- priority order and the global speed limit
- API error handling
