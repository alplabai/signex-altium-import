use std::path::PathBuf;

/// Everything that can go wrong while reading an Altium file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: not an OLE compound document ({source})")]
    NotCompound {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("missing stream `{0}`")]
    MissingStream(String),
    #[error("stream `{stream}` is truncated at byte {offset}: {what}")]
    Truncated {
        stream: String,
        offset: usize,
        what: &'static str,
    },
    #[error("{0}")]
    Malformed(String),
}

pub type Result<T> = std::result::Result<T, Error>;
