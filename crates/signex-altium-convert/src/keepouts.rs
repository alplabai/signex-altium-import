//! Keepouts as Signex keepout zones (`fill_type = "keepout"`).
//!
//! A keepout fill or region keeps its whole area clear. A keepout track or
//! arc keeps copper clear only along the line itself, so each of its
//! segments becomes a thin keepout at the line's width: a closed loop of
//! keepout lines fences nothing inside it.

use std::f64::consts::{FRAC_PI_2, PI};

use altium_format::pcb::{Common, Layer};
use signex_types::pcb::{Point, Zone};
use uuid::Uuid;

use crate::pcb::LayerNames;
use crate::report::Report;

/// The `fill_type` Signex reads as a keepout zone.
pub(crate) const KEEPOUT_FILL: &str = "keepout";

/// Chords per round end of a keepout line.
const CAP_STEPS: usize = 8;

pub(crate) const KEEPOUT_LINE_NO_WIDTH: &str = "keepout line or arc with no width not imported";

pub(crate) const KEEPOUT_NON_COPPER: &str = "keepout on a non-copper layer not imported";

pub(crate) const FOOTPRINT_KEEPOUT: &str =
    "footprint keepout imported as a board keepout: it does not move with the footprint";

pub(crate) const KEEPOUT_RESTRICTIONS: &str =
    "keepout imported as a keepout of all copper: per-object keepout restrictions not kept";

/// A keepout area (fill or region) as keepout zones on the copper it keeps
/// clear.
pub(crate) fn keepout_area(
    c: &Common,
    outline: &[Point],
    names: &LayerNames,
    copper: &[String],
    report: &mut Report,
) -> Vec<Zone> {
    let Some(layers) = keepout_layers(c, names, copper, report) else {
        return Vec::new();
    };
    zones_on(&layers, outline)
}

/// A keepout track or arc (as a polyline) as one thin keepout per segment,
/// `width` wide, with round ends so the segments join.
pub(crate) fn keepout_line(
    c: &Common,
    points: &[Point],
    width: f64,
    names: &LayerNames,
    copper: &[String],
    report: &mut Report,
) -> Vec<Zone> {
    if points.len() < 2 || !width.is_finite() || width <= 0.0 {
        report.warn(KEEPOUT_LINE_NO_WIDTH);
        return Vec::new();
    }
    let Some(layers) = keepout_layers(c, names, copper, report) else {
        return Vec::new();
    };
    points
        .windows(2)
        .flat_map(|w| zones_on(&layers, &capsule(w[0], w[1], width / 2.0)))
        .collect()
}

/// A void in an internal plane's pour: a keepout on that plane only.
pub(crate) fn plane_void(layer: String, outline: Vec<Point>) -> Zone {
    keepout_zone(layer, outline)
}

/// The copper layers a keepout keeps clear: every copper layer for the
/// keep-out layer, the item's own layer for a keepout on copper, none
/// (reported) for any other layer. Also reports what Signex cannot carry.
fn keepout_layers(
    c: &Common,
    names: &LayerNames,
    copper: &[String],
    report: &mut Report,
) -> Option<Vec<String>> {
    let layers = if c.layer == Layer::KEEP_OUT {
        copper.to_vec()
    } else if c.layer.is_copper() {
        vec![names.of(c.layer)]
    } else {
        report.warn(KEEPOUT_NON_COPPER);
        return None;
    };
    if c.component.is_some() {
        report.warn(FOOTPRINT_KEEPOUT);
    }
    report.warn(KEEPOUT_RESTRICTIONS);
    Some(layers)
}

fn zones_on(layers: &[String], outline: &[Point]) -> Vec<Zone> {
    layers
        .iter()
        .map(|layer| keepout_zone(layer.clone(), outline.to_vec()))
        .collect()
}

fn keepout_zone(layer: String, outline: Vec<Point>) -> Zone {
    Zone {
        uuid: Uuid::new_v4(),
        net: 0,
        net_name: String::new(),
        layer,
        outline,
        holes: Vec::new(),
        priority: 0,
        fill_type: KEEPOUT_FILL.into(),
        thermal_relief: false,
        thermal_gap: 0.0,
        thermal_width: 0.0,
        clearance: 0.0,
        min_thickness: 0.0,
        unknown: Default::default(),
    }
}

/// The area a line of half-width `r` from `a` to `b` covers: a stadium
/// whose round ends are chords inside the true arc.
pub(crate) fn capsule(a: Point, b: Point, r: f64) -> Vec<Point> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let base = if dx.hypot(dy) > 1e-9 {
        dy.atan2(dx)
    } else {
        0.0
    };
    let mut pts = Vec::with_capacity(2 * (CAP_STEPS + 1));
    // The end around `b` turns through the forward direction, the end
    // around `a` through the backward one.
    for (centre, from) in [(b, base - FRAC_PI_2), (a, base + FRAC_PI_2)] {
        for i in 0..=CAP_STEPS {
            let t = from + PI * i as f64 / CAP_STEPS as f64;
            pts.push(Point::new(centre.x + r * t.cos(), centre.y + r * t.sin()));
        }
    }
    pts
}

#[cfg(test)]
mod tests {
    use altium_format::pcb::{self as apcb, PcbDoc};
    use altium_format::sch::Pt;
    use signex_sketch::geom::{Point2, point_in_polygon};

    use super::*;
    use crate::pcb::convert;

    fn stacked(stack: &[Layer]) -> PcbDoc {
        let mut doc = PcbDoc::default();
        doc.board.copper_stack = stack.iter().map(|l| (*l, String::new())).collect();
        doc
    }

