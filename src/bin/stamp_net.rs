//! Stamp a quantised NNUE net into the shared Reckless `RNNUE1` container.
//! (Port of `tools/stamp_net.cpp`.)
//!
//! The bullet trainer writes a headerless quantised net; this tool wraps it in the 64-byte
//! `RNNUE1` header — the architecture taken from *this build's* `nnue` constants, the king-bucket
//! map's checksum from this build's own map, and a CRC-64/XZ over the payload — so a file of
//! another shape, or of this shape with the buckets meaning something else, or one corrupted in
//! transit, is refused by the engine instead of being misread. A net stamped by a tool built at
//! another architecture carries that architecture's header and is then refused, which is the
//! point: the header, never the file name, says what a net is.
//!
//! Two input kinds are accepted, both written out identically:
//!
//! - a **raw trainer net**: the payload alone, whose output biases are int16 and which bullet pads
//!   to a 64-byte boundary. The padding is dropped and each bias widened to the container's int32.
//! - a **legacy `PSN1` net**: the same payload behind the 32-byte header this engine used before
//!   the container was shared. The legacy header is dropped and the payload re-stamped.
//!
//! Usage: stamp_net <in.bin> <stamped_out.bin>

use pawnstar::crc64;
use pawnstar::nnue::{
    expected_net_header, FEATURE_ROWS, HIDDEN_SIZE, NET_HEADER_BYTES, NET_PAYLOAD_BYTES,
    NET_TRAINER_PAYLOAD_BYTES, NUM_OUTPUT_BUCKETS,
};

/// The magic of the container this tool writes, and of a file already stamped with it.
const CONTAINER_MAGIC: &[u8] = b"RNNUE1\0\0";

/// The magic of the legacy per-engine header this container replaced, and its size.
const LEGACY_MAGIC: &[u8] = b"PSN1";
const LEGACY_HEADER_BYTES: usize = 32;

/// Everything in the payload before the output biases, identical in the trainer's layout and the
/// container's: the feature weights, the feature bias and the output weights, all int16.
const WEIGHT_BYTES: usize =
    FEATURE_ROWS * HIDDEN_SIZE * 2 + HIDDEN_SIZE * 2 + NUM_OUTPUT_BUCKETS * 2 * HIDDEN_SIZE * 2;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: {} <in.bin> <stamped_out.bin>", args[0]);
        std::process::exit(2);
    }
    let image = match std::fs::read(&args[1]) {
        Ok(image) => image,
        Err(_) => {
            eprintln!("stamp_net: cannot open '{}'", args[1]);
            std::process::exit(1);
        }
    };

    // Which kind of input this is, and where its payload starts.
    let trainer_payload = if image.starts_with(CONTAINER_MAGIC) {
        eprintln!(
            "stamp_net: '{}' is already an RNNUE1 net — stamp the trainer's output, not a stamped net",
            args[1]
        );
        std::process::exit(1);
    } else if image.starts_with(LEGACY_MAGIC) && image.len() >= LEGACY_HEADER_BYTES {
        eprintln!(
            "stamp_net: '{}' carries the legacy PSN1 header; re-stamping its payload into RNNUE1",
            args[1]
        );
        &image[LEGACY_HEADER_BYTES..]
    } else {
        &image[..]
    };

    // The payload must match this build's architecture. The trainer pads its trailing tensor to a
    // 64-byte boundary, so a few bytes beyond the weights are expected — and dropped, because the
    // container's payload is exactly NET_PAYLOAD_BYTES with no padding anywhere.
    if trainer_payload.len() < NET_TRAINER_PAYLOAD_BYTES
        || trainer_payload.len() >= NET_TRAINER_PAYLOAD_BYTES + 64
    {
        eprintln!(
            "stamp_net: '{}' holds a {}-byte payload, expected {} (plus up to 63 bytes of trainer padding) — wrong architecture for this build?",
            args[1],
            trainer_payload.len(),
            NET_TRAINER_PAYLOAD_BYTES
        );
        std::process::exit(1);
    }

    // The container's payload: the int16 weights verbatim, then each output bias widened from the
    // trainer's int16 to int32, sign extended.
    let mut payload = Vec::with_capacity(NET_PAYLOAD_BYTES);
    payload.extend_from_slice(&trainer_payload[..WEIGHT_BYTES]);
    for bucket in 0..NUM_OUTPUT_BUCKETS {
        let at = WEIGHT_BYTES + bucket * 2;
        let bias = i16::from_le_bytes([trainer_payload[at], trainer_payload[at + 1]]);
        payload.extend_from_slice(&(bias as i32).to_le_bytes());
    }
    assert_eq!(
        payload.len(),
        NET_PAYLOAD_BYTES,
        "the assembled payload is the size the container declares"
    );

    let mut header = expected_net_header();
    header.payload_crc = crc64::checksum(&payload);
    let header = header.with_header_crc();
    let header_bytes = header.to_bytes();

    // Distinguish open-for-write from write failure, like the C++ (ofstream open vs write).
    let mut out = match std::fs::File::create(&args[2]) {
        Ok(file) => file,
        Err(_) => {
            eprintln!("stamp_net: cannot write '{}'", args[2]);
            std::process::exit(1);
        }
    };
    use std::io::Write;
    if out
        .write_all(&header_bytes)
        .and_then(|()| out.write_all(&payload))
        .is_err()
    {
        eprintln!("stamp_net: write to '{}' failed", args[2]);
        std::process::exit(1);
    }
    eprintln!(
        "stamp_net: wrote '{}' ({}-byte RNNUE1 header + {}-byte payload)",
        args[2],
        NET_HEADER_BYTES,
        payload.len()
    );
    eprintln!(
        "stamp_net:   king_bucket_map_crc {:#018x}, payload_crc {:#018x}, header_crc {:#018x}",
        header.king_bucket_map_crc, header.payload_crc, header.header_crc
    );
}
