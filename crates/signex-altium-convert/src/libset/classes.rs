//! Part class from the designator prefix.

/// The leading ASCII letters of a designator (`R?` and `R12` give `R`),
/// upper case; empty when there are none.
pub fn designator_prefix(designator: &str) -> String {
    designator
        .trim()
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect::<String>()
        .to_ascii_uppercase()
}

/// The class key for a prefix. Only classes the vendored templates know
/// are used (`resistor`, `capacitor`, `inductor`, `opamp`, `generic`);
/// everything else, diodes, transistors, ICs, connectors, switches,
/// crystals, mechanical parts and fiducials included, is `generic` until a
/// template exists for it.
pub fn class_for_prefix(prefix: &str) -> &'static str {
    match prefix {
        "R" => "resistor",
        "C" => "capacitor",
        "L" => "inductor",
        _ => "generic",
    }
}

/// Display label of a class key in the manifest's class list.
pub fn class_label(key: &str) -> &'static str {
    match key {
        "resistor" => "Resistors",
        "capacitor" => "Capacitors",
        "inductor" => "Inductors",
        "opamp" => "Op-amps",
        _ => "Generic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes() {
        assert_eq!(designator_prefix("R?"), "R");
        assert_eq!(designator_prefix(" led12"), "LED");
        assert_eq!(designator_prefix("?"), "");
        assert_eq!(designator_prefix(""), "");
    }

    #[test]
    fn classes_are_template_classes_only() {
        for (p, c) in [
            ("R", "resistor"),
            ("C", "capacitor"),
            ("L", "inductor"),
            ("D", "generic"),
            ("LED", "generic"),
            ("Q", "generic"),
            ("U", "generic"),
            ("IC", "generic"),
            ("J", "generic"),
            ("SW", "generic"),
            ("Y", "generic"),
            ("FID", "generic"),
            ("", "generic"),
        ] {
            assert_eq!(class_for_prefix(p), c, "{p}");
        }
    }
}
