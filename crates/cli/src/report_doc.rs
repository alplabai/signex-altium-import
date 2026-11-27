//! The library loss report: `import-report.json` and `import-report.md`
//! (contract section 4).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use signex_altium_convert::Report;
use signex_altium_convert::report::{Item, ItemKind, Severity};

use crate::contract::{CONTRACT, CONVERTER, Counts};

pub const REPORT_JSON: &str = "import-report.json";
pub const REPORT_MD: &str = "import-report.md";
pub const MAPPING: &str = "import-mapping.toml";

/// One source file the run read, with its content hash.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub sha256: String,
}

impl SourceFile {
    pub fn hash(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(SourceFile {
            path: path.to_path_buf(),
            sha256: crate::ledger::sha256_hex(&bytes),
        })
    }
}

/// Everything `import-report.json` holds.
pub struct LibraryReport {
    /// RFC 3339, UTC.
    pub started: String,
    pub sources: Vec<SourceFile>,
    pub library_id: Option<String>,
    pub source: Counts,
    pub written: Counts,
    pub report: Report,
    /// What this run (or, for a run that wrote nothing, the previous one)
    /// wrote: the record the next `--update` deletes by.
    pub ledger: Option<crate::ledger::Ledger>,
    /// The column mapping a database run used (`import-mapping.toml`).
    pub mapping: Option<String>,
}

impl LibraryReport {
    pub fn new(started: String) -> Self {
        LibraryReport {
            started,
            sources: Vec::new(),
            library_id: None,
            source: Counts::default(),
            written: Counts::default(),
            report: Report::default(),
            ledger: None,
            mapping: None,
        }
    }

    fn errors_of(&self, kind: ItemKind) -> usize {
        self.report
            .items()
            .iter()
            .filter(|i| {
                (i.kind == kind || (kind == ItemKind::Part && i.kind == ItemKind::Row))
                    && i.severity == Severity::Error
            })
            .count()
    }

    /// Source items neither written nor reported with severity `error`,
    /// per kind. The completeness rule (contract section 4) wants all zero.
    pub fn unaccounted(&self) -> Counts {
        let gap = |src: usize, out: usize, kind| src.saturating_sub(out + self.errors_of(kind));
        Counts {
            symbols: gap(self.source.symbols, self.written.symbols, ItemKind::Symbol),
            footprints: gap(
                self.source.footprints,
                self.written.footprints,
                ItemKind::Footprint,
            ),
            parts: gap(self.source.parts, self.written.parts, ItemKind::Part),
            models: gap(self.source.models, self.written.models, ItemKind::Model),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.unaccounted() == Counts::default()
    }

    /// Items with severity `error`: source items that were not written.
    pub fn skipped(&self) -> usize {
        self.report
            .items()
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count()
    }

    pub fn to_json(&self) -> Value {
        let items: Vec<Value> = self.report.items().iter().map(Item::to_json).collect();
        let sources: Vec<Value> = self
            .sources
            .iter()
            .map(|s| json!({ "path": s.path, "sha256": s.sha256 }))
            .collect();
        let mut v = json!({
            "contract": CONTRACT,
            "converter": CONVERTER,
            "converter_version": env!("CARGO_PKG_VERSION"),
            "started": self.started,
            "sources": sources,
            "library_id": self.library_id,
            "counts": { "source": self.source.to_json(), "written": self.written.to_json() },
            "items": items,
        });
        if let Some(l) = &self.ledger {
            v["ledger"] = l.to_json();
        }
        v
    }

    pub fn to_markdown(&self) -> String {
        let mut s = format!(
            "# Import report\n\n{CONVERTER} {} (contract {CONTRACT}), started {}\n\n",
            env!("CARGO_PKG_VERSION"),
            self.started
        );
        s.push_str("## Sources\n\n");
        for f in &self.sources {
            s.push_str(&format!(
                "- `{}` (sha256 `{}`)\n",
                f.path.display(),
                f.sha256
            ));
        }
        s.push_str("\n## Counts\n\n| | source | written |\n|---|---|---|\n");
        for (name, a, b) in [
            ("symbols", self.source.symbols, self.written.symbols),
            (
                "footprints",
                self.source.footprints,
                self.written.footprints,
            ),
            ("parts", self.source.parts, self.written.parts),
            ("models", self.source.models, self.written.models),
        ] {
            s.push_str(&format!("| {name} | {a} | {b} |\n"));
        }
        s.push_str("\n## Items\n\n");
        if self.report.items().is_empty() {
            s.push_str("Nothing was approximated, dropped or skipped.\n");
        } else {
            s.push_str("| severity | code | kind | source | message |\n|---|---|---|---|---|\n");
            for i in self.report.items() {
                s.push_str(&format!(
                    "| {} | `{}` | {} | {} | {} |\n",
                    i.severity.as_str(),
                    i.code,
                    i.kind.as_str(),
                    cell(&i.source),
                    cell(&i.message),
                ));
            }
        }
        s
    }

    /// Write both report files into `dir`; returns the two paths.
    pub fn write(&self, dir: &Path) -> Result<(PathBuf, PathBuf)> {
        let json_path = dir.join(REPORT_JSON);
        let md_path = dir.join(REPORT_MD);
        for p in [&json_path, &md_path] {
            if std::fs::symlink_metadata(p).is_ok_and(|m| !m.is_file()) {
                anyhow::bail!(
                    "{} is not a regular file; refusing to write it",
                    p.display()
                );
            }
        }
        let mut text = serde_json::to_string_pretty(&self.to_json())?;
        text.push('\n');
        signex_types::atomic_io::atomic_write(&json_path, text.as_bytes())
            .with_context(|| format!("writing {}", json_path.display()))?;
        signex_types::atomic_io::atomic_write(&md_path, self.to_markdown().as_bytes())
            .with_context(|| format!("writing {}", md_path.display()))?;
        if let Some(m) = &self.mapping {
            let p = dir.join(MAPPING);
            signex_types::atomic_io::atomic_write(&p, m.as_bytes())
                .with_context(|| format!("writing {}", p.display()))?;
        }
        Ok((json_path, md_path))
    }
}

/// Keep a table cell on one line.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: ItemKind, sev: Severity) -> Item {
        Item {
            kind,
            source: "L:x".into(),
            id: None,
            code: "library.input_unreadable",
            severity: sev,
            message: "m".into(),
            detail: Value::Null,
        }
    }

    #[test]
    fn completeness_counts_errors_as_accounted() {
        let mut r = LibraryReport::new("2026-01-01T00:00:00Z".into());
        r.source.symbols = 3;
        r.written.symbols = 2;
        assert!(!r.is_complete());
        assert_eq!(r.unaccounted().symbols, 1);
        r.report.push(item(ItemKind::Symbol, Severity::Error));
        assert!(r.is_complete());
        assert_eq!(r.skipped(), 1);
    }

    #[test]
    fn markdown_lists_items() {
        let mut r = LibraryReport::new("t".into());
        r.report.push(item(ItemKind::Symbol, Severity::Dropped));
        let md = r.to_markdown();
        assert!(md.contains("| dropped | `library.input_unreadable` | symbol | L:x | m |"));
    }
}
