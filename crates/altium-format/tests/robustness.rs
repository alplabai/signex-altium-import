//! Truncated or corrupted input must come back as an error, never a panic.

use std::collections::HashMap;

use altium_format::Container;
use altium_format::pcb::{Primitives, parse_primitives, text_records};
use altium_format::sch::{SchDoc, parse_symbol_stream};

fn sample(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/samples")
        .join(name)
}

/// Deterministic byte flips (xorshift), so a failure reproduces.
fn mutations(bytes: &[u8], rounds: usize) -> Vec<Vec<u8>> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..rounds)
        .map(|_| {
            let mut b = bytes.to_vec();
            for _ in 0..8 {
                if b.is_empty() {
                    break;
                }
                let i = (next() as usize) % b.len();
                b[i] ^= (next() & 0xff) as u8;
            }
            b
        })
        .collect()
}

fn variants(bytes: &[u8]) -> Vec<Vec<u8>> {
    let step = (bytes.len() / 64).max(1);
    let mut v: Vec<Vec<u8>> = (0..bytes.len())
        .step_by(step)
        .map(|n| bytes[..n].to_vec())
        .collect();
    v.extend(mutations(bytes, 64));
    v
}

#[test]
fn schematic_streams_never_panic() {
    let mut c = Container::open(sample("simple.SchDoc")).unwrap();
    let header = c.stream("FileHeader").unwrap();
    for v in variants(&header) {
        let _ = SchDoc::from_stream(&v);
    }
    let mut lib = Container::open(sample("simple.SchLib")).unwrap();
    let data = lib.stream("Position/Data").unwrap();
    for v in variants(&data) {
        let _ = parse_symbol_stream(&v);
    }
}

#[test]
fn pcb_streams_never_panic() {
    let mut c = Container::open(sample("simple.PcbDoc")).unwrap();
    for name in ["Pads6", "Tracks6", "Arcs6", "Vias6", "Texts6", "Regions6"] {
        let data = c.stream(&format!("{name}/Data")).unwrap();
        for v in variants(&data) {
            let mut out = Primitives::default();
            let _ = parse_primitives(&v, name, &HashMap::new(), &mut out);
        }
    }
    for name in ["Board6", "Components6", "Polygons6", "Nets6"] {
        let data = c.stream(&format!("{name}/Data")).unwrap();
        for v in variants(&data) {
            let _ = text_records(Some(&v), name);
        }
    }
}

#[test]
fn non_compound_input_is_an_error() {
    let dir = std::env::temp_dir().join(format!("altium-format-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("not-altium.SchDoc");
    std::fs::write(&p, b"definitely not an OLE file").unwrap();
    assert!(SchDoc::open(&p).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
