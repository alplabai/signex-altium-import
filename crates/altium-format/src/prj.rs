//! `.PrjPcb` project files: INI text listing the project's documents.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Project {
    /// Document paths as written, relative to the project file.
    pub documents: Vec<String>,
    /// `[Parameter*]` sections: project-level parameters.
    pub parameters: Vec<(String, String)>,
    /// `[ProjectVariant*]` descriptions.
    pub variants: Vec<String>,
}

impl Project {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self::parse(&decode(&bytes)))
    }

    pub fn parse(text: &str) -> Self {
        let mut prj = Project::default();
        let mut section = String::new();
        let mut pending: Option<(String, String)> = None;
        for raw in text.lines() {
            let line = raw.trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                flush(&mut prj, &mut pending);
                section = name.to_string();
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim().to_string());
            let lower = section.to_ascii_lowercase();
            if lower.starts_with("document") && !lower.starts_with("documentgroup") {
                if k.eq_ignore_ascii_case("DocumentPath") && !v.is_empty() {
                    prj.documents.push(v.replace('\\', "/"));
                }
            } else if lower.starts_with("parameter") {
                let p = pending.get_or_insert_with(Default::default);
                if k.eq_ignore_ascii_case("Name") {
                    p.0 = v;
                } else if k.eq_ignore_ascii_case("Value") {
                    p.1 = v;
                }
            } else if lower.starts_with("projectvariant") && k.eq_ignore_ascii_case("Description") {
                prj.variants.push(v);
            }
        }
        flush(&mut prj, &mut pending);
        prj
    }

    /// Absolute document paths with the given extension (case-insensitive).
    pub fn documents_with_ext(&self, project_dir: &Path, ext: &str) -> Vec<PathBuf> {
        self.documents
            .iter()
            .filter(|d| {
                Path::new(d)
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case(ext))
            })
            .map(|d| project_dir.join(d))
            .collect()
    }
}

fn flush(prj: &mut Project, pending: &mut Option<(String, String)>) {
    if let Some((k, v)) = pending.take()
        && !k.is_empty()
    {
        prj.parameters.push((k, v));
    }
}

/// UTF-8 with or without BOM; anything else is read as Windows-1252.
fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => crate::params::decode_ansi(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_and_parameters() {
        let p = Project::parse(
            "[Design]\nVersion=1.0\n[Document1]\nDocumentPath=Sheets\\Main.SchDoc\n\
             [Document2]\nDocumentPath=Board.PcbDoc\n[Parameter1]\nName=Rev\nValue=B\n",
        );
        assert_eq!(p.documents, ["Sheets/Main.SchDoc", "Board.PcbDoc"]);
        assert_eq!(p.parameters, [("Rev".to_string(), "B".to_string())]);
        let sch = p.documents_with_ext(Path::new("/p"), "schdoc");
        assert_eq!(sch, [PathBuf::from("/p/Sheets/Main.SchDoc")]);
    }
}
