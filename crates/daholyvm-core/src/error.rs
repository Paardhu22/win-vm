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

    #[error("failed to write `{path}`: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A name that cannot safely become a directory name.
    #[error("invalid virtual machine name `{name}`: {reason}")]
    InvalidName { name: String, reason: &'static str },

    /// A configuration value outside the range QEMU can be asked for.
    #[error("invalid configuration: `{field}` {problem}")]
    InvalidConfig {
        field: &'static str,
        problem: String,
    },

    #[error("could not parse `{path}`: {source}")]
    ConfigSyntax {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("could not serialize configuration: {0}")]
    ConfigSerialize(#[from] toml::ser::Error),

    /// Without a home directory there is nowhere to keep virtual machines.
    #[error("cannot locate a home directory: neither $XDG_DATA_HOME nor $HOME is set")]
    NoHome,

    #[error("no virtual machine named `{0}`")]
    NoSuchVm(String),

    #[error("a virtual machine named `{0}` already exists")]
    VmExists(String),

    #[error("refusing to overwrite the existing disk image at `{0}`")]
    DiskExists(PathBuf),

    #[error("installation medium `{0}` does not exist")]
    MissingIso(PathBuf),

    /// Preflight found a blocker, so there is no point trying to launch.
    #[error("this host cannot launch a virtual machine: {0}")]
    HostNotReady(&'static str),

    #[error("`{binary}` was not found on PATH")]
    MissingBinary { binary: &'static str },

    /// The VM asks for a TPM but the host cannot provide one.
    #[error("this virtual machine is configured with a TPM, but {remedy}")]
    TpmUnavailable { remedy: String },

    /// A unix socket path longer than the kernel accepts.
    #[error("the socket path `{path}` is {length} bytes, over the {limit} byte limit")]
    SocketPathTooLong {
        path: PathBuf,
        length: usize,
        limit: usize,
    },

    #[error("`{binary}` did not create its socket at `{path}` within {seconds} seconds")]
    HelperTimeout {
        binary: &'static str,
        path: PathBuf,
        seconds: u64,
    },

    #[error("failed to run `{binary}`: {source}")]
    Spawn {
        binary: String,
        #[source]
        source: std::io::Error,
    },

    #[error("`{binary}` failed ({status}){detail}")]
    CommandFailed {
        binary: String,
        status: String,
        /// Captured stderr, already prefixed for display, or empty.
        detail: String,
    },
}

impl Error {
    pub fn read(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Read {
            path: path.into(),
            source,
        }
    }

    pub fn write(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Write {
            path: path.into(),
            source,
        }
    }
}
