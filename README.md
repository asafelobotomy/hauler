# hauler

Fast, resumable downloads over parallel connections. Linux first.

Status: Phase 0 (engine and CLI), Phase 1 (daemon, API and queue) and Phase 2 (desktop app) are done. The browser extension comes next.

## Install for your user

```sh
scripts/install-user.sh
```

This builds release binaries into `~/.local/bin`, installs the systemd user units and adds Hauler to the app menu. `hauler.socket` starts `haulerd` the first time a client connects.

Building the desktop app needs Qt 6 (base and declarative), Kirigami, KItemModels and qqc2-desktop-style. On Arch and Manjaro, all of these come with Plasma.

## Desktop app

`hauler-desktop` is a Kirigami window onto the daemon. Closing it keeps downloads running; downloads also show up in Plasma's notification area without it.

- **List.** Shows every download with progress, speed, time left and connections. Filters are All, Active and Finished. Each row has pause, resume, open, open folder and remove; removing asks first, with an option to delete the file.
- **Details.** Shows a live segment map of which byte ranges are done and which are being fetched. The speed limit and priority can be changed here.
- **Adding.** Several links at once, folder, file name, connections, speed limit, priority, and "add paused". Ctrl+N opens the dialog, Ctrl+V pastes links, and links can be dropped on the window. `hauler-desktop <url>…` opens the dialog with them filled in.
- **Settings.** Downloads at once, connections, total speed limit, folder, and whether to pause on metered connections.
- **Tray.** Click to show or hide the window. The menu has pause all, resume all and quit. `--minimized` starts in the tray.
- **Plasma.** The daemon reports each download to Plasma's job tracker, so progress shows in the notification area. Pause, resume and cancel work from there; cancel removes the download and its partial file. Without a job tracker (GNOME and others), it sends a notification when a download finishes, with Open and Show in Folder actions.

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
- **Retries.** Exponential backoff with jitter, honouring `Retry-After` up to 2 minutes, for timeouts, 408, 429 and 5xx responses.
- **Connection caps.** Some hosts allow only a few connections per client and redirect the rest to a "too many requests" page or answer 429 or 503. While other connections are still streaming, the turned-away ones drop out and the rest of the file is split among the ones that work.
- **No ranges?** Falls back to one connection, which cannot be resumed.

## The daemon

`haulerd` owns the queue and runs downloads with the engine.

- **API.** JSON-RPC 2.0, one JSON object per line, on `$XDG_RUNTIME_DIR/hauler/hauler.sock` (mode 0600). File permissions are the authentication. Methods and types are in `crates/hauler-ipc`. A `subscribe` call turns the connection into a stream of `event` notifications: task added, updated or removed, progress about once a second, settings, network.
- **Queue.** Up to `max_active` downloads run at once. Waiting downloads start by priority, then by the order they were added. Downloads that were running when the daemon stopped go back in the queue and resume on the next start.
- **Storage.** SQLite in WAL mode at `$XDG_STATE_HOME/hauler/hauler.db`. Per-segment progress stays in each download's `.hauler` file.
- **Sleep.** While anything downloads, the daemon holds a logind sleep inhibitor, visible in `systemd-inhibit --list`.
- **Network.** It follows NetworkManager. Downloads are held while offline, or on a metered connection unless `pause_on_metered` is off, and they resume by themselves.
- **Logs.** Under systemd they go to the journal (`journalctl --user -u hauler`). Started by the desktop app, they go to `$XDG_STATE_HOME/hauler/haulerd.log`, with the previous run's in `haulerd.log.old`. `HAULER_LOG=debug` shows more.
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
| `crates/hauler-desktop` | `hauler-desktop`: Kirigami UI with a cxx-qt bridge to the daemon |
| `crates/hauler-testserver` | A throttled HTTP server with Range/ETag support, for tests and benchmarks |

## Testing

```sh
cargo test
scripts/bench.sh 1024 8192 3 16    # size MiB, KiB/s per connection, slow every Nth, connections
```

To capture the desktop app's main view, a details dialog, the add dialog and settings without showing a window:

```sh
QT_QPA_PLATFORM=offscreen HAULER_DESKTOP_DEMO=/tmp/shot target/debug/hauler-desktop   # writes /tmp/shot-1.png … -4.png
```

Offscreen, the Plasma style does not draw progress bars; on screen it does.

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

## Licence

The reusable libraries are dual-licensed MIT OR Apache-2.0, at your option, so other projects can embed them: `hauler-engine`, `hauler-ipc` and `hauler-testserver` ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).

The applications are GPL-3.0-or-later: `haulerd` (`hauler-daemon`), the `hauler` CLI (`hauler-cli`) and the desktop app (`hauler-desktop`) ([COPYING](COPYING)).