    fn covers(zone: &Zone, p: Point) -> bool {
        let ring: Vec<Point2> = zone.outline.iter().map(|q| Point2::new(q.x, q.y)).collect();
        point_in_polygon(Point2::new(p.x, p.y), &ring)
    }

    /// A 100 x 100 mil square of keepout tracks 10 mil wide on `layer`.
    fn keepout_square(doc: &mut PcbDoc, layer: Layer, keepout: bool) {
        let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
        for i in 0..4 {
            let (a, b) = (corners[i], corners[(i + 1) % 4]);
            doc.primitives.tracks.push(apcb::Track {
                common: apcb::Common {
                    layer,
                    keepout,
                    ..Default::default()
                },
                a: Pt::new(a.0, a.1),
                b: Pt::new(b.0, b.1),
                width: 10.0,
            });
        }
    }

    #[test]
    fn a_closed_loop_of_keepout_lines_keeps_only_the_lines_clear() {
        let mut doc = stacked(&[Layer::TOP, Layer::BOTTOM]);
        keepout_square(&mut doc, Layer::KEEP_OUT, false);
        let mut report = Report::default();
        let board = convert(&doc, &mut report).expect("converts");
        let keepouts: Vec<&Zone> = board
            .zones
            .iter()
            .filter(|z| z.fill_type == KEEPOUT_FILL)
            .collect();
        // Four segments on each of the two copper layers.
        assert_eq!(keepouts.len(), 8);
        // The board frame puts the square's corners at (0, 0) and
        // (2.54, 2.54) mm; its middle is free, its edge is not.
        let middle = Point::new(1.27, 1.27);
        assert!(
            keepouts.iter().all(|z| !covers(z, middle)),
            "the loop's interior is not a keepout"
        );
        let on_edge = Point::new(1.27, 0.0);
        for layer in ["TopCopper", "BottomCopper"] {
            assert!(
                keepouts
                    .iter()
                    .any(|z| z.layer == layer && covers(z, on_edge)),
                "{layer}: the line itself is a keepout"
            );
        }
        // Half the 10 mil width each side of the line, and no further.
        let inside_width = Point::new(1.27, 0.1);
        let past_width = Point::new(1.27, 0.2);
        assert!(keepouts.iter().any(|z| covers(z, inside_width)));
        assert!(keepouts.iter().all(|z| !covers(z, past_width)));
    }

    #[test]
    fn a_keepout_line_on_copper_keeps_its_own_layer_clear() {
        let mut doc = stacked(&[Layer::TOP, Layer::BOTTOM]);
        keepout_square(&mut doc, Layer::TOP, true);
        let board = convert(&doc, &mut Report::default()).expect("converts");
        let layers: Vec<&str> = board
            .zones
            .iter()
            .filter(|z| z.fill_type == KEEPOUT_FILL)
            .map(|z| z.layer.as_str())
            .collect();
        assert_eq!(layers, ["TopCopper"; 4]);
        assert!(board.segments.is_empty());
    }

    #[test]
    fn a_keepout_line_with_no_width_is_reported() {
        let mut doc = stacked(&[Layer::TOP, Layer::BOTTOM]);
        doc.primitives.tracks.push(apcb::Track {
            common: apcb::Common {
                layer: Layer::KEEP_OUT,
                ..Default::default()
            },
            a: Pt::new(0.0, 0.0),
            b: Pt::new(100.0, 0.0),
            width: 0.0,
        });
        let mut report = Report::default();
        let board = convert(&doc, &mut report).expect("converts");
        assert!(board.zones.is_empty());
        assert!(
            report
                .entries()
                .any(|(m, n)| m == KEEPOUT_LINE_NO_WIDTH && n == 1)
        );
    }

    #[test]
    fn a_footprint_keepout_and_its_restrictions_are_reported() {
        let mut doc = stacked(&[Layer::TOP, Layer::BOTTOM]);
        doc.components.push(apcb::Component::default());
        doc.primitives.regions.push(apcb::Region {
            common: apcb::Common {
                layer: Layer::TOP,
                keepout: true,
                component: Some(0),
                ..Default::default()
            },
            outline: vec![
                Pt::new(0.0, 0.0),
                Pt::new(100.0, 0.0),
                Pt::new(100.0, 100.0),
            ],
            ..Default::default()
        });
        let mut report = Report::default();
        let board = convert(&doc, &mut report).expect("converts");
        assert_eq!(
            board
                .zones
                .iter()
                .filter(|z| z.fill_type == KEEPOUT_FILL)
                .count(),
            1
        );
        let messages: Vec<&str> = report.entries().map(|(m, _)| m).collect();
        assert!(messages.contains(&FOOTPRINT_KEEPOUT));
        assert!(messages.contains(&KEEPOUT_RESTRICTIONS));
    }

    #[test]
    fn a_capsule_covers_its_line_and_round_ends() {
        let ring: Vec<Point2> = capsule(Point::new(0.0, 0.0), Point::new(10.0, 0.0), 1.0)
            .iter()
            .map(|p| Point2::new(p.x, p.y))
            .collect();
        assert_eq!(ring.len(), 2 * (CAP_STEPS + 1));
        for (x, y, inside) in [
            (5.0, 0.0, true),
            (5.0, 0.9, true),
            (5.0, 1.1, false),
            (10.9, 0.0, true),
            (-0.9, 0.0, true),
            (11.1, 0.0, false),
        ] {
            assert_eq!(
                point_in_polygon(Point2::new(x, y), &ring),
                inside,
                "({x}, {y})"
            );
        }
    }
}
