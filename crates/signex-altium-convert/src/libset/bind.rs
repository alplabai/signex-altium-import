//! Component implementations to footprint bindings: the current PCB model
//! is the default footprint, the other PCB models are alternates in list
//! order, and a pin map that is not 1:1 becomes pin map overrides.

use std::collections::{HashMap, HashSet};

use altium_format::sch::{Component, Implementation};
use serde_json::json;
use signex_library::PrimitiveRef;
use signex_library::component::PinPadOverride;
use signex_library::footprint_binding::FootprintBinding;
use uuid::Uuid;

use crate::report::Report;

/// Footprints by name, case-insensitive, first one wins.
#[derive(Default)]
pub struct FootprintIndex {
    by_name: HashMap<String, Uuid>,
}

impl FootprintIndex {
    /// Add a footprint; `false` when the name is already taken.
    pub fn insert(&mut self, name: &str, id: Uuid) -> bool {
        let key = name.trim().to_lowercase();
        if self.by_name.contains_key(&key) {
            return false;
        }
        self.by_name.insert(key, id);
        true
    }

    fn get(&self, name: &str) -> Option<Uuid> {
        self.by_name.get(&name.trim().to_lowercase()).copied()
    }
}

/// What a component binds.
#[derive(Debug, Default, PartialEq)]
pub struct Binding {
    pub default: Option<Uuid>,
    /// Pin map of the default footprint (row column `pin_map_overrides`).
    pub default_overrides: Vec<PinPadOverride>,
    pub alternates: Vec<(Uuid, Vec<PinPadOverride>)>,
}

impl Binding {
    pub fn alternate_bindings(&self, library: Uuid) -> Vec<FootprintBinding> {
        self.alternates
            .iter()
            .map(|(id, map)| FootprintBinding {
                footprint: PrimitiveRef::new(library, *id),
                version: String::new(),
                pin_map_overrides: map.clone(),
            })
            .collect()
    }
}

fn overrides(i: &Implementation) -> Vec<PinPadOverride> {
    i.pin_map
        .iter()
        .filter(|(pin, pad)| pin != pad)
        .map(|(pin, pad)| PinPadOverride::new(pin.clone(), pad.clone()))
        .collect()
}

