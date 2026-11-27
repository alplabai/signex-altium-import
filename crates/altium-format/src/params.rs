//! `|KEY=VALUE|KEY=VALUE` property lists — the text payload of most records.

/// One record's properties, in file order.
///
/// Keys compare case-insensitively: the same property shows up as
/// `LOCATION.X` in one file version and `Location.X` in another.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Params {
    entries: Vec<(String, String)>,
    ansi_pipe_assumed: bool,
}

/// Decode legacy ANSI text. Altium writes the system code page; Windows-1252
/// is the overwhelmingly common one and is a superset of ASCII.
pub fn decode_ansi(bytes: &[u8]) -> String {
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
    text.into_owned()
}

/// The byte that stands for `|` inside a record value, where a raw `|`
/// would end the field (`docs/FORMAT_NOTES.md`).
const PIPE_ANSI: u8 = 0x8E;
/// The character the `%UTF8%` form of a value uses for the same `|`.
const PIPE_UTF8: char = '\u{A6}';

/// Decode one ANSI record value, `0x8E` standing for `|`.
fn decode_value(bytes: &[u8]) -> String {
    bytes
        .split(|&b| b == PIPE_ANSI)
        .map(decode_ansi)
        .collect::<Vec<_>>()
        .join("|")
}

/// The `%UTF8%` form `text` of a value whose ANSI form is `ansi`: the `¦`
/// that stand where the ANSI form has `0x8E` become `|`. The ANSI form's
/// `0x8E` and `0xA6` (a real `¦`) pair, in order, with the UTF-8 form's `¦`.
fn unescape_utf8(ansi: &[u8], text: &str) -> String {
    if !ansi.contains(&PIPE_ANSI) {
        return text.to_string();
    }
    let mut marks = ansi.iter().filter(|&&b| b == PIPE_ANSI || b == 0xA6);
    text.chars()
        .map(|c| match c {
            PIPE_UTF8 if marks.next() == Some(&PIPE_ANSI) => '|',
            c => c,
        })
        .collect()
}

