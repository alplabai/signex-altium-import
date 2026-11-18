//! Keepout lines and arcs that close a loop, joined into the area they
//! fence. A Signex keepout is an area, so a closed loop of keepout lines
//! becomes a keepout zone; lines that do not close are reported.

use altium_format::pcb::{Common, Layer};
use signex_types::pcb::Point;

/// How close two ends must be to join, mm.
const JOIN_MM: f64 = 0.001;

/// One keepout line or arc, as a polyline in board coordinates.
pub(crate) struct KeepoutPiece {
    pub(crate) common: Common,
    pub(crate) points: Vec<Point>,
}

fn same(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() <= JOIN_MM && (a.y - b.y).abs() <= JOIN_MM
}

/// The closed loops the pieces make on each layer, each with the first
/// piece's attributes, and how many pieces belong to no closed loop.
pub(crate) fn close_loops(pieces: Vec<KeepoutPiece>) -> (Vec<(Common, Vec<Point>)>, usize) {
    let mut layers: Vec<Layer> = Vec::new();
    for piece in &pieces {
        if !layers.contains(&piece.common.layer) {
            layers.push(piece.common.layer);
        }
    }
    let mut loops = Vec::new();
    let mut open = 0;
    for layer in layers {
        let group: Vec<&KeepoutPiece> = pieces
            .iter()
            .filter(|p| p.common.layer == layer && p.points.len() >= 2)
            .collect();
        let mut taken = vec![false; group.len()];
        while let Some(first) = taken.iter().position(|t| !t) {
            taken[first] = true;
            let mut path = group[first].points.clone();
            let mut members = 1;
            while let (Some(&head), Some(&tail)) = (path.first(), path.last()) {
                if path.len() > 2 && same(head, tail) {
                    path.pop();
                    loops.push((group[first].common, path));
                    members = 0;
                    break;
                }
                let next = (0..group.len()).find(|&i| {
                    !taken[i]
                        && (same(group[i].points[0], tail)
                            || group[i].points.last().is_some_and(|&e| same(e, tail)))
                });
                let Some(i) = next else {
                    break;
                };
                taken[i] = true;
                members += 1;
                let mut points = group[i].points.clone();
                if !same(points[0], tail) {
                    points.reverse();
                }
                path.extend(points.into_iter().skip(1));
            }
            open += members;
        }
    }
    (loops, open)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(layer: Layer, points: &[(f64, f64)]) -> KeepoutPiece {
        KeepoutPiece {
            common: Common {
                layer,
                ..Default::default()
            },
            points: points.iter().map(|&(x, y)| Point::new(x, y)).collect(),
        }
    }

    #[test]
    fn four_lines_in_any_direction_close_one_loop() {
        let pieces = vec![
            piece(Layer::KEEP_OUT, &[(0.0, 0.0), (10.0, 0.0)]),
            piece(Layer::KEEP_OUT, &[(10.0, 5.0), (10.0, 0.0)]),
            piece(Layer::KEEP_OUT, &[(0.0, 5.0), (10.0, 5.0)]),
            piece(Layer::KEEP_OUT, &[(0.0, 5.0), (0.0, 0.0)]),
            piece(Layer::TOP, &[(20.0, 0.0), (30.0, 0.0)]),
        ];
        let (loops, open) = close_loops(pieces);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].0.layer, Layer::KEEP_OUT);
        assert_eq!(loops[0].1.len(), 4);
        assert_eq!(open, 1);
    }
}
