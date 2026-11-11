# hauler in uQ

What this folder adds to the [uQ](https://github.com/asafelobotomy/uq) desktop. hauler's package
installs it unconditionally; without uQ nothing reads it.

| File | Installed to | What it does |
| --- | --- | --- |
| `widgets/hauler/widget.toml`, `Widget.qml` | `/usr/share/uq/shell/widgets/hauler/` | A Downloads icon in the bar: a progress ring while anything downloads, and a popup with the queue, Pause all and Resume all |
| `launcher/hauler.provider` | `/usr/share/uq/launcher/` | A URL typed or pasted into Spotlight offers "Download with hauler" |
| `dbus/org.hauler.Hauler1.service` | `/usr/share/dbus-1/services/` | Lets the session bus start `haulerd`, through systemd's `hauler.service` |

Both the widget and the provider talk to `haulerd` over `org.hauler.Hauler1` on the session bus
(see "D-Bus and uQ" in the top-level README). `haulerd` owns that name for as long as it runs, so
the widget shows while it runs and goes when it stops. The provider is not activatable: uQ
asks it only while `haulerd` runs, so opening Spotlight never starts an idle hauler (which would
then run, and show the widget, for the rest of the session).

## How it was tested

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
