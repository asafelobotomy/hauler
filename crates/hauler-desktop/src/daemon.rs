//! The `haulerd` this app starts when socket activation is not set up, and
//! stopping it again when the app exits.

use std::path::PathBuf;
use std::time::Duration;

/// Start `haulerd` when socket activation is not set up: next to this
/// binary first (a development build), then from `$PATH`.
///
/// Its output goes where ours does, so a failure to start is visible, and a
/// thread waits on it so it never lingers as a zombie if it exits early.
pub fn spawn_daemon() {
    let sibling = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("haulerd")));
    let program = sibling.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("haulerd"));
    let mut command = std::process::Command::new(&program);
    command.stdin(std::process::Stdio::null());
    // If this app dies without stopping it (a crash, a kill), the kernel
    // sends the daemon SIGTERM so it saves progress and exits instead of
    // running on orphaned, possibly from an unmounted AppImage.
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        use std::os::unix::process::CommandExt;
        let parent = libc::getpid();
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have died before prctl took effect.
            if libc::getppid() != parent {
                libc::raise(libc::SIGTERM);
            }
            Ok(())
        });
    }
    match command.spawn() {
        Ok(mut child) => {
            let pid = child.id();
            let reaper = std::thread::spawn(move || {
                if let Ok(status) = child.wait()
                    && !status.success()
                {
                    eprintln!("haulerd exited: {status}");
                }
            });
            *SPAWNED.lock().unwrap() = Some((pid, reaper));
        }
        Err(e) => eprintln!("could not start {}: {e}", program.display()),
    }
}

/// The daemon this app started, if any: its pid and the thread waiting on it.
static SPAWNED: std::sync::Mutex<Option<(u32, std::thread::JoinHandle<()>)>> = std::sync::Mutex::new(None);

/// Stop the daemon this app started, letting it save its downloads' progress.
/// Run from an AppImage, its binary disappears when the app exits, so it must
/// not outlive us. A daemon from systemd or started elsewhere is left alone.
pub fn stop_spawned_daemon() {
    let Some((pid, reaper)) = SPAWNED.lock().unwrap().take() else { return };
    // SAFETY: plain kill(2) on a pid we spawned and have not reaped yet.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    // haulerd allows downloads up to 10 s to flush; wait a little longer.
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    while !reaper.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}