impl Params {
    /// Parse a raw payload. Trailing NULs are ignored. A `%UTF8%KEY` entry
    /// carries the UTF-8 form of `KEY` and replaces the ANSI value. A `|`
    /// inside a value is byte `0x8E` in the ANSI form and `¦` at the same
    /// place in the UTF-8 form; see [`Params::ansi_pipe_assumed`].
    pub fn parse(raw: &[u8]) -> Self {
        struct Entry<'a> {
            key: String,
            value: String,
            ansi: Option<&'a [u8]>,
            utf8: bool,
        }
        let end = raw.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        let raw = &raw[..end];
        let mut entries: Vec<Entry> = Vec::new();
        let mut utf8: Vec<(String, &[u8])> = Vec::new();
        for field in raw.split(|&b| b == b'|') {
            if field.is_empty() {
                continue;
            }
            let Some(eq) = field.iter().position(|&b| b == b'=') else {
                continue;
            };
            let (k, v) = (&field[..eq], &field[eq + 1..]);
            let key = decode_ansi(k);
            if let Some(stripped) = key.strip_prefix("%UTF8%") {
                utf8.push((stripped.to_string(), v));
            } else {
                entries.push(Entry {
                    key,
                    value: decode_value(v),
                    ansi: Some(v),
                    utf8: false,
                });
            }
        }
        for (k, v) in utf8 {
            let text = String::from_utf8_lossy(v);
            match entries.iter_mut().find(|e| e.key.eq_ignore_ascii_case(&k)) {
                Some(e) => {
                    e.value = match e.ansi {
                        Some(a) => unescape_utf8(a, &text),
                        None => text.into_owned(),
                    };
                    e.utf8 = true;
                }
                None => entries.push(Entry {
                    key: k,
                    value: text.into_owned(),
                    ansi: None,
                    utf8: true,
                }),
            }
        }
        let ansi_pipe_assumed = entries
            .iter()
            .any(|e| !e.utf8 && e.ansi.is_some_and(|a| a.contains(&PIPE_ANSI)));
        Self {
            entries: entries.into_iter().map(|e| (e.key, e.value)).collect(),
            ansi_pipe_assumed,
        }
    }

    /// A value had byte `0x8E` and no `%UTF8%` form, and was read as `|`.
    /// In such a value a real `Ž` (Windows-1252 `0x8E`) cannot be told
    /// apart from an escaped `|`.
    pub fn ansi_pipe_assumed(&self) -> bool {
        self.ansi_pipe_assumed
    }

    pub fn from_pairs<K: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        Self {
            entries: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
            ansi_pipe_assumed: false,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn str(&self, key: &str) -> &str {
        self.get(key).unwrap_or("")
    }

    pub fn string(&self, key: &str) -> String {
        self.str(key).to_string()
    }

    pub fn i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(|v| v.trim().parse().ok())
    }

    pub fn i32_or(&self, key: &str, default: i32) -> i32 {
        self.i64(key)
            .and_then(|v| i32::try_from(v).ok())
            .unwrap_or(default)
    }

    pub fn f64(&self, key: &str) -> Option<f64> {
        self.get(key)
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
    }

    pub fn f64_or(&self, key: &str, default: f64) -> f64 {
        self.f64(key).unwrap_or(default)
    }

    /// Altium booleans are `T` / `F`; older files also use `TRUE` / `1`.
    pub fn bool(&self, key: &str) -> bool {
        matches!(
            self.get(key)
                .map(|v| v.trim().to_ascii_uppercase())
                .as_deref(),
            Some("T" | "TRUE" | "1")
        )
    }

    /// A length in a schematic file, in mils: the integer part counts
    /// 10-mil units and `KEY_FRAC` adds 1/100000 of a unit.
    pub fn sch_mils(&self, key: &str) -> f64 {
        let whole = self.i64(key).unwrap_or(0) as f64;
        let frac = self.i64(&format!("{key}_FRAC")).unwrap_or(0) as f64;
        (whole + frac / 100_000.0) * 10.0
    }

    /// A PCB text-record length such as `12.5mil` or `0.3mm`, in mils.
    /// A bare number is taken as mils.
    pub fn pcb_mils(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(parse_length_mils)
    }
}

