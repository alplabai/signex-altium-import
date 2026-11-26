//! Derived ids (contract section 5): the library id from the source's
//! identity, every other id from the library id and the item's source name.

use uuid::Uuid;

/// Namespace of every id this converter derives.
const NAMESPACE: Uuid = Uuid::from_u128(0x6b1e_5a42_93c0_4f0e_8d57_2a7c_91e4_b603);

/// The library id for a set of source files, identified by their file names
/// (lower case, sorted) so a moved or reordered input keeps its id.
pub fn library_id(file_names: &[String]) -> Uuid {
    let mut names: Vec<String> = file_names.iter().map(|n| n.to_lowercase()).collect();
    names.sort();
    Uuid::new_v5(
        &NAMESPACE,
        format!("library\n{}", names.join("\n")).as_bytes(),
    )
}

/// The id of a symbol, footprint, part or envelope within a library; `key`
/// is `<source library>:<item name>` plus a `#n` for a repeated name.
pub fn item_id(library: Uuid, kind: &str, key: &str) -> Uuid {
    Uuid::new_v5(&library, format!("{kind}\n{key}").as_bytes())
}

/// Ids for the items of one source library: `names` in source order, a
/// repeated name keyed `name#2`, `name#3`, ...
pub fn ids_for(library: Uuid, kind: &str, source: &str, names: &[&str]) -> Vec<Uuid> {
    let mut seen = std::collections::HashMap::<&str, usize>::new();
    names
        .iter()
        .map(|n| {
            let k = seen.entry(n).or_default();
            *k += 1;
            let key = if *k == 1 {
                format!("{source}:{n}")
            } else {
                format!("{source}:{n}#{k}")
            };
            item_id(library, kind, &key)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_id_ignores_order_and_case() {
        let a = library_id(&["B.SchLib".into(), "a.PcbLib".into()]);
        let b = library_id(&["A.pcblib".into(), "b.schlib".into()]);
        assert_eq!(a, b);
        assert_ne!(a, library_id(&["a.PcbLib".into()]));
    }

    #[test]
    fn repeated_names_get_distinct_stable_ids() {
        let lib = library_id(&["x.SchLib".into()]);
        let ids = ids_for(lib, "part", "x", &["R", "C", "R"]);
        assert_ne!(ids[0], ids[2]);
        assert_eq!(ids, ids_for(lib, "part", "x", &["R", "C", "R"]));
        assert_eq!(ids[1], item_id(lib, "part", "x:C"));
    }
}
