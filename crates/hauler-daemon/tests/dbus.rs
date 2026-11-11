//! `org.hauler.Hauler1` and the uQ launcher provider on a private session
//! bus: properties, methods, the `Finished` signal, the once-a-second limit
//! on `PropertiesChanged`, and the provider's search reply.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use hauler_daemon::bus::{self, BUS_NAME, INTERFACE, PATH, PROVIDER_PATH};
use hauler_daemon::store::Store;
use hauler_daemon::{Manager, check_url};
use hauler_ipc::SettingsPatch;
use hauler_testserver::{Config as ServerConfig, spawn};
use tokio_util::sync::CancellationToken;
use zbus::OwnedVariant;

const MIB: u64 = 1024 * 1024;

/// One PropertiesChanged: when it arrived, and what it carried.
type Changed = (Instant, HashMap<String, OwnedVariant>);

#[zbus::proxy(
    interface = "org.hauler.Hauler1",
    default_service = "org.hauler.Hauler1",
    default_path = "/org/hauler/Hauler1"
)]
trait Hauler1 {
    fn pause_all(&self) -> zbus::Result<()>;
    fn resume_all(&self) -> zbus::Result<()>;
    fn add(&self, url: &str) -> zbus::Result<()>;
    fn queue(&self) -> zbus::Result<HashMap<String, OwnedVariant>>;
    #[zbus(property)]
    fn active_count(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn queued_count(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn bytes_done(&self) -> zbus::Result<u64>;
    #[zbus(property)]
    fn bytes_total(&self) -> zbus::Result<u64>;
    #[zbus(signal)]
    fn finished(&self, id: String, name: String, ok: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.uq.Launcher1.Provider",
    default_service = "org.hauler.Hauler1",
    default_path = "/org/uq/Launcher1"
)]
trait Provider {
    fn search(&self, query: &str) -> zbus::Result<Vec<(String, String, String, String, String, f64)>>;
    fn activate(&self, id: &str, action: &str) -> zbus::Result<()>;
}

/// A `dbus-daemon` of our own, stopped on drop.
struct PrivateBus {
    child: Child,
    address: String,
}

impl PrivateBus {
    fn start(dir: &Path) -> Option<Self> {
        let address = format!("unix:path={}", dir.join("bus").display());
        let child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
            .arg(format!("--address={address}"))
            .stdout(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => panic!("dbus-daemon: {e}"),
        };
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        assert!(line.starts_with("unix:"), "dbus-daemon printed {line:?}");
        Some(Self { child, address: line.trim().to_string() })
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("haulerd-dbus-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn connect(address: &str) -> zbus::Connection {
    zbus::connection::Builder::address(address).build().await.unwrap()
}

async fn eventually<T: std::fmt::Debug>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The D-Bus error name, which zbus puts first in the message.
fn error_name(e: zbus::Error) -> String {
    e.to_string().split(':').next().unwrap_or_default().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_interface_and_provider_on_a_private_bus() {
    let dir = tempdir("iface");
    let Some(dbus) = PrivateBus::start(&dir) else {
        eprintln!("dbus-daemon is not installed; skipping");
        return;
    };
    let size = 3 * MIB;
    let server = spawn(ServerConfig { size, bytes_per_sec: Some(MIB), ..Default::default() }).await.unwrap();
    let small =
        spawn(ServerConfig { size: 64 * 1024, filename: Some("two.bin".into()), ..Default::default() }).await.unwrap();

    let downloads = dir.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let manager = Manager::new(Store::in_memory().unwrap(), &downloads).unwrap();
    manager.set_settings(SettingsPatch { connections: Some(1), ..Default::default() }).unwrap();
    let stop = CancellationToken::new();
    let scheduler = tokio::spawn(manager.clone().run(stop.clone()));
    let service_conn = connect(&dbus.address).await;
    let service = {
        let (conn, manager, stop) = (service_conn.clone(), manager.clone(), stop.clone());
        tokio::spawn(async move { bus::serve(&conn, manager, stop).await })
    };

    let conn = connect(&dbus.address).await;
    let dbus_proxy = zbus::fdo::DBusProxy::new(&conn).await.unwrap();
    eventually("the bus name", async || {
        dbus_proxy.name_has_owner(BUS_NAME.try_into().unwrap()).await.unwrap().then_some(())
    })
    .await;
    let hauler = Hauler1Proxy::builder(&conn).cache_properties(zbus::proxy::CacheProperties::No).build().await.unwrap();
    let provider = ProviderProxy::builder(&conn).path(PROVIDER_PATH).build().await.unwrap();

    // Every PropertiesChanged, with when it arrived.
    let changes: Arc<Mutex<Vec<Changed>>> = Arc::default();
    let props = zbus::fdo::PropertiesProxy::builder(&conn).destination(BUS_NAME).path(PATH);
    let mut changed_stream = props.build().await.unwrap().receive_properties_changed().await.unwrap();
    let recorder = {
        let changes = changes.clone();
        tokio::spawn(async move {
            while let Some(signal) = changed_stream.next().await {
                let args = signal.args().unwrap();
                assert_eq!(args.interface_name().as_str(), INTERFACE);
                let map = args
                    .changed_properties()
                    .iter()
                    .map(|(k, v)| (k.to_string(), OwnedVariant::try_from(v.clone()).unwrap()))
                    .collect();
                changes.lock().unwrap().push((Instant::now(), map));
            }
        })
    };
    let mut finished = hauler.receive_finished().await.unwrap();

    // Idle: everything zero.
    assert_eq!(hauler.active_count().await.unwrap(), 0);
    assert_eq!(hauler.queued_count().await.unwrap(), 0);
    assert_eq!(hauler.bytes_done().await.unwrap(), 0);
    assert_eq!(hauler.bytes_total().await.unwrap(), 0);
    assert!(hauler.queue().await.unwrap().is_empty());

    // Add checks the URL as the CLI's `add` does.
    for bad in ["ftp://example.com/a.iso", "not a url", ""] {
        let e = hauler.add(bad).await.unwrap_err();
        assert_eq!(error_name(e), "org.freedesktop.DBus.Error.InvalidArgs", "{bad:?}");
        assert!(check_url(bad).is_err());
    }
    assert!(manager.list().is_empty());

    hauler.add(&server.url()).await.unwrap();
    eventually("a running download with a known size", async || {
        let running = hauler.active_count().await.unwrap() == 1;
        (running && hauler.bytes_total().await.unwrap() == size).then_some(())
    })
    .await;
    eventually("some progress", async || (hauler.bytes_done().await.unwrap() > 0).then_some(())).await;

    let queue = hauler.queue().await.unwrap();
    assert_eq!(queue.len(), 1);
    let entry: HashMap<String, OwnedVariant> = queue["000"].try_clone().unwrap().try_into().unwrap();
    assert_eq!(String::try_from(entry["state"].try_clone().unwrap()).unwrap(), "running");
    assert_eq!(u64::try_from(&*entry["size"]).unwrap(), size);

    // Pause and resume everything.
    hauler.pause_all().await.unwrap();
    eventually("everything paused", async || (hauler.active_count().await.unwrap() == 0).then_some(())).await;
    let queue = hauler.queue().await.unwrap();
    let entry: HashMap<String, OwnedVariant> = queue["000"].try_clone().unwrap().try_into().unwrap();
    assert_eq!(String::try_from(entry["state"].try_clone().unwrap()).unwrap(), "paused");
    assert_eq!(hauler.bytes_total().await.unwrap(), 0, "paused downloads are not counted");
    hauler.resume_all().await.unwrap();

    // The provider: only a whole http(s) URL gets a row.
    assert!(provider.search("firefox").await.unwrap().is_empty());
    assert!(provider.search("ftp://example.com/a").await.unwrap().is_empty());
    let rows = provider.search(&format!("  {}  ", small.url())).await.unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let (id, title, subtitle, icon, actions, score) = rows[0].clone();
    assert_eq!(title, "Download with hauler");
    assert_eq!(subtitle, check_url(&small.url()).unwrap().to_string());
    assert_eq!(icon, "folder-download");
    assert_eq!(serde_json::from_str::<Vec<String>>(&actions).unwrap(), ["download"]);
    assert!((0.0..=1.0).contains(&score));
    assert!(provider.activate(&id, "rm -rf").await.is_err());
    provider.activate(&id, "download").await.unwrap();

    // Both finish, each with one Finished signal.
    let mut names = Vec::new();
    while names.len() < 2 {
        let signal = tokio::time::timeout(Duration::from_secs(30), finished.next()).await.unwrap().unwrap();
        let args = signal.args().unwrap();
        assert!(args.ok, "{} failed", args.name);
        assert!(manager.list().iter().any(|t| t.id.to_string() == args.id));
        names.push(args.name.clone());
    }
    names.sort();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"two.bin".to_string()), "{names:?}");

    // Settled: the last change sent says nothing is left.
    eventually("idle properties", async || {
        let last = changes
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|(_, m)| m.get("ActiveCount").map(|v| u32::try_from(&**v).unwrap()));
        (last == Some(0) && hauler.queued_count().await.unwrap() == 0).then_some(())
    })
    .await;
    assert_eq!(hauler.bytes_total().await.unwrap(), 0);

    // At most one PropertiesChanged a second (allowing for delivery jitter),
    // each carrying only the four properties.
    let seen = changes.lock().unwrap().clone();
    assert!(seen.len() >= 3, "expected several updates, got {}", seen.len());
    for pair in seen.windows(2) {
        let gap = pair[1].0 - pair[0].0;
        assert!(gap >= Duration::from_millis(800), "two PropertiesChanged {gap:?} apart");
    }
    for (_, m) in &seen {
        assert!(!m.is_empty());
        for k in m.keys() {
            assert!(["ActiveCount", "QueuedCount", "BytesDone", "BytesTotal"].contains(&k.as_str()), "{k}");
        }
    }

    // Stopping gives the name up.
    stop.cancel();
    service.await.unwrap().unwrap();
    scheduler.await.unwrap();
    eventually("the name to go", async || {
        (!dbus_proxy.name_has_owner(BUS_NAME.try_into().unwrap()).await.unwrap()).then_some(())
    })
    .await;
    recorder.abort();
    std::fs::remove_dir_all(&dir).unwrap();
}
