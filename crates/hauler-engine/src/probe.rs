//! Find out size, range support and validators with a one-byte range request.

use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Client, Response, StatusCode};

use crate::error::{Error, Result};

#[derive(Clone, Debug)]
pub struct Probe {
    /// URL after redirects; segment requests go here directly.
    pub final_url: String,
    pub size: Option<u64>,
    /// The server answered a range request with 206.
    pub ranges: bool,
    /// Strong ETag only; weak validators cannot be used with `If-Range`.
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// From `Content-Disposition`, falling back to the URL path.
    pub filename: Option<String>,
}

impl Probe {
    /// The validator to send in `If-Range`, strongest first.
    pub fn validator(&self) -> Option<&str> {
        self.etag.as_deref().or(self.last_modified.as_deref())
    }
}

pub async fn probe(client: &Client, url: &str, headers: &HeaderMap) -> Result<Probe> {
    let resp = client.get(url).headers(headers.clone()).header(header::RANGE, "bytes=0-0").send().await?;
    let status = resp.status();
    if !(status == StatusCode::OK || status == StatusCode::PARTIAL_CONTENT) {
        return Err(status_error(&resp));
    }

    let h = resp.headers();
    let (ranges, size) = if status == StatusCode::PARTIAL_CONTENT {
        let total = h.get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(content_range_total);
        (total.is_some(), total)
    } else {
        (false, resp.content_length())
    };
    let etag = h.get(header::ETAG).and_then(|v| v.to_str().ok()).filter(|v| !v.starts_with("W/")).map(str::to_owned);
    let last_modified = h.get(header::LAST_MODIFIED).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let final_url = resp.url().to_string();
    let filename = h
        .get(header::CONTENT_DISPOSITION)
        .and_then(disposition_filename)
        .or_else(|| url_filename(resp.url()))
        .map(|n| sanitize_filename(&n))
        .filter(|n| !n.is_empty());

    Ok(Probe { final_url, size, ranges, etag, last_modified, filename })
}

pub(crate) fn status_error(resp: &Response) -> Error {
    let retry_after = resp
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(std::time::Duration::from_secs);
    Error::Status { status: resp.status(), retry_after }
}

/// `bytes 0-0/12345` -> 12345. `*` means the size is unknown.
fn content_range_total(value: &str) -> Option<u64> {
    value.strip_prefix("bytes ")?.rsplit_once('/')?.1.parse().ok()
}

fn disposition_filename(value: &HeaderValue) -> Option<String> {
    let value = String::from_utf8_lossy(value.as_bytes());
    let mut plain = None;
    for part in value.split(';').map(str::trim) {
        if let Some(v) = part.strip_prefix("filename*=") {
            // RFC 5987: charset'lang'percent-encoded
            let encoded = v.splitn(3, '\'').nth(2)?;
            return Some(percent_decode(encoded));
        }
        if let Some(v) = part.strip_prefix("filename=") {
            plain = Some(v.trim_matches('"').to_owned());
        }
    }
    plain
}

fn url_filename(url: &reqwest::Url) -> Option<String> {
    let last = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    Some(percent_decode(last))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Keep only the last path component and drop characters that are unsafe in
/// file names.
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control() && *c != '\0').collect();
    let trimmed = cleaned.trim().trim_start_matches('.');
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_range() {
        assert_eq!(content_range_total("bytes 0-0/1234"), Some(1234));
        assert_eq!(content_range_total("bytes 0-0/*"), None);
    }

    #[test]
    fn parses_disposition() {
        let v = HeaderValue::from_static("attachment; filename=\"report.pdf\"");
        assert_eq!(disposition_filename(&v).as_deref(), Some("report.pdf"));
        let v = HeaderValue::from_static("attachment; filename=x; filename*=UTF-8''na%C3%AFve%20file.txt");
        assert_eq!(disposition_filename(&v).as_deref(), Some("naïve file.txt"));
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename(".hidden"), "hidden");
        assert_eq!(sanitize_filename("a\\b.zip"), "b.zip");
    }
}
