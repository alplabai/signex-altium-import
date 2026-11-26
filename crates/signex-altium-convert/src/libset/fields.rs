//! Component parameters to part fields (`docs/CONVERSION.md`, "Part
//! fields"): manufacturer parts, supply, documents, description and the
//! rest as parameters.

use std::collections::BTreeMap;

use altium_format::sch::Component;
use serde_json::json;
use signex_library::manufacturer::{AlternateStatus, DistributorListing, ManufacturerPart};
use signex_library::param::{ParamMap, ParamValue};
use signex_library::templates::{ParamKind, ParameterTemplate};
use signex_library::{DatasheetDoc, DatasheetRef};
use signex_types::quantity::{quantity_from_parts, split_unit_string, unit_string_matches};

use crate::report::Report;

/// What the parameters of one component say.
pub struct Fields {
    pub description: String,
    /// Primary first; empty when the component has no document.
    pub docs: Vec<DatasheetDoc>,
    pub primary: ManufacturerPart,
    pub alternates: Vec<ManufacturerPart>,
    pub supply: Vec<DistributorListing>,
    pub parameters: ParamMap,
}

/// Parameter names that other fields take, lower case without separators.
fn norm(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// `Some(None)` for `base`, `Some(Some(n))` for `base<n>`, else `None`.
fn indexed(norm: &str, base: &str) -> Option<Option<u32>> {
    let rest = norm.strip_prefix(base)?;
    if rest.is_empty() {
        return Some(None);
    }
    if rest.bytes().all(|b| b.is_ascii_digit()) {
        return rest.parse().ok().map(Some);
    }
    None
}

fn first_indexed(norm: &str, bases: &[&str]) -> Option<u32> {
    bases
        .iter()
        .find_map(|b| indexed(norm, b))
        .map(|i| i.unwrap_or(1))
}

/// `componentlink<n>url` and `componentlink<n>description`.
fn link_index(norm: &str, tail: &str) -> Option<u32> {
    norm.strip_prefix("componentlink")?
        .strip_suffix(tail)?
        .parse()
        .ok()
}

/// Which template slot a parameter name fills.
fn slot_alias(norm: &str) -> Option<&'static str> {
    Some(match norm {
        "value" => "value",
        "tolerance" => "tolerance",
        "power" | "powerrating" | "ratedpower" => "power",
        "voltage" | "voltagerating" | "ratedvoltage" | "voltagemax" => "voltage_max",
        "current" | "currentrating" | "ratedcurrent" | "currentmax" => "current_max",
        "package" => "package",
        "dielectric" => "dielectric",
        "esr" => "esr",
        "dcr" => "dcr",
        "tcr" => "tcr",
        "srf" | "selfresonantfrequency" => "self_resonant_frequency",
        "rating" => "rating",
        _ => return None,
    })
}

/// A value cell: no control characters, no `'''` (the table is a TOML
/// literal string). Returns whether it had to change.
fn clean(text: &str) -> (String, bool) {
    let mut out: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    out = out.trim().to_string();
    let had = out.contains("'''");
    if had {
        out = out.replace("'''", "\u{2019}\u{2019}\u{2019}");
    }
    (out.clone(), out != text.trim() || had)
}

/// A measurement read from `text` for a slot measured in `slot_unit`.
enum Reading {
    Measured(f64),
    /// A number the unit does not settle (no unit, a bare prefix, a
    /// different unit, extra text).
    Ambiguous,
    NotANumber,
}

fn read_measurement(text: &str, slot_unit: &str) -> Reading {
    let s = text.trim().trim_start_matches(['\u{b1}', '+']).trim_start();
    let end = s
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || (i == 0 && c == '-')))
        .map_or(s.len(), |(i, _)| i);
    let (mantissa, rest) = s.split_at(end);
    if !mantissa.chars().any(|c| c.is_ascii_digit()) {
        return Reading::NotANumber;
    }
    let rest = rest.trim();
    let Some((prefix, unit)) = split_unit_string(rest) else {
        return Reading::Ambiguous;
    };
    if !unit_string_matches(unit.token(), slot_unit) {
        return Reading::Ambiguous;
    }
    match quantity_from_parts(mantissa, prefix, unit) {
        Ok(q) => Reading::Measured(q.base()),
        Err(_) => Reading::Ambiguous,
    }
}

