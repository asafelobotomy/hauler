//! The Downloads page in uQ Settings (`uq/pages/hauler/`) against the real `hauler`: every command its `page.toml`
//! declares is run against a daemon of its own, and what it prints must have the shape of the fixture the page
//! tests use (`tests/uq/fixtures/`), so the page tests can't drift from the command line.
//!
//! After changing what a command prints: `UPDATE_FIXTURES=1 cargo test -p hauler-cli --test uq_page`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use hauler_daemon::{Config, run};
use hauler_ipc::{AddParams, Client, SettingsPatch, TaskState};
use hauler_testserver::{Config as ServerConfig, spawn};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The type of every value, with objects by key, lists by the union of their items; `null` fits anything.
fn shape(v: &Value) -> Value {
    match v {
        Value::Null => Value::Null,
        Value::Bool(_) => "bool".into(),
        Value::Number(_) => "number".into(),
        Value::String(_) => "string".into(),
        Value::Array(items) => {
            let merged = items.iter().map(shape).fold(Value::Null, merge);
            Value::Array(vec![merged])
        }
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), shape(v))).collect()),
    }
}

fn merge(a: Value, b: Value) -> Value {
    match (a, b) {
        (Value::Null, x) | (x, Value::Null) => x,
        (Value::Object(mut a), Value::Object(b)) => {
            for (k, v) in b {
                let old = a.remove(&k).unwrap_or(Value::Null);
                a.insert(k, merge(old, v));
            }
            Value::Object(a)
        }
        (Value::Array(a), Value::Array(b)) => Value::Array(vec![merge(a[0].clone(), b[0].clone())]),
        (a, _) => a,
    }
}

/// Where `got` and `want` differ in keys or types (a null on either side fits).
fn differences(path: &str, got: &Value, want: &Value, out: &mut Vec<String>) {
    match (got, want) {
        (Value::Null, _) | (_, Value::Null) => {}
        (Value::Object(g), Value::Object(w)) => {
            for k in g.keys().chain(w.keys()).collect::<std::collections::BTreeSet<_>>() {
                match (g.get(k), w.get(k)) {
                    (Some(g), Some(w)) => differences(&format!("{path}.{k}"), g, w, out),
                    (Some(_), None) => out.push(format!("{path}.{k}: the command prints it, the fixture lacks it")),
                    (None, _) => out.push(format!("{path}.{k}: the fixture has it, the command does not print it")),
                }
            }
        }
        (Value::Array(g), Value::Array(w)) => differences(&format!("{path}[]"), &g[0], &w[0], out),
        (g, w) if g == w => {}
        (g, w) => out.push(format!("{path}: the command prints {g}, the fixture has {w}")),
    }
}

/// Paths in output are the test's own; the fixtures say `/home/user/Downloads`.
fn normalize(text: &str, downloads: &Path) -> String {
    text.replace(&downloads.to_string_lossy().into_owned(), "/home/user/Downloads")
}

struct Run {
    code: i32,
    stdout: String,
}

fn hauler(socket: &Path, args: &[String]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_hauler"))
        .args(args)
        .env("HAULER_SOCKET", socket)
        .env_remove("XDG_RUNTIME_DIR")
        .env("RUST_BACKTRACE", "0")
        .output()
        .unwrap();
    Run { code: out.status.code().unwrap(), stdout: String::from_utf8_lossy(&out.stdout).into_owned() }
}

/// A sample argument that fits the pattern of the command's argument `name`.
fn sample(command: &str, name: &str, task: i64, other: i64) -> String {
    match name {
        // Two removals can't both take the same download.
        "id" if command == "remove-one" => other.to_string(),
        "id" => task.to_string(),
        "state" => "finished".into(),
        "url" => "https://example.com/new.bin".into(),
        "key" => "history_days".into(),
        "value" if command == "preview-config" => "30".into(),
        "value" => "0".into(),
        other => panic!("{command}: no sample for the argument {other}"),
    }
}

/// The fixture each command's answer is compared with, and the command that makes it.
const FIXTURES: &[(&str, &str)] = &[
    ("status", "status.json"),
    ("queue", "queue.json"),
    ("history", "history.json"),
    ("storage", "storage.json"),
    ("config", "config.json"),
    ("preview-config", "preview-config-retention.json"),
    ("preview-remove", "preview-remove.json"),
    ("preview-remove-files", "preview-remove-files.json"),
    ("preview-clear", "preview-clear.json"),
    ("preview-clear-files", "preview-clear-files.json"),
    ("preview-forget-headers", "preview-forget-headers.json"),
    ("preview-cleanup", "preview-cleanup.json"),
    ("config-set", "config-set.json"),
    ("forget-headers", "forget-headers.json"),
    ("remove-one", "remove-one.json"),
    ("clear-history", "clear-history.json"),
    ("cleanup", "cleanup.json"),
];

