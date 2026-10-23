//! hauler's download engine: segmented, resumable HTTP(S) downloads.
//!
//! ```no_run
//! # async fn demo() -> hauler_engine::Result<()> {
//! use hauler_engine::{Options, Outcome, client, prepare};
//! use tokio_util::sync::CancellationToken;
//!
//! let task = prepare(client()?, "https://example.com/big.iso", ".".as_ref(), Options::default()).await?;
//! match task.run(CancellationToken::new()).await? {
//!     Outcome::Completed(path) => println!("saved {}", path.display()),
//!     Outcome::Paused => println!("paused"),
//! }
//! # Ok(()) }
//! ```

mod checksum;
mod download;
mod error;
pub mod plan;
mod probe;
mod state;

pub use checksum::{Checksum, sha256_file};
pub use download::{Options, Outcome, Progress, Task, client, prepare};
pub use error::{Error, Result};
pub use probe::{Probe, probe};
pub use reqwest::header;
pub use state::{ResumeState, SegmentState};
pub use tokio_util::sync::CancellationToken;
