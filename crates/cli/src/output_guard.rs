//! Undo log for the files a `library` run writes. A failed run (contract
//! section 3.3) removes the library files it wrote and leaves a library that
//! existed before an `--update` as it was: every write and delete goes
//! through the guard, and dropping it uncommitted restores the old state.
//! The report files are written outside the guard; they stay.

#![cfg_attr(not(test), allow(dead_code))]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Default)]
pub struct Guard {
    /// `(path, old bytes)` in the order changed; `None` = did not exist.
    undo: Vec<(PathBuf, Option<Vec<u8>>)>,
    committed: bool,
}

impl Guard {
    pub fn new() -> Self {
        Self::default()
    }

    fn remember(&mut self, path: &Path) -> Result<()> {
        let old = match std::fs::read(path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        self.undo.push((path.to_path_buf(), old));
        Ok(())
    }

    /// Write `bytes` atomically, creating parent folders.
    pub fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        self.remember(path)?;
        signex_types::atomic_io::atomic_write(path, bytes)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Delete a file (a missing one is fine).
    pub fn remove(&mut self, path: &Path) -> Result<()> {
        self.remember(path)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
        }
    }

    /// The run succeeded; keep everything.
    pub fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for (path, old) in self.undo.drain(..).rev() {
            let _ = match old {
                Some(b) => signex_types::atomic_io::atomic_write(&path, &b).map_err(drop),
                None => std::fs::remove_file(&path).map_err(drop),
            };
            // Folders this run created are removed once empty.
            let mut dir = path.parent();
            while let Some(d) = dir {
                if std::fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("snx-guard-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn uncommitted_guard_restores_the_old_state() {
        let d = tmp("undo");
        std::fs::write(d.join("old.txt"), "old").unwrap();
        std::fs::write(d.join("gone.txt"), "keep me").unwrap();
        {
            let mut g = Guard::new();
            g.write(&d.join("old.txt"), b"new").unwrap();
            g.write(&d.join("sub/new.txt"), b"x").unwrap();
            g.remove(&d.join("gone.txt")).unwrap();
        }
        assert_eq!(std::fs::read_to_string(d.join("old.txt")).unwrap(), "old");
        assert_eq!(
            std::fs::read_to_string(d.join("gone.txt")).unwrap(),
            "keep me"
        );
        assert!(!d.join("sub").exists());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn committed_guard_keeps_everything() {
        let d = tmp("commit");
        let mut g = Guard::new();
        g.write(&d.join("a.txt"), b"a").unwrap();
        g.commit();
        assert!(d.join("a.txt").is_file());
        std::fs::remove_dir_all(d).unwrap();
    }
}
