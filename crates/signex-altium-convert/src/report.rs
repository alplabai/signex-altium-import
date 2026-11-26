//! Conversion diagnostics: coded items, grouped by message for the legacy
//! single-file summary and kept per item for the library report.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Value, json};

use crate::codes;

/// What a diagnostic cost, in the library converter contract's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Nothing lost.
    Info,
    /// Written, with a documented approximation.
    Approximated,
    /// Written, but part of it was not.
    Dropped,
    /// The item was not written at all.
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Approximated => "approximated",
            Severity::Dropped => "dropped",
            Severity::Error => "error",
        }
    }
}

/// What a report item is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Symbol,
    Footprint,
    Part,
    Model,
    Row,
    Library,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemKind::Symbol => "symbol",
            ItemKind::Footprint => "footprint",
            ItemKind::Part => "part",
            ItemKind::Model => "model",
            ItemKind::Row => "row",
            ItemKind::Library => "library",
        }
    }

    /// The kind a code's `<area>` implies; board and sheet items count as
    /// `library` (they have no library-contract kind).
    fn of_code(code: &str) -> Self {
        match code.split('.').next() {
            Some("symbol") => ItemKind::Symbol,
            Some("footprint") => ItemKind::Footprint,
            _ => ItemKind::Library,
        }
    }
}

/// One thing a conversion could not carry over exactly.
#[derive(Debug, Clone)]
pub struct Item {
    pub kind: ItemKind,
    /// The item's name in the source (`<library>:<item>` where it has one);
    /// empty when the site does not know it.
    pub source: String,
    /// The Signex id written for it, `None` if not written.
    pub id: Option<String>,
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub detail: Value,
}

impl Item {
    pub fn to_json(&self) -> Value {
        json!({
            "kind": self.kind.as_str(),
            "source": self.source,
            "id": self.id,
            "code": self.code,
            "severity": self.severity.as_str(),
            "message": self.message,
            "detail": self.detail,
        })
    }
}

/// Things a conversion could not carry over exactly. `entries` groups them
/// by message so a board with 4000 rotated pads reports one line, not 4000.
#[derive(Debug, Clone, Default)]
pub struct Report {
    items: Vec<Item>,
    scope: String,
}

impl Report {
    /// Record an approximation or loss under a catalogued code
    /// (`docs/CONVERSION.md`, "Report codes").
    pub fn warn(&mut self, code: &'static str, message: impl Into<String>) {
        self.warn_detail(code, message, Value::Null);
    }

    /// As [`Report::warn`], with the code's specifics.
    pub fn warn_detail(&mut self, code: &'static str, message: impl Into<String>, detail: Value) {
        self.items.push(Item {
            kind: ItemKind::of_code(code),
            source: self.scope.clone(),
            id: None,
            code,
            severity: codes::severity(code),
            message: message.into(),
            detail,
        });
    }

    /// Push a fully formed item (library-level errors, per-item reports).
    pub fn push(&mut self, item: Item) {
        self.items.push(item);
    }

    /// Name the source item later warnings belong to (empty to clear).
    pub fn set_scope(&mut self, source: impl Into<String>) {
        self.scope = source.into();
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn merge(&mut self, other: Report) {
        self.items.extend(other.items);
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// `(message, count)`, ordered by message.
    pub fn entries(&self) -> impl Iterator<Item = (&str, usize)> {
        self.grouped(|_| true).into_iter()
    }

    /// As [`Report::entries`], for the non-`info` items only.
    pub fn warnings(&self) -> Vec<(&str, usize)> {
        self.grouped(|i| i.severity > Severity::Info)
    }

    fn grouped(&self, keep: impl Fn(&Item) -> bool) -> Vec<(&str, usize)> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for i in self.items.iter().filter(|i| keep(i)) {
            *counts.entry(i.message.as_str()).or_default() += 1;
        }
        counts.into_iter().collect()
    }

    /// The worst severity present, `None` for an empty report.
    pub fn worst(&self) -> Option<Severity> {
        self.items.iter().map(|i| i.severity).max()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (msg, n) in self.entries() {
            if n == 1 {
                writeln!(f, "  - {msg}")?;
            } else {
                writeln!(f, "  - {msg} (x{n})")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_by_message_like_before() {
        let mut r = Report::default();
        r.warn("pcb.copper_arc_flattened", "arc");
        r.warn("pcb.copper_arc_flattened", "arc");
        r.warn("pcb.pour_hatch_solid", "hatch");
        assert_eq!(
            r.entries().collect::<Vec<_>>(),
            vec![("arc", 2), ("hatch", 1)]
        );
        assert_eq!(format!("{r}"), "  - arc (x2)\n  - hatch\n");
    }

    #[test]
    fn scope_and_kind_follow_the_code() {
        let mut r = Report::default();
        r.set_scope("Lib:R");
        r.warn("symbol.bezier_polygon", "b");
        let i = &r.items()[0];
        assert_eq!((i.kind, i.source.as_str()), (ItemKind::Symbol, "Lib:R"));
        assert_eq!(i.severity, Severity::Approximated);
    }

    #[test]
    fn info_is_not_a_warning() {
        let mut r = Report::default();
        r.warn("project.several_boards", "x");
        assert!(r.warnings().is_empty());
        assert_eq!(r.worst(), Some(Severity::Info));
    }
}
