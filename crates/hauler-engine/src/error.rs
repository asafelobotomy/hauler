use std::time::Duration;

use reqwest::StatusCode;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("server replied {status}")]
    Status { status: StatusCode, retry_after: Option<Duration> },
    #[error("server sent an unexpected range: {0}")]
    BadRange(String),
    #[error("connection closed before the range was complete")]
    Incomplete,
    #[error("no data received for {0:?}")]
    Stalled(Duration),
    /// Sent elsewhere instead of getting the range: how servers that allow
    /// only a few connections per client turn away the rest.
    #[error("the server refused another connection and sent {0} instead")]
    Refused(String),
    #[error("the file changed on the server since the download started")]
    Changed,
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("invalid checksum {0:?}; use sha256:<hex>")]
    BadChecksum(String),
    #[error("unreadable resume state: {0}")]
    State(String),
    #[error("gave up after {attempts} attempts: {last}")]
    RetriesExhausted { attempts: u32, last: Box<Error> },
}

impl Error {
    /// Whether retrying the same request might succeed.
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Http(e) => http_retryable(e),
            Error::Status { status, .. } => {
                *status == StatusCode::REQUEST_TIMEOUT
                    || *status == StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error()
            }
            Error::Incomplete | Error::Stalled(_) | Error::Refused(_) => true,
            _ => false,
        }
    }

    /// Whether the server turned this connection away for being one too
    /// many, rather than failing it.
    pub(crate) fn is_turned_away(&self) -> bool {
        match self {
            Error::Refused(_) => true,
            Error::Status { status, .. } => {
                *status == StatusCode::TOO_MANY_REQUESTS || *status == StatusCode::SERVICE_UNAVAILABLE
            }
            _ => false,
        }
    }

    pub(crate) fn retry_after(&self) -> Option<Duration> {
        match self {
            Error::Status { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

/// Transient network trouble is worth retrying; a bad certificate or a host
/// that does not exist is not, and retrying would only hold a queue slot for
/// minutes before failing anyway.
fn http_retryable(e: &reqwest::Error) -> bool {
    if e.is_builder() || e.is_redirect() {
        return false;
    }
    // Certificate and DNS failures happen while connecting; anything later
    // (a reset or garbled body mid-transfer) is worth another try.
    if e.is_timeout() || !e.is_connect() {
        return true;
    }
    let permanent = |x: &(dyn std::error::Error + 'static)| {
        is_tls(x) || x.downcast_ref::<std::io::Error>().is_some_and(unknown_host)
    };
    !chain_has(e, &permanent)
}

/// Whether `pred` holds for `e` or anything it wraps. io::Errors hide their
/// payload from `source()` (TLS failures arrive as an io::Error inside an
/// io::Error), so look inside them too.
fn chain_has(e: &(dyn std::error::Error + 'static), pred: &dyn Fn(&(dyn std::error::Error + 'static)) -> bool) -> bool {
    let mut current = Some(e);
    while let Some(x) = current {
        if pred(x) {
            return true;
        }
        current = match x.downcast_ref::<std::io::Error>().and_then(|io| io.get_ref()) {
            Some(inner) => Some(inner as &(dyn std::error::Error + 'static)),
            None => x.source(),
        };
    }
    false
}

/// Any TLS failure: an invalid, expired or mismatched certificate, or a peer
/// that does not speak TLS. None of these fix themselves on a retry.
fn is_tls(e: &(dyn std::error::Error + 'static)) -> bool {
    e.downcast_ref::<rustls::Error>().is_some()
}

/// getaddrinfo reports a name that does not exist differently from a
/// temporary DNS failure; only the latter can succeed later.
fn unknown_host(io: &std::io::Error) -> bool {
    let msg = io.to_string();
    msg.contains("Name or service not known")
        || msg.contains("No address associated with hostname")
        || msg.contains("nodename nor servname provided")
}

#[cfg(test)]
mod tests {
    use std::io::{Error as IoError, ErrorKind};

    use super::unknown_host;

    #[test]
    fn tells_unknown_hosts_from_temporary_dns_failures() {
        assert!(unknown_host(&IoError::other("failed to lookup address information: Name or service not known")));
        let temporary = "failed to lookup address information: Temporary failure in name resolution";
        assert!(!unknown_host(&IoError::other(temporary)));
        assert!(!unknown_host(&IoError::from(ErrorKind::ConnectionRefused)));
    }
}