/// Changes things: run after the reads and the plans have seen the seeded state.
const DESTRUCTIVE: &[&str] =
    &["forget-headers", "remove-one", "remove-one-files", "clear-history", "clear-history-files", "cleanup"];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_command_of_the_page_runs_and_prints_what_the_fixtures_say() {
    let dir = std::env::temp_dir().join(format!("hauler-uq-page-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let downloads = dir.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let run_dir = dir.join("run");
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&run_dir).unwrap();
    }
    let socket = run_dir.join("hauler.sock");
    let stop = CancellationToken::new();
    let config = Config {
        socket: socket.clone(),
        db: Some(dir.join("h.db")),
        download_dir: downloads.clone(),
        system_integration: false,
    };
    let daemon = tokio::spawn(run(config, stop.clone()));
    let mut client = loop {
        match Client::connect(&socket).await {
            Ok(c) => break c,
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    };

    // The situation the fixtures were recorded in: a download that kept its cookie, a failed one, a paused one,
    // and an unfinished download nothing owns.
    let server = spawn(ServerConfig { size: 64 * 1024, ..Default::default() }).await.unwrap();
    client.set_settings(&SettingsPatch { scrub_headers: Some(false), ..Default::default() }).await.unwrap();
    let headers = vec![("Cookie".to_string(), "sid=secret".to_string())];
    let done = client
        .add(&AddParams { url: server.url(), filename: Some("done.bin".into()), headers, ..Default::default() })
        .await
        .unwrap();
    let broken = AddParams {
        url: server.url().replace("http://", "https://"),
        filename: Some("broken.bin".into()),
        ..Default::default()
    };
    let failed = client.add(&broken).await.unwrap();
    client
        .add(&AddParams { url: server.url(), filename: Some("later.bin".into()), paused: true, ..Default::default() })
        .await
        .unwrap();
    for id in [done.id, failed.id] {
        for _ in 0..300 {
            if client.get(id).await.unwrap().state.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    assert_eq!(client.get(done.id).await.unwrap().state, TaskState::Completed);
    assert_eq!(client.get(failed.id).await.unwrap().state, TaskState::Failed);
    std::fs::write(downloads.join("stray.bin.part"), vec![0u8; 4096]).unwrap();
    std::fs::write(
        downloads.join("stray.bin.hauler"),
        r#"{"version":1,"url":"u","final_url":"u","size":4096,"etag":null,"last_modified":null,"segments":[]}"#,
    )
    .unwrap();

    let page: toml::Table = std::fs::read_to_string(root().join("uq/pages/hauler/page.toml")).unwrap().parse().unwrap();
    let mut commands: Vec<_> =
        page["command"].as_array().unwrap().iter().map(|c| c.as_table().unwrap().clone()).collect();
    commands.sort_by_key(|c| DESTRUCTIVE.contains(&c["id"].as_str().unwrap()));
    let update = std::env::var_os("UPDATE_FIXTURES").is_some();
    let mut problems = Vec::new();
    for c in &commands {
        let id = c["id"].as_str().unwrap();
        let argv: Vec<_> = c["argv"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect();
        assert_eq!(argv[0], "/usr/bin/hauler", "{id}");
        let mut args: Vec<String> = argv[1..].to_vec();
        for a in c.get("args").and_then(|a| a.as_array()).into_iter().flatten() {
            args.push(sample(id, a["name"].as_str().unwrap(), done.id, failed.id));
        }
        let run = hauler(&socket, &args);
        assert_ne!(run.code, 2, "{id}: `hauler {}` is not a command line hauler accepts", args.join(" "));
        if c["output"].as_str() == Some("json") {
            let doc: Value =
                serde_json::from_str(&run.stdout).unwrap_or_else(|e| panic!("{id}: not JSON ({e}): {}", run.stdout));
            if !id.starts_with("preview-") {
                assert_eq!(doc.get("ok"), Some(&Value::Bool(true)), "{id}: {doc}");
            }
            assert_eq!(run.code, 0, "{id}: {doc}");
            if let Some((_, file)) = FIXTURES.iter().find(|(name, _)| *name == id) {
                let path = root().join("tests/uq/fixtures").join(file);
                let normalized = normalize(&run.stdout, &downloads);
                if update {
                    let doc: Value = serde_json::from_str(&normalized).unwrap();
                    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
                } else {
                    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                    let got: Value = serde_json::from_str(&normalized).unwrap();
                    differences(id, &shape(&got), &shape(&want), &mut problems);
                }
            }
        }
    }
    // A daemon that isn't there: what the page shows when it can't reach it.
    stop.cancel();
    daemon.await.unwrap().unwrap();
    let down = hauler(&socket, &["status".into(), "--json".into()]);
    assert_eq!(down.code, 1);
    let doc: Value = serde_json::from_str(&down.stdout).unwrap();
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["code"], "daemon_down");
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(root().join("tests/uq/fixtures/daemon-down.json")).unwrap())
            .unwrap();
    differences("daemon-down", &shape(&doc), &shape(&fixture), &mut problems);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        problems.is_empty(),
        "the fixtures in tests/uq/fixtures no longer match the command line:\n{}",
        problems.join("\n")
    );
}
