//! Conversion diagnostics.

use std::collections::BTreeMap;
use std::fmt;

/// Things a conversion could not carry over exactly. Grouped by message
/// so a board with 4000 rotated pads reports one line, not 4000.
#[derive(Debug, Clone, Default)]
pub struct Report {
    counts: BTreeMap<String, usize>,
}

impl Report {
    pub fn warn(&mut self, message: impl Into<String>) {
        *self.counts.entry(message.into()).or_default() += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    pub fn merge(&mut self, other: Report) {
        for (k, v) in other.counts {
            *self.counts.entry(k).or_default() += v;
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, usize)> {
        self.counts.iter().map(|(k, v)| (k.as_str(), *v))
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (msg, n) in &self.counts {
            if *n == 1 {
                writeln!(f, "  - {msg}")?;
            } else {
                writeln!(f, "  - {msg} (x{n})")?;
            }
        }
        Ok(())
    }
}
