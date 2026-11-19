//! A self-check of a converted board: where its track ends meet pads once
//! Signex places the pads. A track usually ends on a pad of its own net and
//! never on another net's copper; many ends on other nets' pads mean the
//! pads were placed wrong. Only a pad with a net and copper on the track's
//! layer counts: a track on the bottom crossing under a top pad is not an
//! end on it.

use signex_types::pcb::{Footprint, PadType, PcbBoard, Point};

/// Counts of track ends by what they land on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EndCheck {
    pub ends: usize,
    pub on_own_net_pad: usize,
    pub on_other_net_pad: usize,
}

struct PlacedPad<'a> {
    center: Point,
    reach: f64,
    net: u32,
    /// `None` for a plated through hole, which reaches every copper layer.
    layers: Option<&'a [String]>,
}

impl PlacedPad<'_> {
    fn on_layer(&self, layer: &str) -> bool {
        self.layers
            .is_none_or(|layers| layers.iter().any(|l| l == layer))
    }
}

/// Where Signex puts a footprint-local point: x mirrored on the bottom
/// side, turned by the plain rotation matrix, moved to the footprint.
pub fn signex_world(fp: &Footprint, local: Point) -> Point {
    let p = if fp.layer == "BottomCopper" {
        Point::new(-local.x, local.y)
    } else {
        local
    };
    let (s, c) = fp.rotation.to_radians().sin_cos();
    Point::new(
        fp.position.x + p.x * c - p.y * s,
        fp.position.y + p.x * s + p.y * c,
    )
}

pub fn track_ends(board: &PcbBoard) -> EndCheck {
    let mut pads: Vec<PlacedPad<'_>> = board
        .footprints
        .iter()
        .flat_map(|fp| {
            fp.pads.iter().filter_map(move |pad| {
                let net = pad.net.as_ref()?.number;
                Some(PlacedPad {
                    center: signex_world(fp, pad.position),
                    reach: pad.size.x.abs().min(pad.size.y.abs()) / 2.0,
                    net,
                    layers: (pad.pad_type != PadType::Thru).then_some(pad.layers.as_slice()),
                })
            })
        })
        .filter(|p| p.net != 0)
        .collect();
    pads.sort_by(|a, b| a.center.x.total_cmp(&b.center.x));
    let widest = pads.iter().map(|p| p.reach).fold(0.0, f64::max);
    let mut check = EndCheck::default();
    let ends = board
        .segments
        .iter()
        .flat_map(|s| [(s.start, s.net, &s.layer), (s.end, s.net, &s.layer)]);
    for (at, net, layer) in ends {
        check.ends += 1;
        let from = pads.partition_point(|p| p.center.x < at.x - widest);
        let (mut own, mut other) = (false, false);
        for pad in pads[from..]
            .iter()
            .take_while(|p| p.center.x <= at.x + widest)
            .filter(|p| p.on_layer(layer))
            .filter(|p| (p.center.x - at.x).hypot(p.center.y - at.y) <= p.reach)
        {
            if pad.net == net {
                own = true;
            } else {
                other = true;
            }
        }
        if own {
            check.on_own_net_pad += 1;
        } else if other {
            check.on_other_net_pad += 1;
        }
    }
    check
}