pub fn map_fields(
    comp: &Component,
    template: Option<&ParameterTemplate>,
    report: &mut Report,
) -> Fields {
    let mut sanitized = false;
    let mut mfr: BTreeMap<u32, String> = BTreeMap::new();
    let mut mpn: BTreeMap<u32, String> = BTreeMap::new();
    let mut sup: BTreeMap<u32, String> = BTreeMap::new();
    let mut sku: BTreeMap<u32, String> = BTreeMap::new();
    let mut qty: BTreeMap<u32, String> = BTreeMap::new();
    // (priority, parameter name, url, label)
    let mut links: Vec<(u32, String, String)> = Vec::new();
    let mut link_labels: BTreeMap<u32, String> = BTreeMap::new();
    let mut parameters = ParamMap::new();
    let mut description = String::new();

    for p in &comp.parameters {
        let (text, changed) = clean(&p.text.text);
        sanitized |= changed;
        if text.is_empty() || text == "*" {
            continue;
        }
        let n = norm(&p.name);
        if n == "comment" {
            continue;
        }
        if n == "description" {
            if !description.is_empty() {
                report.warn_detail(
                    "part.param_dropped",
                    format!("parameter {:?} repeats the description; the later value replaced the earlier", p.name),
                    json!({ "parameter": p.name }),
                );
            }
            description = text;
            continue;
        }
        if let Some(i) = first_indexed(&n, &["manufacturerpartnumber", "manufacturerpn", "mpn"]) {
            put(&mut mpn, i, text, &p.name, report);
            continue;
        }
        if let Some(i) = first_indexed(&n, &["manufacturer", "mfr"]) {
            put(&mut mfr, i, text, &p.name, report);
            continue;
        }
        if let Some(i) = first_indexed(&n, &["supplierpartnumber", "supplierpn"]) {
            put(&mut sku, i, text, &p.name, report);
            continue;
        }
        if let Some(i) = first_indexed(&n, &["supplierorderqty", "supplierqty"]) {
            put(&mut qty, i, text, &p.name, report);
            continue;
        }
        if let Some(i) = first_indexed(&n, &["supplier"]) {
            put(&mut sup, i, text, &p.name, report);
            continue;
        }
        match n.as_str() {
            "datasheet" => {
                links.push((0, p.name.trim().to_string(), text));
                continue;
            }
            "helpurl" => {
                links.push((1, p.name.trim().to_string(), text));
                continue;
            }
            _ => {}
        }
        if let Some(i) = link_index(&n, "url") {
            links.push((1 + i, p.name.trim().to_string(), text));
            continue;
        }
        if let Some(i) = link_index(&n, "description") {
            link_labels.insert(1 + i, text);
            continue;
        }
        add_parameter(&mut parameters, &p.name, &n, text, template, report);
    }

    links.sort_by_key(|(prio, ..)| *prio);
    let docs = links
        .into_iter()
        .map(|(prio, name, url)| {
            let label = link_labels
                .get(&prio)
                .cloned()
                .unwrap_or_else(|| if prio == 0 { String::new() } else { name });
            DatasheetDoc::new(label, DatasheetRef::url(url))
        })
        .collect();

    let (primary, alternates) = manufacturer_parts(&mfr, &mpn);
    let supply = supply_listings(&sup, &sku, &qty, report);
    if description.is_empty() {
        description = clean(&comp.description).0;
    }
    if sanitized {
        report.warn(
            "part.text_sanitized",
            "control characters or a triple quote in a parameter replaced",
        );
    }
    Fields {
        description,
        docs,
        primary,
        alternates,
        supply,
        parameters,
    }
}

/// Record a numbered value; a repeat of the same number keeps the later
/// value and says so.
fn put(map: &mut BTreeMap<u32, String>, index: u32, text: String, name: &str, report: &mut Report) {
    if map.insert(index, text).is_some() {
        report.warn_detail(
            "part.param_dropped",
            format!(
                "parameter {name:?} repeats number {index}; the later value replaced the earlier"
            ),
            json!({ "parameter": name, "index": index }),
        );
    }
}

