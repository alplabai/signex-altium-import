//! The record of what this converter wrote, kept in `import-report.json`
//! (contract section 5). An update deletes only what the record names, and
//! the record is untrusted input: any entry of the wrong shape, a foreign
//! converter or another library's id means no record, and nothing is deleted.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Value, json};
use uuid::Uuid;

use crate::contract::CONVERTER;
use crate::report_doc::REPORT_JSON;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    pub library_id: Option<Uuid>,
    pub rows: BTreeSet<Uuid>,
    pub symbols: BTreeSet<Uuid>,
    pub footprints: BTreeSet<Uuid>,
    /// Paths relative to the library root, in the three expected forms, with
    /// the SHA-256 (lower-case hex) of the bytes written. A file is deleted
    /// only while its bytes still match.
    pub files: BTreeMap<String, String>,
}

/// More entries than any real library; a bigger record is not ours.
const MAX_ENTRIES: usize = 2_000_000;

fn uuids(v: &Value) -> Option<BTreeSet<Uuid>> {
    let a = v.as_array().filter(|a| a.len() <= MAX_ENTRIES)?;
    a.iter()
        .map(|x| {
            let s = x.as_str()?;
            let u = Uuid::parse_str(s).ok()?;
            (u.to_string() == s).then_some(u)
        })
        .collect()
}

fn is_sha256(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A plain relative file name of one of the three forms this converter writes.
pub fn expected_path(p: &str) -> bool {
    if p.is_empty() || p.len() > 255 || p.contains(['\\', '\0']) || p.starts_with('/') {
        return false;
    }
    let parts: Vec<&str> = p.split('/').collect();
    if parts
        .iter()
        .any(|c| c.is_empty() || *c == "." || *c == "..")
    {
        return false;
    }
    match parts.as_slice() {
        [name] => name.ends_with(".snxlib") && name.len() > ".snxlib".len(),
        ["symbols", name] => name.ends_with(".snxsym") && name.len() > ".snxsym".len(),
        ["footprints", name] => name
            .strip_suffix(".snxfpt")
            .is_some_and(|stem| Uuid::parse_str(stem).is_ok_and(|u| u.to_string() == stem)),
        _ => false,
    }
}

impl Ledger {
    pub fn to_json(&self) -> Value {
        let ids = |s: &BTreeSet<Uuid>| s.iter().map(Uuid::to_string).collect::<Vec<_>>();
        json!({
            "converter": CONVERTER,
            "rows": ids(&self.rows),
            "symbols": ids(&self.symbols),
            "footprints": ids(&self.footprints),
            "files": self.files.iter().map(|(p, h)| json!({ "path": p, "sha256": h })).collect::<Vec<_>>(),
        })
    }

    /// The record in a parsed report, or `None` when it is absent or not
    /// entirely well formed.
    pub fn from_report(report: &Value) -> Option<Ledger> {
        let l = report.get("ledger")?.as_object()?;
        if l.get("converter")?.as_str()? != CONVERTER {
            return None;
        }
        let files = l
            .get("files")?
            .as_array()
            .filter(|a| a.len() <= MAX_ENTRIES)?;
        let files: BTreeMap<String, String> = files
            .iter()
            .map(|f| {
                let path = f.get("path")?.as_str().filter(|p| expected_path(p))?;
                let hash = f.get("sha256")?.as_str().filter(|h| is_sha256(h))?;
                Some((path.to_string(), hash.to_string()))
            })
            .collect::<Option<_>>()?;
        let library_id = match report.get("library_id")? {
            Value::String(s) => Some(Uuid::parse_str(s).ok()?),
            Value::Null => None,
            _ => return None,
        };
        Some(Ledger {
            library_id,
            rows: uuids(l.get("rows")?)?,
            symbols: uuids(l.get("symbols")?)?,
            footprints: uuids(l.get("footprints")?)?,
            files,
        })
    }

    /// The record `dir` holds: its report, if a regular file of sane size.
    pub fn read(dir: &Path) -> Option<Ledger> {
        let p = dir.join(REPORT_JSON);
        let m = std::fs::symlink_metadata(&p).ok()?;
        if !m.is_file() || m.len() > 256 * 1024 * 1024 {
            return None;
        }
        let v: Value = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
        Self::from_report(&v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(p: &str) -> Value {
        json!({ "path": p, "sha256": "0".repeat(64) })
    }

    fn good() -> Value {
        json!({
            "library_id": "5b0f3b0e-0000-4000-8000-000000000001",
            "ledger": {
                "converter": CONVERTER,
                "rows": ["5b0f3b0e-0000-4000-8000-000000000002"],
                "symbols": [],
                "footprints": ["5b0f3b0e-0000-4000-8000-000000000003"],
                "files": [f("a.snxlib"), f("symbols/x.snxsym"),
                          f("footprints/5b0f3b0e-0000-4000-8000-000000000003.snxfpt")],
            }
        })
    }

    #[test]
    fn a_well_formed_record_reads_and_round_trips() {
        let l = Ledger::from_report(&good()).unwrap();
        assert_eq!(l.files.len(), 3);
        let mut again = good();
        again["ledger"] = l.to_json();
        assert_eq!(Ledger::from_report(&again), Some(l));
    }

    #[test]
    fn any_bad_entry_means_no_record() {
        for bad in [
            "../evil.snxlib",
            "/etc/passwd",
            "symbols/../../x.snxsym",
            "symbols/sub/x.snxsym",
            "footprints/not-a-uuid.snxfpt",
            "a.txt",
            "symbols\\x.snxsym",
            ".snxlib",
            "",
        ] {
            let mut v = good();
            v["ledger"]["files"]
                .as_array_mut()
                .unwrap()
                .push(json!(bad));
            assert!(Ledger::from_report(&v).is_none(), "{bad:?}");
        }
        let mut v = good();
        v["ledger"]["rows"] = json!(["not a uuid"]);
        assert!(Ledger::from_report(&v).is_none());
        let mut v = good();
        v["ledger"]["converter"] = json!("someone-else");
        assert!(Ledger::from_report(&v).is_none());
        let mut v = good();
        v["ledger"]["rows"] = json!(["5B0F3B0E-0000-4000-8000-000000000002"]);
        assert!(
            Ledger::from_report(&v).is_none(),
            "ids are canonical lower case"
        );
        let mut v = good();
        v["ledger"]["files"]
            .as_array_mut()
            .unwrap()
            .push(json!({"path": "b.snxlib", "sha256": "xyz"}));
        assert!(
            Ledger::from_report(&v).is_none(),
            "a malformed hash is not a record"
        );
        assert!(Ledger::from_report(&json!({})).is_none());
    }
}
