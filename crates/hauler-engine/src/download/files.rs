//! Where a download lives on disk: its part and state files, and finishing.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use tracing::warn;

use super::{Options, Outcome};
use crate::error::{Error, Result};

pub(super) struct Paths {
    pub(super) target: PathBuf,
    pub(super) part: PathBuf,
    pub(super) state: PathBuf,
}

pub(super) fn part_path(target: &Path) -> PathBuf {
    with_suffix(target, ".part")
}

pub(super) fn state_path(target: &Path) -> PathBuf {
    with_suffix(target, ".hauler")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Open the part file and take an exclusive advisory lock on it. `None`
/// when another process or task already holds it.
pub(super) fn lock_part(path: &Path) -> Result<Option<File>> {
    let f = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
    match rustix::fs::flock(&f, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(Some(f)),
        Err(rustix::io::Errno::WOULDBLOCK) => Ok(None),
        // Some network filesystems cannot lock; carry on without it.
        Err(e) => {
            warn!("cannot lock {}: {e}", path.display());
            Ok(Some(f))
        }
    }
}

/// `name.ext` -> `name (1).ext`, the first one with no file, part or state.
pub(super) fn free_name(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (1..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists() && !part_path(p).exists() && !state_path(p).exists())
        .unwrap()
}

pub(super) fn preallocate(file: &File, size: u64) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    if rustix::fs::fallocate(file, rustix::fs::FallocateFlags::empty(), 0, size).is_ok() {
        return Ok(());
    }
    // Filesystems without fallocate (FAT, some network mounts) get a sparse file.
    file.set_len(size)
}

pub(super) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> std::io::Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(std::io::Error::other)?.map_err(Error::from)
}

pub(super) async fn finish(opts: &Options, paths: &Paths) -> Result<Outcome> {
    if let Some(sum) = opts.checksum.clone() {
        let part = paths.part.clone();
        tokio::task::spawn_blocking(move || sum.verify_file(&part)).await.map_err(std::io::Error::other)??;
    }
    fs::rename(&paths.part, &paths.target)?;
    match fs::remove_file(&paths.state) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    Ok(Outcome::Completed(paths.target.clone()))
}
