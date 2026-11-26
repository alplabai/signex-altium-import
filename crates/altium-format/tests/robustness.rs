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

mod intlib {
    use altium_format::intlib::{IntLib, inflate_embedded};

    #[test]
    fn embedded_streams_must_be_type_2_zlib() {
        assert!(inflate_embedded("s", &[]).is_err());
        assert!(inflate_embedded("s", &[1, 2, 3]).is_err());
        assert!(inflate_embedded("s", &[2, 0xde, 0xad, 0xbe, 0xef]).is_err());
        assert!(inflate_embedded("s", &[2]).is_err());
    }

    #[test]
    fn garbage_and_truncated_files_error_without_panicking() {
        assert!(IntLib::from_bytes(Vec::new()).is_err());
        assert!(IntLib::from_bytes(vec![0xd0, 0xcf, 0x11, 0xe0, 1, 2, 3]).is_err());
        assert!(IntLib::from_bytes((0..5000u32).map(|i| (i * 7) as u8).collect()).is_err());
    }
}

mod intlib_packed {
    use altium_format::intlib::{IntLib, pack};

    fn bytes(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/samples")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn a_packed_library_reads_back_both_halves() {
        let packed = pack(
            Some(&bytes("simple.SchLib")),
            Some(&bytes("Simple.PcbLib")),
            None,
        )
        .unwrap();
        let l = IntLib::from_bytes(packed).unwrap();
        assert_eq!(l.schlibs.len(), 1);
        assert_eq!(l.pcblibs.len(), 1);
        assert_eq!(l.schlibs[0].lib.components.len(), 10);
        assert!(!l.pcblibs[0].lib.footprints.is_empty());
    }

    #[test]
    fn a_file_without_embedded_libraries_is_an_error() {
        assert!(IntLib::from_bytes(pack(None, None, None).unwrap()).is_err());
    }

    #[test]
    fn a_non_zlib_or_truncated_embedded_stream_is_an_error() {
        // A stream that is not zlib.
        let bad = pack(Some(b"not a compound file"), None, None).unwrap();
        // pack deflates, so the payload inflates but is not a compound file.
        assert!(IntLib::from_bytes(bad).is_err());
        // A truncated inner library.
        let whole = bytes("simple.SchLib");
        let cut = pack(Some(&whole[..whole.len() / 2]), None, None).unwrap();
        assert!(IntLib::from_bytes(cut).is_err());
    }

    #[test]
    fn cross_reference_mismatches_are_reported_not_fatal() {
        let sch = bytes("simple.SchLib");
        let ok = IntLib::from_bytes(pack(Some(&sch), None, None).unwrap()).unwrap();
        assert!(
            ok.cross_ref_mismatches().is_empty(),
            "no stream, nothing to check"
        );
        let wrong = pack(Some(&sch), None, Some(&3u32.to_le_bytes())).unwrap();
        let l = IntLib::from_bytes(wrong).unwrap();
        let m = l.cross_ref_mismatches();
        assert!(m.iter().any(|x| x.contains("entries")), "{m:?}");
        assert!(m.iter().any(|x| x.contains("not in the cross-reference")));
    }
}
