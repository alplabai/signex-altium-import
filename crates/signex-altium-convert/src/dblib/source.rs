//! Reading a database's tables as text rows: from a folder of per-table CSV
//! files, or from an Access file through mdbtools (`mdb-tables` and
//! `mdb-export`, run as separate processes, never linked: mdbtools is GPL).

use std::path::{Path, PathBuf};
use std::process::Command;

/// One table, every cell as text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableData {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Columns whose text held bytes that are not UTF-8 (replaced).
    pub lossy_columns: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error(
        "reading Access files needs mdbtools: install the `mdbtools` package (it provides mdb-tables and mdb-export), or pass --db <folder of per-table .csv files> (the only way on Windows until direct connections land)"
    )]
    NoMdbTools,
    #[error("{0}")]
    Other(String),
}

/// Where the rows come from.
#[derive(Debug, Clone)]
pub enum DbSource {
    /// A folder of `<table>.csv` files.
    Csv(PathBuf),
    /// An Access file (`.mdb`, `.accdb`); `tools` is the folder holding the
    /// mdbtools programs, `None` for the `PATH`.
    Access {
        file: PathBuf,
        tools: Option<PathBuf>,
    },
}

/// Largest export accepted from one table.
const MAX_EXPORT: usize = 1 << 30;
/// How long one mdbtools run may take.
const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

impl DbSource {
    pub fn access(file: impl Into<PathBuf>) -> Self {
        DbSource::Access {
            file: file.into(),
            tools: None,
        }
    }

    /// The source kind a `--db` value (or a stored path) names: a folder is
    /// CSV, an Access file is read through mdbtools.
    pub fn from_path(p: &Path) -> Option<Self> {
        if p.is_dir() {
            return Some(DbSource::Csv(p.to_path_buf()));
        }
        let ext = p.extension()?.to_str()?.to_ascii_lowercase();
        (p.is_file() && matches!(ext.as_str(), "mdb" | "accdb")).then(|| DbSource::access(p))
    }

    /// The path that identifies the source (for the report's sources).
    pub fn path(&self) -> &Path {
        match self {
            DbSource::Csv(d) => d,
            DbSource::Access { file, .. } => file,
        }
    }

    fn tool(tools: &Option<PathBuf>, name: &str) -> Command {
        match tools {
            Some(d) => Command::new(d.join(name)),
            None => Command::new(name),
        }
    }

    /// Run a tool with a deadline, a cap on its output and a tail of its
    /// error text. The process is killed when the deadline passes.
    fn run(mut cmd: Command) -> Result<Vec<u8>, SourceError> {
        use std::io::Read;
        use std::process::Stdio;
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.env("LC_ALL", "C");
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SourceError::NoMdbTools
            } else {
                SourceError::Other(format!("running mdbtools: {e}"))
            }
        })?;
        let mut out = child.stdout.take().expect("piped");
        let mut err = child.stderr.take().expect("piped");
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 64 * 1024];
            loop {
                match out.read(&mut chunk) {
                    Ok(0) | Err(_) => return Ok(buf),
                    Ok(n) => {
                        if buf.len() + n > MAX_EXPORT {
                            return Err(());
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                }
            }
        });
        let tail = std::thread::spawn(move || {
            let mut t: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = err.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                t.extend_from_slice(&chunk[..n]);
                if t.len() > 8192 {
                    t.drain(..t.len() - 4096);
                }
            }
            String::from_utf8_lossy(&t).into_owned()
        });
        let start = std::time::Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if start.elapsed() > TOOL_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                Err(e) => return Err(SourceError::Other(format!("waiting for mdbtools: {e}"))),
            }
        };
        let stdout = reader.join().unwrap_or(Err(()));
        let stderr = tail.join().unwrap_or_default();
        let Some(status) = status else {
            return Err(SourceError::Other(
                "mdbtools did not finish in time; stopped".into(),
            ));
        };
        if !status.success() {
            let last: Vec<&str> = stderr.lines().rev().take(3).collect();
            return Err(SourceError::Other(format!(
                "mdbtools failed: {}",
                last.into_iter().rev().collect::<Vec<_>>().join(" / ")
            )));
        }
        stdout.map_err(|()| SourceError::Other("a table export is too large".into()))
    }

    /// The table names the source holds.
    pub fn tables(&self) -> Result<Vec<String>, SourceError> {
        match self {
            DbSource::Csv(dir) => {
                let mut v: Vec<String> = std::fs::read_dir(dir)
                    .map_err(|e| SourceError::Other(format!("{}: {e}", dir.display())))?
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .filter_map(|e| {
                        let p = e.path();
                        p.extension()?
                            .eq_ignore_ascii_case("csv")
                            .then(|| p.file_stem().map(|s| s.to_string_lossy().into_owned()))?
                    })
                    .collect();
                v.sort();
                Ok(v)
            }
            DbSource::Access { file, tools } => {
                let mut c = Self::tool(tools, "mdb-tables");
                c.arg("-1").arg("--").arg(file);
                let out = Self::run(c)?;
                Ok(String::from_utf8_lossy(&out)
                    .lines()
                    .map(|l| l.trim_end_matches('\r').to_string())
                    .filter(|l| !l.is_empty())
                    .collect())
            }
        }
    }

    /// One table's columns and rows; `None` when the source has no such table
    /// (table names match without case).
    pub fn read(&self, table: &str) -> Result<Option<TableData>, SourceError> {
        let Some(name) = self
            .tables()?
            .into_iter()
            .find(|t| t.eq_ignore_ascii_case(table))
        else {
            return Ok(None);
        };
        let bytes = match self {
            DbSource::Csv(dir) => {
                let p = dir.join(format!("{name}.csv"));
                std::fs::read(&p)
                    .map_err(|e| SourceError::Other(format!("{}: {e}", p.display())))?
            }
            DbSource::Access { file, tools } => {
                let mut c = Self::tool(tools, "mdb-export");
                // ISO dates, one header row, quoted text with embedded newlines.
                c.args([
                    "-D",
                    "%Y-%m-%d",
                    "-T",
                    "%Y-%m-%dT%H:%M:%S",
                    "-b",
                    "strip",
                    "--",
                ])
                .arg(file)
                .arg(&name);
                Self::run(c)?
            }
        };
        parse_csv(&bytes).map(Some)
    }
}

