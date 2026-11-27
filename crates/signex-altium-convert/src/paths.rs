//! Absolute paths as people read them.
//!
//! `std::fs::canonicalize` returns a verbatim path on Windows
//! (`\\?\C:\Users\…`). That form reaches error messages and the import
//! report, where it reads as noise, and a verbatim path takes no `/`, so
//! a join onto it would not open. [`canonical`] gives the ordinary form
//! whenever it means the same file.
//!
//! Library-internal paths (`footprints/<uuid>.snxfpt`, the ledger's file
//! keys) are `/`-separated by contract. [`under`] turns one into a file
//! path below a folder, in the platform's own separators, so a written or
//! shown path never mixes `\` and `/`.

use std::path::{Path, PathBuf};

/// `path` made absolute with every link resolved, as
/// `std::fs::canonicalize`, but without the Windows verbatim prefix when
/// the plain form names the same file.
pub fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    path.canonicalize().map(plain)
}

/// `rel`, a `/`-separated path, below `dir` in the platform's separators.
/// An absolute `rel` (a drive path from a `.DbLib`) replaces `dir`, as
/// [`Path::join`] does.
pub fn under(dir: &Path, rel: &str) -> PathBuf {
    dir.join(with_separator(rel, std::path::MAIN_SEPARATOR))
}

/// `rel` with each `/` turned into `sep`.
fn with_separator(rel: &str, sep: char) -> String {
    rel.replace('/', sep.encode_utf8(&mut [0; 4]))
}

/// `path` without the `\\?\` prefix, when dropping it keeps the meaning:
/// a drive path (not `\\?\UNC\…`), under the classic length limit, with no
/// component Windows would read differently without the prefix. Any other
/// path is returned as it is.
pub fn plain(path: PathBuf) -> PathBuf {
    let Some(rest) = path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) else {
        return path;
    };
    let drive = rest.as_bytes();
    let is_drive_path =
        drive.len() >= 3 && drive[0].is_ascii_alphabetic() && drive[1] == b':' && drive[2] == b'\\';
    if !is_drive_path || rest.len() >= 260 || !rest[3..].split('\\').all(plain_component) {
        return path;
    }
    PathBuf::from(rest)
}

/// A component that means the same with and without the verbatim prefix:
/// not empty, not `.`/`..`, no trailing dot or space, no character Windows
/// reserves, and not a device name such as `CON` or `COM1`.
fn plain_component(part: &str) -> bool {
    if part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' ']) {
        return false;
    }
    if part
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '|' | '?' | '*') || c < ' ')
    {
        return false;
    }
    let stem = part
        .split('.')
        .next()
        .unwrap_or(part)
        .trim_end()
        .to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    !device
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        plain(PathBuf::from(s))
    }

    #[test]
    fn a_drive_path_loses_the_verbatim_prefix() {
        assert_eq!(
            p(r"\\?\C:\Users\me\First Doc.SchDoc"),
            PathBuf::from(r"C:\Users\me\First Doc.SchDoc")
        );
    }

    #[test]
    fn a_path_without_the_prefix_is_unchanged() {
        assert_eq!(p("/home/me/lib"), PathBuf::from("/home/me/lib"));
        assert_eq!(p(r"C:\lib"), PathBuf::from(r"C:\lib"));
    }

    #[test]
    fn paths_that_need_the_prefix_keep_it() {
        for s in [
            r"\\?\UNC\server\share\lib",
            r"\\?\C:\lib\CON",
            r"\\?\C:\lib\com1.txt",
            r"\\?\C:\lib\trailing.",
            r"\\?\C:\lib\a?b",
            r"\\?\Volume{0123}\lib",
        ] {
            assert_eq!(p(s), PathBuf::from(s), "{s}");
        }
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(p(&long), PathBuf::from(&long));
    }

    /// The output guard compares a written file with its plain root: a
    /// verbatim file path under that root must come out under it.
    #[test]
    fn a_verbatim_file_under_a_plain_root_comes_out_under_it() {
        let root = p(r"\\?\C:\Users\me\AppData\Local\Temp\out");
        let file = p(r"\\?\C:\Users\me\AppData\Local\Temp\out\a.txt");
        let (root, file) = (root.to_str().unwrap(), file.to_str().unwrap());
        assert_eq!(root, r"C:\Users\me\AppData\Local\Temp\out");
        assert!(file.starts_with(&format!("{root}\\")), "{file}");
    }

    #[test]
    fn a_library_path_takes_the_platform_separator() {
        assert_eq!(
            with_separator(
                "footprints/0cb7388f-0000-4000-8000-000000000001.snxfpt",
                '\\'
            ),
            r"footprints\0cb7388f-0000-4000-8000-000000000001.snxfpt"
        );
        assert_eq!(with_separator("a.snxlib", '\\'), "a.snxlib");
        assert_eq!(with_separator("symbols/x.snxsym", '/'), "symbols/x.snxsym");
        let dir = Path::new("out");
        let want = dir.join("symbols").join("x.snxsym");
        assert_eq!(under(dir, "symbols/x.snxsym").to_str(), want.to_str());
        if cfg!(windows) {
            let p = under(Path::new(r"C:\out"), "footprints/a.snxfpt");
            assert_eq!(p.to_str().unwrap(), r"C:\out\footprints\a.snxfpt");
        }
    }

    #[test]
    fn com0_and_longer_names_are_ordinary() {
        assert_eq!(
            p(r"\\?\C:\COM0\console.txt"),
            PathBuf::from(r"C:\COM0\console.txt")
        );
    }
}
