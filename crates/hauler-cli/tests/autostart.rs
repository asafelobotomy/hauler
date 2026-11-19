//! `hauler` starts `haulerd` by itself when nothing is listening (the settings page relies on it), and
//! `HAULER_NO_AUTOSTART` turns that off.

use std::path::PathBuf;
use std::process::Command;

fn run(runtime: &std::path::Path, extra: &[(&str, &str)]) -> (i32, String) {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_hauler"));
    let mut cmd = Command::new(&exe);
    cmd.args(["status", "--json"])
        .env("XDG_RUNTIME_DIR", runtime)
        .env("XDG_DATA_HOME", runtime.join("data"))
        .env("XDG_CONFIG_HOME", runtime.join("config"))
        .env("XDG_STATE_HOME", runtime.join("state"))
        .env("HOME", runtime)
        .env_remove("HAULER_SOCKET")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env("PATH", format!("{}:/usr/bin:/bin", exe.parent().unwrap().display()));
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    (out.status.code().unwrap(), String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn starts_the_daemon_on_demand() {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_hauler"));
    if !exe.with_file_name("haulerd").is_file() {
        eprintln!("skipped: build haulerd first (cargo build --workspace)");
        return;
    }
    let dir = std::env::temp_dir().join(format!("hauler-autostart-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();

    let (code, out) = run(&dir, &[("HAULER_NO_AUTOSTART", "1")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("daemon_down"), "{out}");

    let (code, out) = run(&dir, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("\"ok\":true"), "{out}");

    // It stays up for the next command.
    let (code, _) = run(&dir, &[("HAULER_NO_AUTOSTART", "1")]);
    assert_eq!(code, 0);

    let _ = Command::new("pkill").args(["-f", &format!("{}/data", dir.display())]).status();
    let _ = Command::new("pkill").args(["-u", &unsafe { libc::getuid() }.to_string(), "-x", "haulerd"]).status();
    let _ = std::fs::remove_dir_all(&dir);
}