/// Parse `"12.5mil"`, `"0.3mm"`, `"0.1in"` or a bare number (mils).
pub fn parse_length_mils(raw: &str) -> Option<f64> {
    let s = raw.trim().to_ascii_lowercase();
    let (num, scale) = if let Some(n) = s.strip_suffix("mil") {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix("mm") {
        (n, 1.0 / 0.0254)
    } else if let Some(n) = s.strip_suffix("in") {
        (n, 1000.0)
    } else {
        (s.as_str(), 1.0)
    };
    num.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .map(|v| v * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pairs_and_ignores_trailing_nul() {
        let p = Params::parse(b"|RECORD=1|LibReference=RES|OwnerPartId=-1\0");
        assert_eq!(p.str("record"), "1");
        assert_eq!(p.str("LIBREFERENCE"), "RES");
        assert_eq!(p.i64("OwnerPartId"), Some(-1));
    }

    #[test]
    fn utf8_variant_wins_over_ansi() {
        let p = Params::parse("|Text=?|%UTF8%Text=Ω|".as_bytes());
        assert_eq!(p.str("Text"), "Ω");
    }

    #[test]
    fn escaped_pipe_in_a_value() {
        // As in `simple.SchDoc`'s pin records: `SwapIDPart` is `|&|`.
        let p = Params::parse(b"|%UTF8%SwapIDPart=\xc2\xa6&\xc2\xa6|SwapIDPart=\x8e&\x8e|");
        assert_eq!(p.str("SwapIDPart"), "|&|");
        let p = Params::parse(b"|Name=Netlist|Text=A\x8eB\x8eC|");
        assert_eq!(p.str("Text"), "A|B|C");
        assert_eq!(p.str("Name"), "Netlist");
    }

    #[test]
    fn repeated_and_new_utf8_keys_mix_with_escapes() {
        // UTF-8-only keys (one repeated) after and between escaped ANSI ones.
        let p = Params::parse(
            b"|A=x\x8ey|%UTF8%New=n1|%UTF8%New=n2|B=b|%UTF8%A=x\xc2\xa6y|%UTF8%Other=o|%UTF8%B=c\xc2\xa6d|",
        );
        assert_eq!(p.str("A"), "x|y");
        assert_eq!(p.str("New"), "n2");
        assert_eq!(p.str("Other"), "o");
        assert_eq!(p.str("B"), "c\u{A6}d");
    }

    #[test]
    fn random_records_never_panic() {
        // A small deterministic generator over the bytes that matter here.
        let alphabet: &[&[u8]] = &[
            b"|",
            b"=",
            b"%UTF8%",
            b"A",
            b"b",
            b"\x8e",
            b"\xa6",
            b"\xc2\xa6",
            b"\0",
            b"\xff",
        ];
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..20_000 {
            let mut raw = Vec::new();
            for _ in 0..(state % 40) {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                raw.extend_from_slice(alphabet[(state % alphabet.len() as u64) as usize]);
            }
            let p = Params::parse(&raw);
            for (k, _) in p.iter() {
                assert!(p.get(k).is_some());
            }
        }
    }

    #[test]
    fn real_broken_bars_and_escaped_pipes_by_position() {
        let p = Params::parse(b"|%UTF8%Text=a\xc2\xa6b\xc2\xa6c\xc2\xa6d|Text=a\xa6b\x8ec\xa6d|");
        assert_eq!(p.str("Text"), "a\u{A6}b|c\u{A6}d");
        assert!(!p.ansi_pipe_assumed());
    }

    #[test]
    fn an_ansi_only_escape_is_read_as_a_pipe_and_flagged() {
        let p = Params::parse(b"|Name=Netlist|Text=a\x8eb|");
        assert_eq!(p.str("Text"), "a|b");
        assert!(p.ansi_pipe_assumed());
        let p = Params::parse(b"|Text=a\x8eb|%UTF8%Text=a\xc2\xa6b|");
        assert_eq!(p.str("Text"), "a|b");
        assert!(!p.ansi_pipe_assumed());
        assert!(!Params::parse(b"|Text=ab|").ansi_pipe_assumed());
    }

    #[test]
    fn broken_bar_without_an_escape_is_kept() {
        // A real `¦` is `0xA6` in the ANSI form; no `0x8E`, no rewrite.
        let p = Params::parse(b"|%UTF8%Text=a\xc2\xa6b|Text=a\xa6b|");
        assert_eq!(p.str("Text"), "a\u{A6}b");
        let p = Params::parse("|%UTF8%Text=a¦b|".as_bytes());
        assert_eq!(p.str("Text"), "a\u{A6}b");
    }

    #[test]
    fn value_may_contain_equals() {
        let p = Params::parse(b"|Text=a=b|");
        assert_eq!(p.str("Text"), "a=b");
    }

    #[test]
    fn schematic_coordinates_combine_frac() {
        let p = Params::parse(b"|Location.X=12|Location.X_Frac=50000|");
        assert!((p.sch_mils("Location.X") - 125.0).abs() < 1e-9);
    }

    #[test]
    fn length_units() {
        assert_eq!(parse_length_mils("10mil"), Some(10.0));
        assert!((parse_length_mils("0.254mm").unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(parse_length_mils("x"), None);
    }

    #[test]
    fn booleans() {
        let p = Params::parse(b"|A=T|B=F|C=TRUE|");
        assert!(p.bool("A") && !p.bool("B") && p.bool("C") && !p.bool("D"));
    }
}
