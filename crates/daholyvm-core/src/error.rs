//! Error types shared across the core crate.

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// DA-HOLY-VM is Linux-first and does not target other platforms.
    #[error("DA-HOLY-VM requires Linux (this build targets `{0}`)")]
    UnsupportedPlatform(&'static str),

    #[error("failed to read `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl Error {
    pub fn read(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Read {
            path: path.into(),
            source,
        }
    }
}
