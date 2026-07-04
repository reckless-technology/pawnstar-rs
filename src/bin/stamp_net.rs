//! Prepend a self-describing NetHeader to a raw bullet NNUE .bin. (Port of `tools/stamp_net.cpp`.)
//!
//! The bullet trainer writes a headerless quantised net; this tool stamps it with the 32-byte
//! NetHeader (magic + architecture parameters from this build's nnue constants) so the engine can
//! reject an incompatible file instead of silently misreading it. Run it on the exported
//! quantised.bin before shipping a net.
//!
//! Usage: stamp_net <raw_in.bin> <stamped_out.bin>

use pawnstar::nnue::{expected_net_header, NET_FILE_BYTES};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: {} <raw_in.bin> <stamped_out.bin>", args[0]);
        std::process::exit(2);
    }
    let payload = match std::fs::read(&args[1]) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("stamp_net: cannot open '{}'", args[1]);
            std::process::exit(1);
        }
    };

    // The raw payload must match this build's architecture (bullet pads the trailing tensor < 64
    // bytes). An already-stamped net is (header + payload) sized and is rejected here too.
    if payload.len() < NET_FILE_BYTES || payload.len() >= NET_FILE_BYTES + 64 {
        eprintln!(
            "stamp_net: '{}' is {} bytes, expected {} (raw) — wrong architecture for this build?",
            args[1],
            payload.len(),
            NET_FILE_BYTES
        );
        std::process::exit(1);
    }

    let header = expected_net_header().to_bytes();
    let mut out = Vec::with_capacity(header.len() + payload.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&payload);
    if std::fs::write(&args[2], &out).is_err() {
        eprintln!("stamp_net: write to '{}' failed", args[2]);
        std::process::exit(1);
    }
    eprintln!(
        "stamp_net: wrote '{}' ({}-byte header + {}-byte payload)",
        args[2],
        header.len(),
        payload.len()
    );
}