/// Bind `comp`'s PCB models. `None` means the part must be skipped; the
/// error item is already in `report`.
pub fn bind(comp: &Component, index: &FootprintIndex, report: &mut Report) -> Option<Binding> {
    let mut pcb: Vec<&Implementation> = Vec::new();
    for i in &comp.implementations {
        if i.model_type.eq_ignore_ascii_case("PCBLIB") {
            if !i.model_name.trim().is_empty() {
                pcb.push(i);
            }
        } else if !i.model_type.is_empty() || !i.model_name.is_empty() {
            report.warn_detail(
                "part.model_skipped",
                format!("{} model {:?} not imported", i.model_type, i.model_name),
                json!({ "model_type": i.model_type, "model_name": i.model_name }),
            );
        }
    }
    if pcb.is_empty() {
        return Some(Binding::default());
    }
    let current = pcb.iter().position(|i| i.is_current);
    if current.is_none() && pcb.len() > 1 {
        report.warn(
            "part.footprint_no_current",
            "no footprint is marked current; the first is the default",
        );
    }
    let default_at = current.unwrap_or(0);
    let order = std::iter::once(default_at).chain((0..pcb.len()).filter(|k| *k != default_at));

    let mut out = Binding::default();
    let mut seen = HashSet::new();
    for k in order {
        let i = pcb[k];
        let Some(id) = index.get(&i.model_name) else {
            report.warn_detail(
                "part.footprint_missing",
                format!(
                    "footprint {} not found in the given libraries; part skipped",
                    i.model_name
                ),
                json!({ "footprint": i.model_name }),
            );
            return None;
        };
        if !seen.insert(id) {
            report.warn_detail(
                "part.model_skipped",
                format!(
                    "footprint {:?} is bound twice; the repeat is not imported",
                    i.model_name
                ),
                json!({ "model_type": i.model_type, "model_name": i.model_name }),
            );
            continue;
        }
        if out.default.is_none() {
            out.default = Some(id);
            out.default_overrides = overrides(i);
        } else {
            out.alternates.push((id, overrides(i)));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imp(name: &str, current: bool, map: &[(&str, &str)]) -> Implementation {
        Implementation {
            model_name: name.into(),
            model_type: "PCBLIB".into(),
            is_current: current,
            pin_map: map
                .iter()
                .map(|(a, b)| ((*a).into(), (*b).into()))
                .collect(),
        }
    }

    fn index() -> (FootprintIndex, [Uuid; 3]) {
        let ids = [Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)];
        let mut ix = FootprintIndex::default();
        for (n, id) in ["A", "B", "C"].iter().zip(ids) {
            assert!(ix.insert(n, id));
        }
        (ix, ids)
    }

    fn comp(list: Vec<Implementation>) -> Component {
        Component {
            implementations: list,
            ..Default::default()
        }
    }

    #[test]
    fn no_implementation_means_no_footprint() {
        let (ix, _) = index();
        let mut r = Report::default();
        assert_eq!(bind(&comp(vec![]), &ix, &mut r), Some(Binding::default()));
        assert!(r.is_empty());
    }

    #[test]
    fn one_footprint_is_the_default() {
        let (ix, ids) = index();
        let mut r = Report::default();
        let b = bind(&comp(vec![imp("a", true, &[])]), &ix, &mut r).unwrap();
        assert_eq!(b.default, Some(ids[0]));
        assert!(b.alternates.is_empty() && b.default_overrides.is_empty());
    }

    #[test]
    fn the_current_model_is_the_default_and_the_rest_keep_list_order() {
        let (ix, ids) = index();
        let mut r = Report::default();
        let list = vec![
            imp("A", false, &[]),
            imp("B", true, &[]),
            imp("C", false, &[]),
        ];
        let b = bind(&comp(list), &ix, &mut r).unwrap();
        assert_eq!(b.default, Some(ids[1]));
        assert_eq!(
            b.alternates.iter().map(|a| a.0).collect::<Vec<_>>(),
            [ids[0], ids[2]]
        );
        assert!(r.is_empty());
    }

    #[test]
    fn without_a_current_flag_the_first_is_the_default() {
        let (ix, ids) = index();
        let mut r = Report::default();
        let b = bind(
            &comp(vec![imp("B", false, &[]), imp("C", false, &[])]),
            &ix,
            &mut r,
        )
        .unwrap();
        assert_eq!(b.default, Some(ids[1]));
        assert_eq!(r.items()[0].code, "part.footprint_no_current");
    }

    #[test]
    fn only_remapped_pins_become_overrides() {
        let (ix, ids) = index();
        let mut r = Report::default();
        let list = vec![
            imp("A", true, &[("1", "1"), ("2", "3")]),
            imp("B", false, &[("1", "2")]),
        ];
        let b = bind(&comp(list), &ix, &mut r).unwrap();
        assert_eq!(b.default_overrides, vec![PinPadOverride::new("2", "3")]);
        assert_eq!(
            b.alternates,
            vec![(ids[1], vec![PinPadOverride::new("1", "2")])]
        );
    }

    #[test]
    fn a_missing_footprint_skips_the_part() {
        let (ix, _) = index();
        let mut r = Report::default();
        let list = vec![imp("A", true, &[]), imp("Nope", false, &[])];
        assert_eq!(bind(&comp(list), &ix, &mut r), None);
        assert_eq!(r.items()[0].code, "part.footprint_missing");
        assert_eq!(r.items()[0].severity, crate::report::Severity::Error);
    }

    #[test]
    fn other_model_kinds_are_reported_as_info() {
        let (ix, ids) = index();
        let mut r = Report::default();
        let mut sim = imp("SPICE1", false, &[]);
        sim.model_type = "SIM".into();
        let b = bind(&comp(vec![sim, imp("A", true, &[])]), &ix, &mut r).unwrap();
        assert_eq!(b.default, Some(ids[0]));
        assert_eq!(r.items()[0].code, "part.model_skipped");
        assert_eq!(r.worst(), Some(crate::report::Severity::Info));
    }

    #[test]
    fn a_footprint_bound_twice_is_bound_once() {
        let (ix, ids) = index();
        let mut r = Report::default();
        let b = bind(
            &comp(vec![imp("A", true, &[]), imp("a", false, &[])]),
            &ix,
            &mut r,
        )
        .unwrap();
        assert_eq!(b.default, Some(ids[0]));
        assert!(b.alternates.is_empty());
        assert_eq!(r.items().len(), 1);
    }
}
