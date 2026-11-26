#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub fn sample(name: &str) -> PathBuf {
    repo().join("tests/samples").join(name)
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
