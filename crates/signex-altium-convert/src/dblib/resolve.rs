//! Finding the library files a database row names. Order (Altium's own):
//! the path as given, then relative to the `.DbLib`, then by file name across
//! the extra search folders (`--search-path`) and the `.DbLib`'s own
//! `LibrarySearchPath` (relative to it), one level or with sub-folders.
//! Paths from another machine (`C:\Users\...\Symbols\A.SchLib`) therefore
//! resolve by their file name.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::paths::under;

pub struct Resolver {
    dblib_dir: PathBuf,
    folders: Vec<PathBuf>,
    subdirs: bool,
    listings: RefCell<HashMap<PathBuf, Vec<(String, PathBuf)>>>,
    truncated: std::cell::Cell<bool>,
}

fn plain_file(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file())
}

impl Resolver {
    /// `extra` are the `--search-path` folders; `own` the `.DbLib`'s search
    /// path entries, relative to `dblib_dir` (or absolute).
    pub fn new(dblib_dir: &Path, extra: &[PathBuf], own: &[String], subdirs: bool) -> Self {
        let mut folders: Vec<PathBuf> = extra.to_vec();
        for o in own {
            folders.push(under(dblib_dir, &o.replace('\\', "/")));
        }
        folders.push(dblib_dir.to_path_buf());
        Resolver {
            dblib_dir: dblib_dir.to_path_buf(),
            folders,
            subdirs,
            listings: RefCell::new(HashMap::new()),
            truncated: std::cell::Cell::new(false),
        }
    }

    fn listing(&self, dir: &Path) -> Vec<(String, PathBuf)> {
        if let Some(l) = self.listings.borrow().get(dir) {
            return l.clone();
        }
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        let mut seen = 0usize;
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            let mut entries: Vec<_> = rd.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                seen += 1;
                if seen > 1_000_000 {
                    self.truncated.set(true);
                    break;
                }
                let Ok(t) = e.file_type() else { continue };
                if t.is_file() {
                    out.push((e.file_name().to_string_lossy().to_lowercase(), e.path()));
                } else if t.is_dir() && !t.is_symlink() && self.subdirs {
                    stack.push(e.path());
                }
            }
        }
        self.listings
            .borrow_mut()
            .insert(dir.to_path_buf(), out.clone());
        out
    }

    /// A listing stopped at its size limit.
    pub fn truncated(&self) -> bool {
        self.truncated.get()
    }

    /// The file `given` names, if any.
    pub fn resolve(&self, given: &str) -> Option<PathBuf> {
        let norm = given.trim().replace('\\', "/");
        if norm.is_empty() {
            return None;
        }
        let as_given = under(Path::new(""), &norm);
        if as_given.is_absolute() && plain_file(&as_given) {
            return Some(as_given);
        }
        let beside = under(&self.dblib_dir, &norm);
        if as_given.is_relative() && plain_file(&beside) {
            return Some(beside);
        }
        let name = norm.rsplit('/').next()?.to_lowercase();
        for f in &self.folders {
            if let Some((_, p)) = self.listing(f).iter().find(|(n, _)| *n == name) {
                return Some(p.clone());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(n: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("snx-res-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn the_three_steps_in_order() {
        let root = tmp("order");
        let dbdir = root.join("db");
        std::fs::create_dir_all(dbdir.join("Symbols/sub")).unwrap();
        std::fs::create_dir_all(root.join("extra")).unwrap();
        std::fs::write(dbdir.join("beside.SchLib"), "x").unwrap();
        std::fs::write(dbdir.join("Symbols/A.SchLib"), "own").unwrap();
        std::fs::write(dbdir.join("Symbols/sub/Deep.SchLib"), "deep").unwrap();
        std::fs::write(root.join("extra/A.SchLib"), "extra").unwrap();
        let r = Resolver::new(&dbdir, &[root.join("extra")], &["Symbols".into()], false);
        // As given, absolute.
        let abs = root.join("extra/A.SchLib");
        assert_eq!(r.resolve(abs.to_str().unwrap()), Some(abs.clone()));
        // Relative to the .DbLib.
        assert_eq!(
            r.resolve("beside.SchLib"),
            Some(dbdir.join("beside.SchLib"))
        );
        // Another machine's path: by file name, --search-path first.
        assert_eq!(r.resolve("C:\\Users\\x\\Lib\\a.schlib"), Some(abs));
        // Sub-folders only with the flag.
        assert_eq!(r.resolve("Deep.SchLib"), None);
        let r2 = Resolver::new(&dbdir, &[], &["Symbols".into()], true);
        assert_eq!(
            r2.resolve("D:\\x\\deep.schlib"),
            Some(dbdir.join("Symbols/sub/Deep.SchLib"))
        );
        assert_eq!(r2.resolve("missing.SchLib"), None);
        assert_eq!(r2.resolve(""), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