/// CSV text (UTF-8, a byte-order mark tolerated; quotes, doubled quotes and
/// line breaks inside quotes) as a header and rows.
pub fn parse_csv(bytes: &[u8]) -> Result<TableData, SourceError> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let text = String::from_utf8_lossy(bytes);
    let lossy = text.contains('\u{fffd}')
        && !String::from_utf8_lossy(bytes).is_empty()
        && std::str::from_utf8(bytes).is_err();
    let mut r = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut it = r.records();
    let Some(head) = it.next() else {
        return Ok(TableData::default());
    };
    let columns: Vec<String> = head
        .map_err(|e| SourceError::Other(format!("csv header: {e}")))?
        .iter()
        .map(str::to_string)
        .collect();
    let mut rows = Vec::new();
    for rec in it {
        let rec = rec.map_err(|e| SourceError::Other(format!("csv row: {e}")))?;
        let mut row: Vec<String> = rec.iter().map(str::to_string).collect();
        row.resize(columns.len(), String::new());
        rows.push(row);
    }
    let lossy_columns = if lossy {
        columns
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                rows.iter()
                    .any(|r: &Vec<String>| r.get(*i).is_some_and(|c| c.contains('\u{fffd}')))
            })
            .map(|(_, c)| c.clone())
            .collect()
    } else {
        Vec::new()
    };
    Ok(TableData {
        columns,
        rows,
        lossy_columns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_newlines_and_a_byte_order_mark() {
        let t = parse_csv("\u{feff}A,B\n1,\"x, \"\"y\"\"\nz\"\n2\n".as_bytes()).unwrap();
        assert_eq!(t.columns, ["A", "B"]);
        assert_eq!(t.rows[0], ["1", "x, \"y\"\nz"]);
        assert_eq!(t.rows[1], ["2", ""], "a short row is padded");
    }

    #[test]
    fn missing_mdbtools_is_a_clear_error() {
        let s = DbSource::Access {
            file: "x.mdb".into(),
            tools: Some("/nonexistent/tools".into()),
        };
        let e = s.tables().unwrap_err();
        assert!(matches!(e, SourceError::NoMdbTools), "{e}");
        assert!(e.to_string().contains("install"));
    }

    #[test]
    fn a_csv_folder_lists_and_reads_tables_without_case() {
        let d = std::env::temp_dir().join(format!("snx-csv-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Resistors.csv"), "Part Number,Value\nR1,10k\n").unwrap();
        let s = DbSource::Csv(d.clone());
        assert_eq!(s.tables().unwrap(), ["Resistors"]);
        let t = s.read("resistors").unwrap().unwrap();
        assert_eq!(t.rows, vec![vec!["R1".to_string(), "10k".to_string()]]);
        assert!(s.read("nope").unwrap().is_none());
        std::fs::remove_dir_all(d).unwrap();
    }
}