fn manufacturer_parts(
    mfr: &BTreeMap<u32, String>,
    mpn: &BTreeMap<u32, String>,
) -> (ManufacturerPart, Vec<ManufacturerPart>) {
    let mut keys: Vec<u32> = mfr.keys().chain(mpn.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    let mut parts = keys.into_iter().map(|k| {
        let mut p = ManufacturerPart::draft(
            mfr.get(&k).cloned().unwrap_or_default(),
            mpn.get(&k).cloned().unwrap_or_default(),
        );
        p.status = AlternateStatus::Approved;
        p
    });
    let primary = parts
        .next()
        .map(|mut p| {
            p.status = AlternateStatus::Primary;
            p
        })
        .unwrap_or_else(|| ManufacturerPart::draft("", ""));
    (primary, parts.collect())
}

fn supply_listings(
    sup: &BTreeMap<u32, String>,
    sku: &BTreeMap<u32, String>,
    qty: &BTreeMap<u32, String>,
    report: &mut Report,
) -> Vec<DistributorListing> {
    let mut keys: Vec<u32> = sup.keys().chain(sku.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    let mut out = Vec::new();
    for k in keys {
        let (Some(d), Some(s)) = (sup.get(&k), sku.get(&k)) else {
            report.warn_detail(
                "part.param_ambiguous",
                format!("supplier {k} needs both a supplier and a part number; not imported"),
                json!({ "supplier_index": k }),
            );
            continue;
        };
        let mut l = DistributorListing::new(d.clone(), s.clone());
        if let Some(q) = qty.get(&k) {
            match q.trim().parse::<u32>() {
                Ok(n) => l.moq = Some(n),
                Err(_) => report.warn_detail(
                    "part.param_ambiguous",
                    format!(
                        "supplier {k} order quantity {q:?} is not a whole number; not imported"
                    ),
                    json!({ "supplier_index": k, "value": q }),
                ),
            }
        }
        out.push(l);
    }
    out
}

fn add_parameter(
    out: &mut ParamMap,
    name: &str,
    n: &str,
    text: String,
    template: Option<&ParameterTemplate>,
    report: &mut Report,
) {
    let slot = slot_alias(n).and_then(|s| template?.slot(s));
    let key = slot.map_or_else(|| name.trim().to_string(), |s| s.name.clone());
    if key.is_empty() || key == signex_library::component::DESIGNATOR_PREFIX_PARAM {
        report.warn_detail(
            "part.param_dropped",
            format!("parameter {name:?} has no usable name; not imported"),
            json!({ "parameter": name }),
        );
        return;
    }
    let value = match slot {
        Some(s) if s.kind == ParamKind::Measurement => {
            let unit = s.unit.clone().unwrap_or_default();
            match read_measurement(&text, &unit) {
                Reading::Measured(v) => ParamValue::Measurement { value: v, unit },
                Reading::Ambiguous => {
                    report.warn_detail(
                        "part.param_ambiguous",
                        format!("{name} = {text:?} kept as text: no clear {unit} value"),
                        json!({ "parameter": name, "value": text, "unit": unit }),
                    );
                    ParamValue::Text(text)
                }
                Reading::NotANumber => ParamValue::Text(text),
            }
        }
        _ => ParamValue::Text(text),
    };
    if out.insert(key.clone(), value).is_some() {
        report.warn_detail(
            "part.param_dropped",
            format!("parameter {name:?} repeats {key:?}; the later value replaced the earlier"),
            json!({ "parameter": name }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use altium_format::sch::{Parameter, Text};
    use signex_library::templates::TemplateRegistry;
    use uuid::Uuid;

    fn comp(params: &[(&str, &str)]) -> Component {
        Component {
            parameters: params
                .iter()
                .map(|(n, v)| Parameter {
                    name: (*n).into(),
                    text: Text {
                        text: (*v).into(),
                        ..Default::default()
                    },
                })
                .collect(),
            ..Default::default()
        }
    }

    fn map(class: &str, params: &[(&str, &str)]) -> (Fields, Report) {
        let reg = TemplateRegistry::new_with_builtins();
        let mut r = Report::default();
        let f = map_fields(&comp(params), reg.resolve(Uuid::nil(), class), &mut r);
        (f, r)
    }

    #[test]
    fn manufacturers_primary_then_alternates() {
        let (f, _) = map(
            "generic",
            &[
                ("Manufacturer", "Acme"),
                ("Manufacturer Part Number", "A-1"),
                ("Manufacturer 2", "Beta"),
                ("Manufacturer Part Number 2", "B-2"),
                ("Manufacturer Part Number 3", "C-3"),
            ],
        );
        assert_eq!(
            (f.primary.manufacturer.as_str(), f.primary.mpn.as_str()),
            ("Acme", "A-1")
        );
        assert_eq!(f.primary.status, AlternateStatus::Primary);
        assert_eq!(f.alternates.len(), 2);
        assert_eq!(
            (
                f.alternates[0].manufacturer.as_str(),
                f.alternates[0].mpn.as_str()
            ),
            ("Beta", "B-2")
        );
        assert_eq!(f.alternates[1].mpn, "C-3");
        assert_eq!(f.alternates[1].status, AlternateStatus::Approved);
    }

    #[test]
    fn supply_with_order_quantity() {
        let (f, r) = map(
            "generic",
            &[
                ("Supplier 1", "DigiKey"),
                ("Supplier Part Number 1", "311-10KJRCT-ND"),
                ("Supplier Order Qty 1", "100"),
                ("Supplier 2", "Mouser"),
            ],
        );
        assert_eq!(f.supply.len(), 1);
        assert_eq!(f.supply[0].distributor, "DigiKey");
        assert_eq!(f.supply[0].sku, "311-10KJRCT-ND");
        assert_eq!(f.supply[0].moq, Some(100));
        assert_eq!(r.items().len(), 1, "supplier 2 has no part number");
        assert_eq!(r.items()[0].code, "part.param_ambiguous");
    }

    #[test]
    fn documents_primary_and_more() {
        let (f, _) = map(
            "generic",
            &[
                ("HelpURL", "https://example.com/help"),
                ("Datasheet", "https://example.com/ds.pdf"),
                ("ComponentLink2URL", "https://example.com/errata"),
                ("ComponentLink2Description", "Errata"),
            ],
        );
        assert_eq!(f.docs.len(), 3);
        assert_eq!(f.docs[0].label, "");
        assert_eq!(
            f.docs[0].source,
            DatasheetRef::url("https://example.com/ds.pdf")
        );
        assert_eq!(f.docs[1].label, "HelpURL");
        assert_eq!(f.docs[2].label, "Errata");
    }

    #[test]
    fn description_prefers_the_parameter_then_the_component() {
        let (f, _) = map("generic", &[("Description", "Thick film")]);
        assert_eq!(f.description, "Thick film");
        let mut c = comp(&[]);
        c.description = "From the record".into();
        let mut r = Report::default();
        assert_eq!(map_fields(&c, None, &mut r).description, "From the record");
    }

    #[test]
    fn measurements_only_with_a_unit_the_template_knows() {
        let (f, r) = map(
            "resistor",
            &[
                ("Value", "4.7 kOhm"),
                ("Tolerance", "\u{b1}1%"),
                ("Power", "0.1W"),
                ("Voltage", "50"),
                ("TCR", "100ppm/C"),
                ("Package", "0603"),
                ("Series", "RC"),
            ],
        );
        let p = &f.parameters;
        assert_eq!(
            p["value"],
            ParamValue::Measurement {
                value: 4700.0,
                unit: "ohm".into()
            }
        );
        assert_eq!(
            p["tolerance"],
            ParamValue::Measurement {
                value: 1.0,
                unit: "%".into()
            }
        );
        assert_eq!(
            p["power"],
            ParamValue::Measurement {
                value: 0.1,
                unit: "W".into()
            }
        );
        assert_eq!(
            p["tcr"],
            ParamValue::Measurement {
                value: 100.0,
                unit: "ppm/C".into()
            }
        );
        // A bare number: ambiguous, kept as text, reported as info.
        assert_eq!(p["voltage_max"], ParamValue::Text("50".into()));
        assert_eq!(p["package"], ParamValue::Text("0603".into()));
        assert_eq!(p["Series"], ParamValue::Text("RC".into()));
        assert_eq!(r.items().len(), 1);
        assert_eq!(r.items()[0].code, "part.param_ambiguous");
    }

    #[test]
    fn a_bare_prefix_is_ambiguous() {
        let (f, r) = map("resistor", &[("Value", "10k")]);
        assert_eq!(f.parameters["value"], ParamValue::Text("10k".into()));
        assert_eq!(r.items()[0].code, "part.param_ambiguous");
    }

    #[test]
    fn unknown_class_keeps_everything_as_text() {
        let (f, r) = map("generic", &[("Value", "10k"), ("Voltage", "5V")]);
        assert_eq!(f.parameters["value"], ParamValue::Text("10k".into()));
        assert_eq!(f.parameters["Voltage"], ParamValue::Text("5V".into()));
        assert!(r.is_empty());
    }

    #[test]
    fn control_characters_and_triple_quotes_are_replaced() {
        let (f, r) = map("generic", &[("Note", "a\tb'''c\n")]);
        assert_eq!(
            f.parameters["Note"],
            ParamValue::Text("a b\u{2019}\u{2019}\u{2019}c".into())
        );
        assert_eq!(r.items()[0].code, "part.text_sanitized");
    }

    #[test]
    fn blank_and_star_values_and_comment_are_skipped() {
        let (f, r) = map("generic", &[("Comment", "10k"), ("X", "*"), ("Y", " ")]);
        assert!(f.parameters.is_empty() && r.is_empty());
    }

    #[test]
    fn a_repeated_number_or_description_keeps_the_later_and_reports() {
        let (f, r) = map(
            "generic",
            &[
                ("Manufacturer", "Acme"),
                ("Manufacturer 1", "Other"),
                ("Manufacturer Part Number", "A"),
                ("Description", "one"),
                ("Description", "two"),
            ],
        );
        assert_eq!(f.primary.manufacturer, "Other");
        assert_eq!(f.description, "two");
        assert_eq!(
            r.items()
                .iter()
                .filter(|i| i.code == "part.param_dropped")
                .count(),
            2
        );
    }

    #[test]
    fn a_repeated_parameter_is_reported() {
        let (f, r) = map("generic", &[("X", "1"), ("X", "2")]);
        assert_eq!(f.parameters["X"], ParamValue::Text("2".into()));
        assert_eq!(r.items()[0].code, "part.param_dropped");
    }
}
