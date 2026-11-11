//! uQ launcher provider: `org.uq.Launcher1.Provider` at `/org/uq/Launcher1`.
//! A query that is an http(s) URL gets one row, "Download with hauler";
//! choosing it queues the URL. Nothing from the query is ever run.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use hauler_ipc::AddParams;

use super::hauler1::to_fdo;
use crate::manager::{Manager, check_url};

/// `(id, title, subtitle, icon, actions as a JSON list, score)`.
pub type Row = (String, String, String, String, String, f64);

pub const TITLE: &str = "Download with hauler";
pub const ACTION: &str = "download";
const ICON: &str = "folder-download";
/// A pasted URL is a strong signal; uQ still ranks it below matching apps.
const SCORE: f64 = 0.95;
/// uQ drops rows whose id is longer than this, so longer URLs get a token.
pub const MAX_ID_BYTES: usize = 512;
const TOKEN_PREFIX: &str = "hauler-url:";
/// Tokens for long URLs kept from recent searches.
const TOKENS_KEPT: usize = 32;

/// The URL to offer for `query`: the whole query, trimmed, when it is a URL
/// `Add` would take; never a URL inside a sentence.
pub fn offer(query: &str) -> Option<String> {
    let q = query.trim();
    if q.is_empty() || q.chars().any(char::is_whitespace) {
        return None;
    }
    check_url(q).ok().map(|u| u.to_string())
}

pub struct LauncherProvider {
    manager: Arc<Manager>,
    /// `(token, url)` for URLs too long to be an id, newest last.
    tokens: Mutex<(u64, VecDeque<(String, String)>)>,
}

impl LauncherProvider {
    pub fn new(manager: Arc<Manager>) -> Self {
        Self { manager, tokens: Mutex::new((0, VecDeque::new())) }
    }

    fn id_for(&self, url: String) -> String {
        if url.len() <= MAX_ID_BYTES && !url.starts_with(TOKEN_PREFIX) {
            return url;
        }
        let mut guard = self.tokens.lock().unwrap();
        let (counter, kept) = &mut *guard;
        if let Some((token, _)) = kept.iter().find(|(_, u)| *u == url) {
            return token.clone();
        }
        *counter += 1;
        let token = format!("{TOKEN_PREFIX}{counter}");
        kept.push_back((token.clone(), url));
        if kept.len() > TOKENS_KEPT {
            kept.pop_front();
        }
        token
    }

    fn url_for(&self, id: &str) -> Option<String> {
        if id.starts_with(TOKEN_PREFIX) {
            let guard = self.tokens.lock().unwrap();
            return guard.1.iter().find(|(t, _)| t == id).map(|(_, u)| u.clone());
        }
        offer(id)
    }
}

#[zbus::interface(name = "org.uq.Launcher1.Provider")]
impl LauncherProvider {
    pub fn search(&self, query: String) -> Vec<Row> {
        let Some(url) = offer(&query) else { return Vec::new() };
        let id = self.id_for(url.clone());
        vec![(id, TITLE.into(), url, ICON.into(), format!("[\"{ACTION}\"]"), SCORE)]
    }

    pub fn activate(&self, id: String, action: String) -> zbus::fdo::Result<()> {
        if !action.is_empty() && action != ACTION {
            return Err(zbus::fdo::Error::InvalidArgs(format!("unknown action {action:?}")));
        }
        let url = self.url_for(&id).ok_or_else(|| zbus::fdo::Error::InvalidArgs("unknown item".into()))?;
        self.manager.add(AddParams { url, ..Default::default() }).map(|_| ()).map_err(to_fdo)
    }
}
