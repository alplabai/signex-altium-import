//! The OLE2 / Compound File Binary container every binary Altium file uses.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// An opened Altium file: a tree of storages (directories) and streams.
///
/// The whole file is read into memory up front. Design files are a few MB
/// at most, and owning the bytes keeps the parsers free of I/O.
pub struct Container {
    path: PathBuf,
    cfb: cfb::CompoundFile<std::io::Cursor<Vec<u8>>>,
}

impl Container {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let bytes = std::fs::read(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        Self::from_bytes(path, bytes)
    }

    pub fn from_bytes(path: PathBuf, bytes: Vec<u8>) -> Result<Self> {
        let cfb = cfb::CompoundFile::open(std::io::Cursor::new(bytes)).map_err(|source| {
            Error::NotCompound {
                path: path.clone(),
                source,
            }
        })?;
        Ok(Self { path, cfb })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read a stream by its `/`-separated path, e.g. `"Tracks6/Data"`.
    pub fn stream(&mut self, name: &str) -> Result<Vec<u8>> {
        let full = format!("/{}", name.trim_start_matches('/'));
        if !self.cfb.is_stream(&full) {
            return Err(Error::MissingStream(name.to_string()));
        }
        let mut s = self.cfb.open_stream(&full).map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })?;
        let mut out = Vec::new();
        s.read_to_end(&mut out).map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })?;
        Ok(out)
    }

    /// Like [`Self::stream`], but a missing stream is `None` rather than an error.
    pub fn stream_opt(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
        match self.stream(name) {
            Ok(b) => Ok(Some(b)),
            Err(Error::MissingStream(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Names of the storages directly under the root.
    pub fn root_storages(&self) -> Vec<String> {
        self.children("/", true)
    }

    /// Names of the streams directly under `storage` (`"/"` for the root).
    pub fn streams_in(&self, storage: &str) -> Vec<String> {
        self.children(storage, false)
    }

    fn children(&self, storage: &str, storages: bool) -> Vec<String> {
        let Ok(entries) = self.cfb.read_storage(storage) else {
            return Vec::new();
        };
        entries
            .filter(|e| {
                if storages {
                    e.is_storage()
                } else {
                    e.is_stream()
                }
            })
            .map(|e| e.name().to_string())
            .collect()
    }

    /// Every stream path in the file, depth first. Used by the dump tool.
    pub fn all_streams(&self) -> Vec<String> {
        self.cfb
            .walk()
            .filter(|e| e.is_stream())
            .map(|e| {
                e.path()
                    .to_string_lossy()
                    .trim_start_matches('/')
                    .to_string()
            })
            .collect()
    }
}
