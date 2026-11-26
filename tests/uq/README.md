Tests for hauler's Downloads page in uQ Settings (`uq/pages/hauler/`).

```
logic.test.mjs     Node: logic.js, edit.js, page.toml, and what the QML may send   node --test 'tests/uq/*.test.mjs'
load-logic.mjs     loads logic.js and edit.js (no Qt calls in them; `.import`s followed) for Node
toml-lite.mjs      the small TOML reader the Node tests use for page.toml
tst_*.qml          QML: the page, tab by tab, against a fake ctx (needs Qt 6; CI runs it)
PageHarness.qml    what the QML tests share: the page in a Loader, a fake ctx answering every command from fixtures/
stub/ FakeContext.qml tst_components.qml
                   the page test kit of uQ at the commit pinned in .github/workflows/ci.yml (uq_api 3,
                   templates/settings-page-tests/), unchanged. Refresh them when moving to a newer uQ.
fixtures/          what the real `hauler` commands print (see below)
```

Run the QML tests offscreen (`/usr/lib/qt6/bin/qmltestrunner` on Arch, from `qt6-declarative`; Debian and Ubuntu
also need `qml6-module-qtquick-dialogs` for the folder dialog). `QML_XHR_ALLOW_FILE_READ` lets the tests read
`fixtures/`:

```
QML_XHR_ALLOW_FILE_READ=1 QT_QPA_PLATFORM=offscreen qmltestrunner -import tests/uq/stub -input tests/uq
```

Check the folder itself with uQ's own checker, built from the commit pinned in CI:

```
uqctl integrations check --no-programs uq/
```

## Fixtures

`fixtures/*.json` are the real output of each `hauler` command the page runs, recorded against a daemon with a
completed download that kept a cookie, one whose file is gone, a failed one, one running, two paused, and a stray
unfinished download, and paths rewritten to `/home/user/Downloads`.
`crates/hauler-cli/tests/uq_page.rs` runs the commands again, in a daemon of its own, and fails when a fixture's
keys or value types no longer match, so the page tests can't drift from the CLI. After changing a command's output:

```
UPDATE_FIXTURES=1 cargo test -p hauler-cli --test uq_page
```

That test also runs every `argv` in `page.toml` against the real command line parser, with sample arguments that
fit each pattern.
