//! SQLite persistence for tasks and settings.
//!
//! Per-segment progress stays in the engine's `.hauler` file beside each
//! download, which is written crash-safely on its own. This database holds
//! the queue: what to download, where, with which options, and how far it got.

use std::path::Path;
use std::sync::Mutex;

use hauler_ipc::{Settings, TaskId, TaskInfo, TaskState};
use rusqlite::{Connection, OptionalExtension, params};

/// Options a task keeps across restarts that are not part of [`TaskInfo`].
#[derive(Clone, Debug, Default)]
pub struct TaskSpec {
    pub filename: Option<String>,
    pub headers: Vec<(String, String)>,
    pub checksum: Option<String>,
    pub connections: Option<u16>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tasks (
    id          INTEGER PRIMARY KEY,
    url         TEXT NOT NULL,
    dir         TEXT NOT NULL,
    filename    TEXT,
    headers     TEXT NOT NULL DEFAULT '[]',
    checksum    TEXT,
    connections INTEGER,
    speed_limit INTEGER NOT NULL DEFAULT 0,
    priority    INTEGER NOT NULL DEFAULT 0,
    state       TEXT NOT NULL,
    size        INTEGER,
    received    INTEGER NOT NULL DEFAULT 0,
    resumable   INTEGER,
    error       TEXT,
    added_at    INTEGER NOT NULL,
    finished_at INTEGER
);
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

impl Store {
    /// The database holds request headers such as cookies, so it is made
    /// readable by this user only. SQLite gives the `-wal` and `-shm` files
    /// the same mode as the database.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn insert(&self, info: &TaskInfo, spec: &TaskSpec) -> rusqlite::Result<TaskId> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tasks (url, dir, filename, headers, checksum, connections, speed_limit, priority, state, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                info.url,
                info.dir,
                spec.filename,
                serde_json::to_string(&spec.headers).unwrap(),
                spec.checksum,
                spec.connections,
                info.limit as i64,
                info.priority,
                info.state.as_str(),
                info.added_at,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Save everything that can change while a task lives.
    pub fn update(&self, info: &TaskInfo, spec: &TaskSpec) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE tasks SET filename = ?2, speed_limit = ?3, priority = ?4, state = ?5, size = ?6,
                 received = ?7, resumable = ?8, error = ?9, finished_at = ?10
             WHERE id = ?1",
            params![
                info.id,
                spec.filename,
                info.limit as i64,
                info.priority,
                info.state.as_str(),
                info.size.map(|s| s as i64),
                info.received as i64,
                info.resumable,
                info.error,
                info.finished_at,
            ],
        )?;
        Ok(())
    }

    /// Cheap progress checkpoint for many running tasks at once.
    pub fn update_received(&self, rows: &[(TaskId, u64)]) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached("UPDATE tasks SET received = ?2 WHERE id = ?1")?;
            for (id, received) in rows {
                stmt.execute(params![id, *received as i64])?;
            }
        }
        tx.commit()
    }

    pub fn delete(&self, id: TaskId) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute("DELETE FROM tasks WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn load_all(&self) -> rusqlite::Result<Vec<(TaskInfo, TaskSpec)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, url, dir, filename, headers, checksum, connections, speed_limit, priority, state,
                    size, received, resumable, error, added_at, finished_at
             FROM tasks ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            let filename: Option<String> = r.get(3)?;
            let headers: String = r.get(4)?;
            let state: String = r.get(9)?;
            let info = TaskInfo {
                id: r.get(0)?,
                url: r.get(1)?,
                dir: r.get(2)?,
                name: filename.clone(),
                state: TaskState::parse(&state).unwrap_or(TaskState::Queued),
                size: r.get::<_, Option<i64>>(10)?.map(|s| s as u64),
                received: r.get::<_, i64>(11)? as u64,
                speed: 0,
                connections: 0,
                limit: r.get::<_, i64>(7)? as u64,
                priority: r.get(8)?,
                resumable: r.get(12)?,
                error: r.get(13)?,
                added_at: r.get(14)?,
                finished_at: r.get(15)?,
            };
            let spec = TaskSpec {
                filename,
                headers: serde_json::from_str(&headers).unwrap_or_default(),
                checksum: r.get(5)?,
                connections: r.get(6)?,
            };
            Ok((info, spec))
        })?;
        rows.collect()
    }

    pub fn load_settings(&self) -> rusqlite::Result<Option<Settings>> {
        let conn = self.conn.lock().unwrap();
        let json: Option<String> =
            conn.query_row("SELECT value FROM settings WHERE key = 'settings'", [], |r| r.get(0)).optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    pub fn save_settings(&self, settings: &Settings) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO settings (key, value) VALUES ('settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [serde_json::to_string(settings).unwrap()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> TaskInfo {
        TaskInfo {
            id: 0,
            url: "https://example.com/a.iso".into(),
            dir: "/tmp".into(),
            name: None,
            state: TaskState::Queued,
            size: None,
            received: 0,
            speed: 0,
            connections: 0,
            limit: 0,
            priority: 0,
            resumable: None,
            error: None,
            added_at: 1,
            finished_at: None,
        }
    }

    #[test]
    fn database_files_are_private() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("hauler-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("hauler.db");
        // An existing world-readable database is tightened too.
        std::fs::write(&path, b"").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let store = Store::open(&path).unwrap();
        store.insert(&info(), &TaskSpec::default()).unwrap();
        for f in ["hauler.db", "hauler.db-wal"] {
            assert_eq!(std::fs::metadata(dir.join(f)).unwrap().mode() & 0o777, 0o600, "{f}");
        }
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn round_trips_tasks_and_settings() {
        let store = Store::in_memory().unwrap();
        let spec = TaskSpec { headers: vec![("Cookie".into(), "a=b".into())], ..Default::default() };
        let id = store.insert(&info(), &spec).unwrap();

        let mut task = info();
        task.id = id;
        task.state = TaskState::Paused;
        task.size = Some(10);
        task.name = Some("a.iso".into());
        store.update(&task, &TaskSpec { filename: Some("a.iso".into()), ..spec }).unwrap();
        store.update_received(&[(id, 5)]).unwrap();

        let (loaded, spec) = store.load_all().unwrap().pop().unwrap();
        assert_eq!(loaded.state, TaskState::Paused);
        assert_eq!(loaded.received, 5);
        assert_eq!(loaded.name.as_deref(), Some("a.iso"));
        assert_eq!(spec.headers, vec![("Cookie".to_string(), "a=b".to_string())]);

        let settings = Settings {
            max_active: 3,
            connections: 8,
            global_limit: 0,
            download_dir: "/tmp".into(),
            pause_on_metered: true,
        };
        store.save_settings(&settings).unwrap();
        assert_eq!(store.load_settings().unwrap(), Some(settings));
    }
}
