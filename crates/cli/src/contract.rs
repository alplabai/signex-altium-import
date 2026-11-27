//! The library converter contract (v1): `describe`, exit codes, counts and
//! the summary event. The contract is `docs/LIBRARY_CONVERTER_CONTRACT.md`
//! in the Signex repository.

use std::path::PathBuf;

use clap::Parser;
use serde_json::{Value, json};
use signex_altium_convert::Report;
use signex_altium_convert::report::Severity;

/// The contract version this converter implements.
pub const CONTRACT: &str = "1.0";
/// The name `describe` and the report give for this converter.
pub const CONVERTER: &str = "signex-altium-import";

/// How a run ended; mirrors the process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Ok,
    Partial,
    Failed,
}

impl Exit {
    pub fn code(self) -> i32 {
        match self {
            Exit::Ok => 0,
            Exit::Partial => 2,
            Exit::Failed => 1,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Exit::Ok => "ok",
            Exit::Partial => "partial",
            Exit::Failed => "failed",
        }
    }

    /// `partial` when anything was dropped or not written, else `ok`.
    pub fn of_report(report: &Report) -> Self {
        match report.worst() {
            Some(s) if s >= Severity::Dropped => Exit::Partial,
            _ => Exit::Ok,
        }
    }
}

/// Item counts of a library, source side or written side.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub symbols: usize,
    pub footprints: usize,
    pub parts: usize,
    pub models: usize,
}

impl Counts {
    pub fn to_json(self) -> Value {
        json!({
            "symbols": self.symbols,
            "footprints": self.footprints,
            "parts": self.parts,
            "models": self.models,
        })
    }
}

/// The last stdout line of a `library` run.
#[derive(Debug, Clone, Default)]
pub struct LibrarySummary {
    /// `None` on success.
    pub error: Option<String>,
    pub exit: Option<Exit>,
    pub library: Option<PathBuf>,
    pub report: Option<PathBuf>,
    pub files: Vec<PathBuf>,
    pub counts: Option<Counts>,
    pub skipped: usize,
    /// Non-`info` report items aggregated by message.
    pub warnings: Vec<(String, usize)>,
}

impl LibrarySummary {
    pub fn failed(error: impl Into<String>) -> Self {
        LibrarySummary {
            error: Some(error.into()),
            exit: Some(Exit::Failed),
            ..Default::default()
        }
    }

    pub fn exit(&self) -> Exit {
        self.exit.unwrap_or(Exit::Failed)
    }

    pub fn to_json(&self) -> Value {
        let counts = self.counts.map(|c| {
            let mut v = c.to_json();
            v["skipped"] = json!(self.skipped);
            v
        });
        let warnings: Vec<Value> = self
            .warnings
            .iter()
            .map(|(m, n)| json!({ "message": m, "count": n }))
            .collect();
        let mut v = json!({
            "event": "summary",
            "ok": self.error.is_none(),
            "error": self.error,
            "exit": self.exit().as_str(),
            "library": self.library,
            "report": self.report,
            "files": self.files,
            "warnings": warnings,
        });
        // `counts` is absent, not null, when the run never got that far.
        if let Some(c) = counts {
            v["counts"] = c;
        }
        v
    }
}

/// `describe --json`.
pub fn describe_json() -> Value {
    let input = |label: &str, ext: &str, kind: &str| json!({ "label": label, "extensions": [ext], "kind": kind });
    json!({
        "contract": CONTRACT,
        "converter": CONVERTER,
        "version": env!("CARGO_PKG_VERSION"),
        "inputs": [
            input("Altium schematic library", "schlib", "library"),
            input("Altium PCB library", "pcblib", "library"),
            input("Altium integrated library", "intlib", "library"),
            input("Altium database library", "dblib", "database"),
        ],
    })
}

#[derive(Parser, Debug)]
#[command(name = "describe")]
struct DescribeArgs {
    /// Print the description as JSON (the only format).
    #[arg(long)]
    json: bool,
}

/// Entry point of `signex-altium-import describe [--json]`.
pub fn describe_main(args: &[std::ffi::OsString]) -> i32 {
    let argv = std::iter::once("describe".into()).chain(args.iter().cloned());
    match DescribeArgs::try_parse_from(argv) {
        Ok(_) => {
            println!("{}", describe_json());
            0
        }
        Err(e) => {
            let is_err = e.use_stderr();
            let _ = e.print();
            i32::from(is_err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_follows_worst_severity() {
        let mut r = Report::default();
        assert_eq!(Exit::of_report(&r), Exit::Ok);
        r.warn("pcb.copper_arc_flattened", "a");
        assert_eq!(Exit::of_report(&r), Exit::Ok);
        r.warn("pcb.region_too_few_points", "b");
        assert_eq!(Exit::of_report(&r), Exit::Partial);
        assert_eq!(
            (Exit::Ok.code(), Exit::Partial.code(), Exit::Failed.code()),
            (0, 2, 1)
        );
    }

    #[test]
    fn failed_summary_shape() {
        let v = LibrarySummary::failed("boom").to_json();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"], "boom");
        assert_eq!(v["exit"], "failed");
        assert_eq!(v["files"], json!([]));
        assert_eq!(v["warnings"], json!([]));
    }
    fn validator() -> jsonschema::Validator {
        let schema: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/library-converter-v1.json"
        ))
        .unwrap();
        jsonschema::validator_for(&schema).unwrap()
    }

    fn written(exit: Exit, warnings: Vec<(String, usize)>) -> LibrarySummary {
        LibrarySummary {
            error: None,
            exit: Some(exit),
            library: Some("/abs/out/L".into()),
            report: Some("/abs/out/L/import-report.json".into()),
            files: vec!["/abs/out/L/L.snxlib".into()],
            counts: Some(Counts {
                symbols: 2,
                footprints: 1,
                parts: 2,
                models: 0,
            }),
            skipped: 1,
            warnings,
        }
    }

    #[test]
    fn ok_summary_matches_the_schema() {
        let v = written(Exit::Ok, vec![]).to_json();
        validator().validate(&v).unwrap();
        assert_eq!(
            (v["ok"].clone(), v["exit"].clone()),
            (json!(true), json!("ok"))
        );
        assert_eq!(v["counts"]["skipped"], 1);
    }

    #[test]
    fn partial_summary_matches_the_schema() {
        let v = written(Exit::Partial, vec![("octagonal pad".into(), 37)]).to_json();
        validator().validate(&v).unwrap();
        assert_eq!(v["exit"], "partial");
        assert_eq!(v["warnings"][0]["count"], 37);
    }

    #[test]
    fn failed_summary_matches_the_schema() {
        let v = LibrarySummary::failed("boom").to_json();
        validator().validate(&v).unwrap();
    }
}
