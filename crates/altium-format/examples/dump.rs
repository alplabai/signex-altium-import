//! `cargo run -p altium-format --example dump -- <file> [stream]`
//!
//! Lists every stream in an Altium file, or prints one stream's records.
use altium_format::{Container, sch_stream};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: dump <file> [stream] [hex|sch|params]")?;
    let mut c = Container::open(&path)?;
    let Some(stream) = args.next() else {
        for s in c.all_streams() {
            let len = c.stream(&s).map(|b| b.len()).unwrap_or(0);
            println!("{len:>9}  {s}");
        }
        return Ok(());
    };
    let mode = args.next().unwrap_or_else(|| "sch".into());
    let bytes = c.stream(&stream)?;
    match mode.as_str() {
        "hex" => {
            for (i, chunk) in bytes.chunks(32).enumerate() {
                let hex: String = chunk.iter().map(|b| format!("{b:02x}")).collect();
                let asc: String = chunk
                    .iter()
                    .map(|&b| {
                        if (0x20..0x7f).contains(&b) {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                println!("{:06x}  {hex:<64}  {asc}", i * 32);
            }
        }
        "raw" => {
            use std::io::Write;
            std::io::stdout().write_all(&bytes)?;
        }
        "pcbtext" => {
            let mut r = altium_format::bin::Reader::new(&bytes, &stream);
            let mut i = 0;
            while r.remaining() >= 4 {
                let block = r.block("text record")?;
                let p = altium_format::Params::parse(block);
                let s: Vec<String> = p.iter().map(|(k, v)| format!("{k}={v}")).collect();
                println!("#{i} {}", s.join(" | "));
                i += 1;
            }
        }
        "pcbbin" => {
            let mut r = altium_format::bin::Reader::new(&bytes, &stream);
            while !r.is_empty() {
                let at = r.pos();
                let ty = r.u8("type")?;
                let block = r.block("block")?;
                let hex: String = block.iter().map(|b| format!("{b:02x}")).collect();
                println!("@{at:06x} t={ty} len={} {hex}", block.len());
            }
        }
        _ => {
            for (i, f) in sch_stream::frames(&bytes, &stream)?.into_iter().enumerate() {
                match f {
                    sch_stream::Frame::Text(p) => {
                        let s: Vec<String> = p.iter().map(|(k, v)| format!("{k}={v}")).collect();
                        println!("#{i} {}", s.join(" | "));
                    }
                    sch_stream::Frame::Binary(b) => {
                        let hex: String = b.iter().map(|b| format!("{b:02x}")).collect();
                        println!("#{i} BIN {hex}");
                    }
                }
            }
        }
    }
    Ok(())
}
