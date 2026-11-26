//! The catalogue of report codes. Each code is `<area>.<what>` and never
//! changes meaning; `docs/CONVERSION.md` ("Report codes") lists them with
//! the same severities.

use crate::report::Severity::{self, Approximated as A, Dropped as D, Error as E, Info as I};

/// Every code with its severity.
pub const CODES: &[(&str, Severity)] = &[
    // Library symbols.
    ("symbol.pin_ieee_unmapped", D),
    ("symbol.bezier_polygon", A),
    ("symbol.rounded_rect_radius", A),
    ("symbol.unreadable", E),
    ("part.symbol_unreadable", E),
    ("symbol.display_mode_dropped", D),
    ("symbol.pin_decoration_merged", A),
    ("symbol.text_sanitized", A),
    ("footprint.text_sanitized", A),
    // Schematic sheets.
    ("sch.rounded_rect_square", A),
    ("sch.bezier_polygon", A),
    ("sch.image_dropped", D),
    ("sch.record_dropped", D),
    // Library footprints.
    ("footprint.courtyard_text_dropped", D),
    ("footprint.graphic_layer_unmapped", D),
    ("footprint.copper_track_dropped", D),
    ("footprint.fill_dropped", D),
    ("footprint.region_dropped", D),
    ("footprint.courtyard_open_bbox", A),
    ("footprint.pad_octagon_custom", A),
    ("pad.slot_not_placeable", A),
    // Boards.
    ("pcb.keepout_line_no_width", D),
    ("pcb.keepout_non_copper", D),
    ("pcb.footprint_keepout_moved", A),
    ("pcb.keepout_restrictions_merged", A),
    ("pcb.component_uid_dropped", D),
    ("pcb.free_pad_holder", A),
    ("pcb.plane_split_line_dropped", D),
    ("pcb.copper_arc_flattened", A),
    ("pcb.plane_void_keepout", A),
    ("pcb.pour_hatch_solid", A),
    ("pcb.region_too_few_points", D),
    ("pcb.footprint_region_dropped", D),
    ("pcb.region_cutout_dropped", D),
    ("pcb.body_dropped", D),
    ("pcb.primitive_dropped", D),
    ("pcb.plane_no_outline", D),
    ("pcb.split_plane_nets_lost", D),
    ("pcb.plane_no_net", A),
    ("pcb.pullback_not_applied", A),
    ("pcb.layer_unmapped", A),
    ("pcb.pad_rotation_dropped", A),
    ("pcb.pad_octagon_rect", A),
    // Projects.
    ("project.several_boards", I),
    // Parts of a library run.
    ("part.pn_collision", A),
    ("part.footprint_missing", E),
    ("part.pin_map_unread", E),
    ("part.footprint_no_current", I),
    ("part.model_skipped", I),
    ("part.param_ambiguous", I),
    ("part.param_dropped", D),
    ("part.text_sanitized", I),
    ("part.template_incomplete", I),
    ("part.row_invalid", E),
    ("footprint.name_collision", A),
    // 3D models.
    ("model.body_without_model", I),
    ("model.skipped", I),
    ("model.extra_body", E),
    ("model.missing", E),
    ("model.external", E),
    ("model.unreadable", E),
    ("model.not_step", E),
    ("model.placement_approximated", A),
    // Library runs (`signex-altium-import library`).
    ("table.disabled", I),
    ("table.missing", E),
    ("table.no_key", E),
    ("table.sim_skipped", I),
    ("table.reserved_unknown", I),
    ("table.lossy_text", I),
    ("library.search_truncated", I),
    ("row.key_missing", E),
    ("row.symbol_missing", E),
    ("row.symbol_library_missing", E),
    ("row.symbol_library_unreadable", E),
    ("row.footprint_library_unreadable", E),
    ("row.footprint_missing", E),
    ("row.footprint_library_missing", E),
    ("library.input_unreadable", E),
    ("library.crossref_mismatch", I),
    ("library.kept_referenced", I),
    ("library.kept_edited", I),
];

/// The severity of a catalogued code. An uncatalogued code is a bug in the
/// caller; it reads as `approximated` in release builds.
pub fn severity(code: &str) -> Severity {
    match CODES.iter().find(|(c, _)| *c == code) {
        Some((_, s)) => *s,
        None => {
            debug_assert!(false, "report code {code} is not in the catalogue");
            A
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_well_formed() {
        for (i, (c, _)) in CODES.iter().enumerate() {
            assert!(c.contains('.'), "{c}");
            assert!(
                c.bytes().all(|b| b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || b == b'_'
                    || b == b'.'),
                "{c}"
            );
            assert!(CODES[..i].iter().all(|(o, _)| o != c), "duplicate {c}");
        }
    }
}
