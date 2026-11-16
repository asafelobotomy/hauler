# hauler in uQ

What this folder adds to the [uQ](https://github.com/asafelobotomy/uq) desktop. hauler's package
installs it unconditionally; without uQ nothing reads it.

| File | Installed to | What it does |
| --- | --- | --- |
| `widgets/hauler/widget.toml`, `Widget.qml` | `/usr/share/uq/shell/widgets/hauler/` | A Downloads icon in the bar: a progress ring while anything downloads, and a popup with the queue, Pause all and Resume all |
| `pages/hauler/` | `/usr/share/uq/settings/pages/hauler/` | **Downloads** under Apps in Settings: the live queue, the history of finished downloads, the download folder and disk space, what Hauler refuses and keeps for safety, and its settings. See below |
| `launcher/hauler.provider` | `/usr/share/uq/launcher/` | A URL typed or pasted into look=see>do_ offers "Download with hauler" |
| `dbus/org.hauler.Hauler1.service` | `/usr/share/dbus-1/services/` | Lets the session bus start `haulerd`, through systemd's `hauler.service` |

Both the widget and the provider talk to `haulerd` over `org.hauler.Hauler1` on the session bus
(see "D-Bus and uQ" in the top-level README). `haulerd` owns that name for as long as it runs, so
the widget shows while it runs and goes when it stops. The provider is not activatable: uQ
asks it only while `haulerd` runs, so opening look=see>do_ never starts an idle hauler (which would
then run, and show the widget, for the rest of the session).

## The Downloads page in Settings

Five tabs, followed live through haulerd's D-Bus signals, so it shows what Hauler's own window and the browser do:

- **Overview.** What runs now, a progress bar per download, Pause and Resume (all or one), and a field to add a link.
- **History.** Finished downloads, newest first, with a filter. Remove one, remove it with its file, or clear them all,
  each after a plan of exactly what goes. Set how long the list is kept.
- **Storage.** The download folder, free space, what Hauler's downloads take, a reserve of space to keep free, and
  unfinished leftovers nothing is working on, which can be deleted after a plan.
- **Security.** Only allow https; forget saved cookies and logins when a download completes, or now; and what is
  always on.
- **Preferences.** Downloads at once, connections, the speed limit for all, and pausing on a metered connection.

It runs only the `hauler` command line, as you (`hauler status|list|storage|config show --json`, and `--dry-run --json`
plans for everything that deletes). The commands are in `pages/hauler/page.toml`; the shape each prints is the contract
that `crates/hauler-cli/tests/uq_page.rs` checks against `tests/uq/fixtures/`. The page's tests are in `tests/uq/`.

## How it was tested

- `node --test 'tests/uq/*.test.mjs'` and `qmltestrunner` run the page against a fake `ctx` (see `tests/uq/README.md`),
  and `cargo test -p hauler-cli --test uq_page` runs every command it declares against a real daemon.
- `uqctl integrations check uq/` is clean, apart from the program paths under `/usr/bin` when
  hauler isn't installed.
- `cargo test -p hauler-daemon --test dbus` runs the interface and the provider on a private
  bus: properties, `PauseAll`, `ResumeAll`, `Add` (and its URL check), `Queue`, `Finished`, the
  once-a-second limit on `PropertiesChanged`, and the provider's `Search` and `Activate`.
- `Widget.qml` was loaded in `qmltestrunner` with a stand-in `ctx`: the ring follows the
  properties, the popup lists the queue as plain text, and the buttons call `PauseAll` and
  `ResumeAll`.

By hand, with hauler running:

```sh
busctl --user get-property org.hauler.Hauler1 /org/hauler/Hauler1 org.hauler.Hauler1 ActiveCount
busctl --user call org.hauler.Hauler1 /org/uq/Launcher1 org.uq.Launcher1.Provider Search s https://example.com/a.iso
```
