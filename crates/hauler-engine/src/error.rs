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
            Error::Http(e) => !e.is_builder() && !e.is_redirect(),
            Error::Status { status, .. } => {
                *status == StatusCode::REQUEST_TIMEOUT
                    || *status == StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error()
            }
            Error::Incomplete | Error::Stalled(_) => true,
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
