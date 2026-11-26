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

/// The class key for a prefix, from the stock library's class list. Only
/// the resistor, capacitor and inductor classes have a parameter template
/// today; the rest take the generic template's slots.
pub fn class_for_prefix(prefix: &str) -> &'static str {
    match prefix {
        "R" => "resistor",
        "C" => "capacitor",
        "L" => "inductor",
        "D" => "diode",
        "LED" => "led",
        "Q" => "transistor",
        "U" | "IC" => "ic",
        "J" | "P" | "CN" => "connector",
        "SW" | "S" => "switch",
        "Y" | "X" => "crystal",
        "H" | "MH" => "mechanical",
        "FID" => "fiducial",
        _ => "generic",
    }
}

/// Display label of a class key in the manifest's class list: the stock
/// library's labels.
pub fn class_label(key: &str) -> &'static str {
    match key {
        "resistor" => "Resistor",
        "capacitor" => "Capacitor",
        "inductor" => "Inductor",
        "diode" => "Diode",
        "led" => "LED",
        "transistor" => "Transistor",
        "ic" => "Integrated Circuit",
        "connector" => "Connector",
        "switch" => "Switch",
        "crystal" => "Crystal / Oscillator",
        "mechanical" => "Mechanical",
        "fiducial" => "Fiducial",
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
    fn prefixes_map_to_the_stock_classes() {
        for (p, c) in [
            ("R", "resistor"),
            ("C", "capacitor"),
            ("L", "inductor"),
            ("D", "diode"),
            ("LED", "led"),
            ("Q", "transistor"),
            ("U", "ic"),
            ("IC", "ic"),
            ("J", "connector"),
            ("P", "connector"),
            ("CN", "connector"),
            ("SW", "switch"),
            ("S", "switch"),
            ("Y", "crystal"),
            ("X", "crystal"),
            ("H", "mechanical"),
            ("MH", "mechanical"),
            ("FID", "fiducial"),
            ("ZZ", "generic"),
            ("", "generic"),
        ] {
            assert_eq!(class_for_prefix(p), c, "{p}");
            assert!(!class_label(c).is_empty());
        }
        assert_eq!(class_label("resistor"), "Resistor");
        assert_eq!(class_label("ic"), "Integrated Circuit");
    }
}
