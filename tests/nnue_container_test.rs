//! The shared Reckless NNUE net container (`RNNUE1`): CRC-64/XZ, the 64-byte header's on-disk
//! layout, and the loader's two checks.
//!
//! The CRC's published check value is asserted **before anything else**, because every other
//! assertion here — and the integrity of every net file all five engines read — rests on this
//! engine computing the same checksum from the same bytes as the other four.
//!
//! The refusals are then walked field by field. Each must be loud (naming the field and both
//! values, which the loader prints to stderr), must leave a previously loaded net playing, and
//! must never be papered over with another net: a net that fails a check never plays a move, so
//! an engine that quietly substituted one would report a plausible Elo for a net that never ran.

mod common;

use pawnstar::crc64;
use pawnstar::nnue::{
    expected_net_header, king_bucket_map_crc, NetHeader, NetLoadOutcome, Network, HIDDEN_SIZE,
    INPUT_SIZE, KING_BUCKET_MAP, NET_ACTIVATION_SCRELU, NET_FLAGS, NET_FORMAT_VERSION,
    NET_HEADER_BYTES, NET_HEADER_CHECKSUMMED_BYTES, NET_MAGIC, NET_PAYLOAD_BYTES,
    NET_TRAINER_PAYLOAD_BYTES, NUM_KING_BUCKETS, NUM_OUTPUT_BUCKETS, QA, QB, SCALE,
};

/// The shipped net, which the NNUE suites skip when it is absent (the C++ exit-0 contract).
const NET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/nnue/pawnstar-v12.bin");

fn check(condition: bool, message: &str) {
    common::check(condition, message);
}

/// CRC-64/XZ against its published check value, and the properties the container relies on.
fn test_crc64() {
    check(
        crc64::checksum(b"123456789") == 0x995D_C9BB_DF19_39FA,
        "CRC-64/XZ of '123456789' is 0x995dc9bbdf1939fa, the algorithm's check value",
    );
    check(
        crc64::CHECK_VALUE == 0x995D_C9BB_DF19_39FA,
        "the published check value is the one the module names",
    );
    check(
        crc64::checksum(b"") == 0,
        "the checksum of no bytes is zero (init XOR final)",
    );
    check(
        crc64::checksum(b"123456789") != crc64::checksum(b"123456798"),
        "two orderings of the same bytes check differently",
    );
    // The payload's checksum is taken over bytes the stamping tool assembles in pieces, so the
    // piecewise form must agree with the whole-slice form exactly.
    check(
        crc64::checksum_of_parts(&[b"1234", b"5678", b"9"]) == crc64::checksum(b"123456789"),
        "checksumming in parts matches checksumming the whole",
    );
    check(
        crc64::finish(crc64::update(!0u64, b"123456789")) == crc64::checksum(b"123456789"),
        "update from the initial value then finish is the checksum",
    );
}

