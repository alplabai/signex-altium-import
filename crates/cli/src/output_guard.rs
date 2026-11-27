//! Undo log for the files a `library` run writes. A failed run (contract
//! section 3.3) removes the library files it wrote and leaves a library that
//! existed before an `--update` as it was: every write and delete goes
//! through the guard, and dropping it uncommitted restores the old state.
//! The report files are written outside the guard; they stay.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub struct Guard {
    /// The canonical output folder; nothing outside it is touched.
    root: PathBuf,
    /// `(path, old bytes)` in the order changed; `None` = did not exist.
    undo: Vec<(PathBuf, Option<Vec<u8>>)>,
    committed: bool,
}

impl Guard {
    pub fn new(root: &Path) -> Self {
        Guard {
            root: signex_altium_convert::paths::canonical(root)
                .unwrap_or_else(|_| signex_altium_convert::paths::plain(root.to_path_buf())),
            undo: Vec::new(),
            committed: false,
        }
    }

    /// `path` is under the root by plain names only (no `..`), no folder on
    /// the way is a symlink, and the target, if it exists, is a regular file.
    fn check(&self, path: &Path) -> Result<()> {
        // The root is in the plain form; a caller's path may still be
        // verbatim (`\\?\C:\...` on Windows), so compare the same form.
        let plain = signex_altium_convert::paths::plain(path.to_path_buf());
        let rel = plain
            .strip_prefix(&self.root)
            .map_err(|_| anyhow::anyhow!("{} is outside the output folder", path.display()))?;
        let mut cur = self.root.clone();
        let n = rel.components().count();
        for (i, c) in rel.components().enumerate() {
            let std::path::Component::Normal(name) = c else {
                anyhow::bail!("{} is not a plain path", path.display());
            };
            cur.push(name);
            match std::fs::symlink_metadata(&cur) {
                Ok(m) if m.file_type().is_symlink() => {
                    anyhow::bail!("{} is a symlink; refusing to follow it", cur.display())
                }
                Ok(m) if i + 1 < n && !m.is_dir() => {
                    anyhow::bail!("{} is not a folder", cur.display())
                }
                Ok(m) if i + 1 == n && !m.is_file() => {
                    anyhow::bail!("{} is not a regular file", cur.display())
                }
                _ => {}
            }
        }
        Ok(())
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
        self.check(path)?;
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
        self.check(path)?;
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

    /// Undo every write and delete, newest first, then remove the folders
    /// this run created under `root` (never `root` itself) once empty.
    /// Returns what could not be undone. A guard dropped without this or
    /// [`Guard::commit`] rolls back and drops the errors.
    pub fn rollback(mut self, root: &Path) -> Vec<String> {
        self.committed = true;
        self.undo_all(Some(root))
    }

    fn undo_all(&mut self, root: Option<&Path>) -> Vec<String> {
        let mut errors = Vec::new();
        // The same form as the recorded paths, so the walk up stops at it.
        let root = root.map(|r| signex_altium_convert::paths::plain(r.to_path_buf()));
        let root = root.as_deref();
        let guard_root = Guard {
            root: self.root.clone(),
            undo: Vec::new(),
            committed: true,
        };
        for (path, old) in self.undo.drain(..).rev() {
            let path = signex_altium_convert::paths::plain(path);
            if let Err(e) = guard_root.check(&path) {
                errors.push(format!("could not restore {}: {e}", path.display()));
                continue;
            }
            let done = match old {
                Some(b) => {
                    signex_types::atomic_io::atomic_write(&path, &b).map_err(|e| e.to_string())
                }
                None => match std::fs::remove_file(&path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                    _ => Ok(()),
                },
            };
            if let Err(e) = done {
                errors.push(format!("could not restore {}: {e}", path.display()));
            }
            let mut dir = path.parent();
            while let Some(d) = dir {
                if root.is_some_and(|r| d == r || !d.starts_with(r)) {
                    break;
                }
                if std::fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
        }
        errors
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if !self.committed {
            self.undo_all(None);
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
        p.canonicalize().unwrap()
    }

    /// A verbatim path to a file inside the folder is inside it.
    #[cfg(windows)]
    #[test]
    fn a_verbatim_path_under_a_plain_root_is_inside() {
        let d = tmp("verbatim");
        let plain_root = signex_altium_convert::paths::plain(d.clone());
        let mut g = Guard::new(&plain_root);
        g.write(&d.join("a.txt"), b"a").unwrap();
        g.commit();
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn uncommitted_guard_restores_the_old_state() {
        let d = tmp("undo");
        std::fs::write(d.join("old.txt"), "old").unwrap();
        std::fs::write(d.join("gone.txt"), "keep me").unwrap();
        {
            let mut g = Guard::new(&d);
            g.write(&d.join("old.txt"), b"new").unwrap();
            g.write(&d.join("sub/new.txt"), b"x").unwrap();
            g.remove(&d.join("gone.txt")).unwrap();
            assert!(g.rollback(&d).is_empty());
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
    fn rollback_stops_at_the_root_and_reports_what_it_cannot_undo() {
        let d = tmp("root");
        let mut g = Guard::new(&d);
        g.write(&d.join("only.txt"), b"x").unwrap();
        assert!(g.rollback(&d).is_empty());
        assert!(d.is_dir(), "the output root stays");

        let mut g = Guard::new(&d);
        std::fs::write(d.join("keep.txt"), "old").unwrap();
        g.write(&d.join("keep.txt"), b"new").unwrap();
        // Make the restore fail: a directory where the file was.
        std::fs::remove_file(d.join("keep.txt")).unwrap();
        std::fs::create_dir(d.join("keep.txt")).unwrap();
        let errors = g.rollback(&d);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("keep.txt"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_guard_refuses_dotdot_outside_paths_and_symlinks() {
        let d = tmp("escape");
        let outside = tmp("escape-outside");
        std::fs::write(outside.join("victim.txt"), "keep").unwrap();
        let mut g = Guard::new(&d);
        assert!(g.write(&d.join("../x.txt"), b"x").is_err());
        assert!(g.write(&outside.join("y.txt"), b"x").is_err());
        assert!(g.remove(&d.join("a/../../victim.txt")).is_err());
        // A symlinked folder and a symlinked file are not followed.
        std::os::unix::fs::symlink(&outside, d.join("link")).unwrap();
        assert!(g.write(&d.join("link/z.txt"), b"x").is_err());
        assert!(g.remove(&d.join("link/victim.txt")).is_err());
        std::os::unix::fs::symlink(outside.join("victim.txt"), d.join("f.txt")).unwrap();
        assert!(g.remove(&d.join("f.txt")).is_err());
        assert!(g.write(&d.join("f.txt"), b"x").is_err());
        assert_eq!(
            std::fs::read_to_string(outside.join("victim.txt")).unwrap(),
            "keep"
        );
        assert!(!outside.join("z.txt").exists());
        std::fs::remove_dir_all(d).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn committed_guard_keeps_everything() {
        let d = tmp("commit");
        let mut g = Guard::new(&d);
        g.write(&d.join("a.txt"), b"a").unwrap();
        g.commit();
        assert!(d.join("a.txt").is_file());
        std::fs::remove_dir_all(d).unwrap();
    }
}
