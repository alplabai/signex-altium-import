//! `|KEY=VALUE|KEY=VALUE` property lists — the text payload of most records.

/// One record's properties, in file order.
///
/// Keys compare case-insensitively: the same property shows up as
/// `LOCATION.X` in one file version and `Location.X` in another.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Params {
    entries: Vec<(String, String)>,
}

/// Decode legacy ANSI text. Altium writes the system code page; Windows-1252
/// is the overwhelmingly common one and is a superset of ASCII.
pub fn decode_ansi(bytes: &[u8]) -> String {
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
    text.into_owned()
}

impl Params {
    /// Parse a raw payload. Trailing NULs are ignored. A `%UTF8%KEY` entry
    /// carries the UTF-8 form of `KEY` and replaces the ANSI value.
    pub fn parse(raw: &[u8]) -> Self {
        let end = raw.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        let raw = &raw[..end];
        let mut entries: Vec<(String, String)> = Vec::new();
        let mut utf8: Vec<(String, String)> = Vec::new();
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
                utf8.push((
                    stripped.to_string(),
                    String::from_utf8_lossy(v).into_owned(),
                ));
            } else {
                entries.push((key, decode_ansi(v)));
            }
        }
        for (k, v) in utf8 {
            match entries
                .iter_mut()
                .find(|(ek, _)| ek.eq_ignore_ascii_case(&k))
            {
                Some(slot) => slot.1 = v,
                None => entries.push((k, v)),
            }
        }
        Self { entries }
    }

    pub fn from_pairs<K: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        Self {
            entries: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
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