/// The architecture this engine is compiled for, and the header's on-disk layout, offset by offset
/// against the container specification.
fn test_header_layout() {
    let header = expected_net_header();
    let bytes = header.to_bytes();

    check(bytes.len() == NET_HEADER_BYTES, "the header is 64 bytes");
    check(
        NET_HEADER_CHECKSUMMED_BYTES == 56,
        "header_crc covers bytes 0..55",
    );
    check(&bytes[0..8] == b"RNNUE1\0\0", "offset 0: the magic");
    check(
        u16::from_le_bytes([bytes[8], bytes[9]]) == 1,
        "offset 8: format_version 1",
    );
    check(
        u16::from_le_bytes([bytes[10], bytes[11]]) == 64,
        "offset 10: header_bytes 64",
    );
    check(
        u16::from_le_bytes([bytes[12], bytes[13]]) == 768,
        "offset 12: features_per_bucket 768",
    );
    check(
        u16::from_le_bytes([bytes[14], bytes[15]]) == 8,
        "offset 14: king_buckets 8",
    );
    check(
        u16::from_le_bytes([bytes[16], bytes[17]]) == 1024,
        "offset 16: hidden_size 1024",
    );
    check(
        u16::from_le_bytes([bytes[18], bytes[19]]) == 1,
        "offset 18: output_buckets 1 (material buckets are a later change)",
    );
    check(
        bytes[20] == 0,
        "offset 20: flags 0 (king mirroring is a later change)",
    );
    check(bytes[21] == 0, "offset 21: activation 0, SCReLU");
    check(
        u16::from_le_bytes([bytes[22], bytes[23]]) == 255,
        "offset 22: qa 255",
    );
    check(
        u16::from_le_bytes([bytes[24], bytes[25]]) == 64,
        "offset 24: qb 64",
    );
    check(
        u16::from_le_bytes([bytes[26], bytes[27]]) == 400,
        "offset 26: eval_scale 400",
    );
    check(
        u32::from_le_bytes(bytes[28..32].try_into().unwrap()) == 0,
        "offset 28: reserved zero",
    );
    check(
        u64::from_le_bytes(bytes[32..40].try_into().unwrap()) == king_bucket_map_crc(),
        "offset 32: king_bucket_map_crc, this engine's own map",
    );
    check(
        u64::from_le_bytes(bytes[40..48].try_into().unwrap()) == NET_PAYLOAD_BYTES as u64,
        "offset 40: payload_bytes, the size this architecture implies",
    );

    check(
        NetHeader::from_bytes(&bytes) == header,
        "the layout round-trips through from_bytes",
    );

    // The header fields are the engine's own constants, not a second copy of them.
    check(
        header.magic == NET_MAGIC
            && header.format_version == NET_FORMAT_VERSION
            && header.features_per_bucket == INPUT_SIZE as u16
            && header.king_buckets == NUM_KING_BUCKETS as u16
            && header.hidden_size == HIDDEN_SIZE as u16
            && header.output_buckets == NUM_OUTPUT_BUCKETS as u16
            && header.flags == NET_FLAGS
            && header.activation == NET_ACTIVATION_SCRELU
            && header.qa == QA as u16
            && header.qb == QB as u16
            && header.eval_scale == SCALE as u16,
        "every header field is the engine's compile-time constant",
    );

    // 8 * 768 * 1024 int16 feature weights, 1024 int16 feature biases, 2 * 1024 int16 output
    // weights per output bucket, one int32 output bias per output bucket.
    check(
        NET_PAYLOAD_BYTES == 12_582_912 + 2_048 + 4_096 + 4,
        "the payload is 12,589,060 bytes at this shape",
    );
    check(
        NET_TRAINER_PAYLOAD_BYTES == NET_PAYLOAD_BYTES - 2,
        "the trainer writes the same weights with int16 output biases",
    );

    let stamped = header.with_header_crc();
    check(
        stamped.header_crc == crc64::checksum(&bytes[..NET_HEADER_CHECKSUMMED_BYTES]),
        "with_header_crc checksums the header's own first 56 bytes",
    );

    // The map's contents, not just its length, decide which weight bank a king square reads.
    let mut entries = [0u8; 64];
    for (square, &bucket) in KING_BUCKET_MAP.iter().enumerate() {
        entries[square] = bucket as u8;
    }
    check(
        king_bucket_map_crc() == crc64::checksum(&entries),
        "the map's checksum is taken over the 64 entries, one uint8 each",
    );
    let mut shuffled = entries;
    shuffled.swap(0, 7);
    check(
        crc64::checksum(&shuffled) != king_bucket_map_crc(),
        "a map of the same eight buckets in another order checksums differently",
    );
}

/// A valid image of the shipped net: its own bytes, read from disk.
fn shipped_net_image() -> Option<Vec<u8>> {
    std::fs::read(NET).ok()
}

/// The shipped net passes both checks, and its header says what this engine expects.
fn test_shipped_net(image: &[u8]) {
    check(
        image.len() == NET_HEADER_BYTES + NET_PAYLOAD_BYTES,
        "the shipped net is a 64-byte header and an exact payload, with no padding",
    );
    let header = NetHeader::from_bytes(image);
    let mut expected = expected_net_header();
    expected.payload_crc = header.payload_crc;
    expected.header_crc = header.header_crc;
    check(
        header == expected,
        "the shipped net's header is this engine's architecture",
    );
    check(
        header.header_crc == crc64::checksum(&image[..NET_HEADER_CHECKSUMMED_BYTES]),
        "the shipped net's header_crc covers its own first 56 bytes",
    );
    check(
        header.payload_crc == crc64::checksum(&image[NET_HEADER_BYTES..]),
        "the shipped net's payload_crc covers its payload",
    );

    let mut net = Network::default();
    check(
        net.load_from_memory(image, "shipped").is_loaded() && net.loaded,
        "the shipped net loads",
    );
}

