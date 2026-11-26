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
        let cfb = match cfb::CompoundFile::open(std::io::Cursor::new(bytes.clone())) {
            Ok(c) => c,
            Err(first) => {
                // Some writers leave stale entries in the DIFAT beyond the
                // declared FAT sector count (observed in a private corpus
                // file); a reader must stop at the declared count.
                match limit_difat(&bytes)
                    .and_then(|fixed| cfb::CompoundFile::open(std::io::Cursor::new(fixed)).ok())
                {
                    Some(c) => c,
                    None => {
                        return Err(Error::NotCompound {
                            path,
                            source: first,
                        });
                    }
                }
            }
        };
        Ok(Self { path, cfb })
    }

    /// The root entry's modified time, when the file records one after the
    /// Unix epoch (a zero time reads as 1601 and counts as none).
    pub fn modified(&self) -> Option<std::time::SystemTime> {
        let t = self.cfb.root_entry().modified();
        (t > std::time::UNIX_EPOCH).then_some(t)
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

/// The file with every DIFAT entry past the header's declared FAT sector
/// count freed, or `None` when the file has no such entries to free.
fn limit_difat(bytes: &[u8]) -> Option<Vec<u8>> {
    const FREE: u32 = 0xFFFF_FFFF;
    const END: u32 = 0xFFFF_FFFE;
    let rd = |b: &[u8], at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
    };
    if bytes.get(..8)? != [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1] {
        return None;
    }
    let shift = u16::from_le_bytes(bytes.get(0x1E..0x20)?.try_into().ok()?);
    if !(7..=16).contains(&shift) {
        return None;
    }
    let sector = 1usize << shift;
    let declared = rd(bytes, 0x2C)? as usize;
    let mut out = bytes.to_vec();
    let mut changed = false;
    // Header DIFAT: 109 entries.
    for i in 0..109usize {
        if i >= declared {
            let at = 0x4C + i * 4;
            if rd(&out, at)? != FREE {
                out[at..at + 4].copy_from_slice(&FREE.to_le_bytes());
                changed = true;
            }
        }
    }
    // DIFAT sectors: `sector/4 - 1` entries each, then the next sector.
    let per = sector / 4 - 1;
    let mut next = rd(bytes, 0x44)?;
    let mut used = 109usize;
    let mut guard = 0;
    while next < END && guard < 1_000_000 {
        guard += 1;
        let base = 512usize.max(sector) + next as usize * sector;
        for j in 0..per {
            let at = base + j * 4;
            if used + j >= declared && rd(&out, at)? != FREE {
                out[at..at + 4].copy_from_slice(&FREE.to_le_bytes());
                changed = true;
            }
        }
        used += per;
        next = rd(&out, base + per * 4)?;
    }
    changed.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_header_difat_entry_is_freed() {
        let mut c = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
        c.create_stream("/a").unwrap();
        c.flush().unwrap();
        let mut bytes = c.into_inner().into_inner();
        // A stale entry beyond the declared FAT sector count.
        let declared = u32::from_le_bytes(bytes[0x2C..0x30].try_into().unwrap()) as usize;
        let at = 0x4C + (declared + 3) * 4;
        bytes[at..at + 4].copy_from_slice(&7u32.to_le_bytes());
        let fixed = limit_difat(&bytes).expect("something to free");
        assert!(cfb::CompoundFile::open(std::io::Cursor::new(fixed)).is_ok());
        // Nothing stale: nothing to do.
        let mut clean = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
        clean.flush().unwrap();
        assert!(limit_difat(&clean.into_inner().into_inner()).is_none());
        assert!(limit_difat(b"not a compound file").is_none());
    }
}
