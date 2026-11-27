#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn repo() -> PathBuf {
    plain(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap(),
    )
}

pub fn sample(name: &str) -> PathBuf {
    repo().join("tests").join("samples").join(name)
}

/// `p` without the `\\?\` prefix `canonicalize` adds on Windows (a
/// verbatim path takes no `/`, so joins onto it would not open).
pub fn plain(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with(r"UNC\") => PathBuf::from(rest),
        _ => p,
    }
}

/// `text` (JSON output) with each path in `subs` replaced by its
/// placeholder, in its JSON-escaped form, with and without the verbatim
/// prefix. On Windows the remaining escaped separators become `/` and the
/// OS text for a missing file becomes the Unix one, so one golden serves
/// every platform; on Unix the text is otherwise untouched.
pub fn placeholders(text: &str, subs: &[(&Path, &str)]) -> String {
    let escaped = |s: &str| {
        let q = serde_json::to_string(s).unwrap();
        q[1..q.len() - 1].to_string()
    };
    let mut t = text.to_string();
    for (p, name) in subs {
        let full = p.display().to_string();
        t = t.replace(&escaped(&full), name);
        if let Some(rest) = full.strip_prefix(r"\\?\") {
            t = t.replace(&escaped(rest), name);
        }
    }
    if cfg!(windows) {
        t = t.replace(r"\\", "/").replace(
            "The system cannot find the file specified.",
            "No such file or directory",
        );
    }
    t
}

pub fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_signex-altium-import"))
        .args(args)
        .output()
        .expect("spawn")
}

/// A fresh directory under the system temp dir, removed on drop.
pub struct Tmp(pub PathBuf);

impl Tmp {
    pub fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "snx-import-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn schema() -> jsonschema::Validator {
    let text = std::fs::read_to_string(repo().join("tests/fixtures/library-converter-v1.json"))
        .expect("schema fixture");
    jsonschema::validator_for(&serde_json::from_str(&text).unwrap()).expect("schema compiles")
}