/// Every refusal the two checks make, each one leaving the net that is already loaded playing.
fn test_refusals(image: &[u8]) {
    // A net that is already loaded, so each refusal can be checked not to disturb it.
    let mut net = Network::default();
    check(
        net.load_from_memory(image, "shipped").is_loaded(),
        "a good net is loaded before the refusals",
    );
    let start_position = pawnstar::position::Position::from_string(
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    );
    let evaluation_before = net.evaluate_position(&start_position);

    // Each case is (what is wrong, the image it produces).
    let header = NetHeader::from_bytes(image);
    let restamp = |mutate: &dyn Fn(&mut NetHeader)| {
        let mut changed = header;
        mutate(&mut changed);
        let mut changed_image = changed.with_header_crc().to_bytes().to_vec();
        changed_image.extend_from_slice(&image[NET_HEADER_BYTES..]);
        changed_image
    };

    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();

    // Check 1, the header.
    let mut wrong_magic = image.to_vec();
    wrong_magic[0] = b'Z';
    cases.push(("the magic", wrong_magic));
    cases.push(("a short image", image[..NET_HEADER_BYTES - 1].to_vec()));
    let mut corrupt_header = image.to_vec();
    corrupt_header[12] ^= 0x01; // features_per_bucket, without re-stamping header_crc
    cases.push(("header_crc", corrupt_header));
    cases.push((
        "format_version",
        restamp(&|h| h.format_version = NET_FORMAT_VERSION + 1),
    ));
    cases.push(("header_bytes", restamp(&|h| h.header_bytes = 32)));
    cases.push((
        "features_per_bucket",
        restamp(&|h| h.features_per_bucket = 704),
    ));
    cases.push(("king_buckets", restamp(&|h| h.king_buckets = 4)));
    cases.push(("hidden_size", restamp(&|h| h.hidden_size = 512)));
    cases.push(("output_buckets", restamp(&|h| h.output_buckets = 8)));
    cases.push((
        "flags (king mirroring asked for)",
        restamp(&|h| h.flags = 1),
    ));
    cases.push(("activation", restamp(&|h| h.activation = 1)));
    cases.push(("qa", restamp(&|h| h.qa = 181)));
    cases.push(("qb", restamp(&|h| h.qb = 128)));
    cases.push(("eval_scale", restamp(&|h| h.eval_scale = 410)));
    cases.push(("reserved", restamp(&|h| h.reserved = 1)));
    cases.push((
        "king_bucket_map_crc (the same eight buckets meaning other squares)",
        restamp(&|h| h.king_bucket_map_crc ^= 1),
    ));

    // Check 2, the payload.
    cases.push((
        "payload_bytes",
        restamp(&|h| h.payload_bytes = NET_PAYLOAD_BYTES as u64 - 2),
    ));
    let mut padded = image.to_vec();
    padded.extend_from_slice(b"bullet"); // what the trainer's own padding looks like
    cases.push(("trailing bytes after the payload", padded));
    cases.push(("a truncated payload", image[..image.len() - 1].to_vec()));
    let mut flipped_weight = image.to_vec();
    flipped_weight[NET_HEADER_BYTES + 5_000_000] ^= 0x01;
    cases.push(("payload_crc (one flipped bit)", flipped_weight));

    for (what, bad_image) in &cases {
        check(
            net.load_from_memory(bad_image, "bad") == NetLoadOutcome::Refused,
            &format!("a net refused on {what}"),
        );
        check(
            net.loaded && net.evaluate_position(&start_position) == evaluation_before,
            &format!("the net already loaded still plays after the refusal on {what}"),
        );
    }

    // A missing or unreadable file is not a refusal: nothing is known about its contents.
    check(
        net.load("no/such/net.bin") == NetLoadOutcome::FileUnavailable,
        "a missing file is unavailable, not refused",
    );
}

#[test]
fn nnue_container_suite() {
    test_crc64();
    test_header_layout();
    match shipped_net_image() {
        Some(image) => {
            test_shipped_net(&image);
            test_refusals(&image);
        }
        None => println!("[PASS] the shipped net is absent; its container checks are skipped"),
    }
    assert_eq!(
        common::summary("NNUE container: CRC-64/XZ, the RNNUE1 header, and the loader's refusals"),
        0,
        "nnue container suite reported failures"
    );
}
