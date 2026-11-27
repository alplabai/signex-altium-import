//! Absolute paths as people read them.
//!
//! `std::fs::canonicalize` returns a verbatim path on Windows
//! (`\\?\C:\Users\…`). That form reaches error messages and the import
//! report, where it reads as noise, and a verbatim path takes no `/`, so
//! a join onto it would not open. [`canonical`] gives the ordinary form
//! whenever it means the same file.

use std::path::{Path, PathBuf};

/// `path` made absolute with every link resolved, as
/// `std::fs::canonicalize`, but without the Windows verbatim prefix when
/// the plain form names the same file.
pub fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    path.canonicalize().map(plain)
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

    #[test]
    fn com0_and_longer_names_are_ordinary() {
        assert_eq!(
            p(r"\\?\C:\COM0\console.txt"),
            PathBuf::from(r"C:\COM0\console.txt")
        );
    }
}
