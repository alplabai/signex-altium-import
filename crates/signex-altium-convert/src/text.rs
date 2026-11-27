//! Text that goes into a TSV cell or a TOML literal string. The library
//! writer refuses a tab, a control character or a triple apostrophe there,
//! and one refusal fails the whole library, so every such string is cleaned.

/// `text` safe for a cell, and whether it had to change.
pub fn cell_safe(text: &str) -> (String, bool) {
    let mut out: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if out.contains("'''") {
        out = out.replace("'''", "\u{2019}\u{2019}\u{2019}");
    }
    let changed = out != text;
    (out, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_and_triple_quotes_are_replaced() {
        assert_eq!(cell_safe("ok"), ("ok".into(), false));
        assert_eq!(cell_safe("a\tb\r\nc\u{7f}"), ("a b  c ".into(), true));
        assert_eq!(cell_safe("x'''y").0, "x\u{2019}\u{2019}\u{2019}y");
    }
}
