//! NNUE (efficiently-updatable neural network) evaluation — the engine's only evaluator.
//! (Port of `nnue.h`.)
//!
//! The network is a "perspective" net with bullet's `ChessBuckets` feature set:
//!
//! ```text
//!   768 inputs per (perspective, king bucket)  ->  HIDDEN_SIZE feature transformer  (x2 perspectives)
//!   SCReLU activation, concat[own | opponent] = 2*HIDDEN_SIZE  ->  NUM_OUTPUT_BUCKETS heads
//!                                                                  (centipawns, stm relative)
//! ```
//!
//! The shipped net (**v20**, release 2.0.0) is **1024-wide with NUM_KING_BUCKETS = 8 and eight
//! material output heads** — eight king-square weight banks selected by the king's *mirrored*
//! file × board-half (`KING_BUCKET_MAP`). Each perspective selects one bank by *its own* king's
//! square (in that perspective's orientation), the feature row being
//! `bucket * 768 + chess768_index`, matching bullet's `ChessBucketsMirrored`.
//!
//! Two parts of the shape are **compile-time knobs**, the Cargo features
//! `nnue-king-mirroring` and `nnue-output-buckets-8` (together, `nnue-shared-shape`), so that
//! either shape can be built from one tree. **Both are in `default` as of release 2.0.0**, that
//! being the shape the shipped net needs; dropping the defaults gives the retired v12 shape, which
//! has no net in the tree any more. Both are stamped into every net's header, so a net can only
//! load into a build of its own shape.
//!
//! This port ships the scalar kernels, which the C++ documents (and tests) as bit-identical to its
//! AVX2/VNNI kernels; x86 SIMD is a later parity-locked perf phase.

use crate::constants::{Color, Piece, MAX_EVALUATION, RANK_FLIP};
use crate::crc64;
use crate::position::Position;
use crate::uci_println;

pub const INPUT_SIZE: usize = 768; // Features per perspective per bucket: 2 colors * 6 types * 64 squares.
pub const HIDDEN_SIZE: usize = 1024; // Feature-transformer / accumulator width per perspective.

/// Whether a perspective whose own king stands on files e-h is horizontally mirrored
/// (`square ^ FILE_FLIP`) before indexing. Measured in zenith at **+14.3 Elo** at fixed depth, and
/// worth more than that number suggests: halving the king-placement space gives twice the
/// effective data per bucket at the same file size, so it is a data multiplier as much as an Elo
/// add. It only paid past ~1.4B training positions, which is why v12 does not have it.
pub const IS_KING_MIRRORING_ENABLED: bool = cfg!(feature = "nnue-king-mirroring");

/// Material output heads, selected by `output_bucket` from the piece count. Eight in the shared
/// architecture, measured in zenith at **+14.7 Elo** at fixed depth; one in v12's shape. A
/// compile-time constant because it sizes the payload and is stamped into the header, so a net and
/// a build can never disagree about it silently.
pub const NUM_OUTPUT_BUCKETS: usize = if cfg!(feature = "nnue-output-buckets-8") {
    8
} else {
    1
};

/// XOR mask that mirrors a square across the d/e file boundary (it flips the three file bits).
pub const FILE_FLIP: usize = 0x07;

/// King-square weight banks, **derived** from `KING_BUCKET_MAP` rather than written down twice:
/// the highest bank the map selects, plus one. Change the map and the count follows, so the two
/// cannot disagree.
pub const NUM_KING_BUCKETS: usize = max_king_bucket_index() + 1;

pub const FEATURE_ROWS: usize = INPUT_SIZE * NUM_KING_BUCKETS; // Feature-transformer input rows (bucket-major).

/// The highest weight-bank index `KING_BUCKET_MAP` selects, i.e. the bank count it needs, less one.
const fn max_king_bucket_index() -> usize {
    let mut highest = 0;
    let mut square = 0;
    while square < KING_BUCKET_MAP.len() {
        if KING_BUCKET_MAP[square] > highest {
            highest = KING_BUCKET_MAP[square];
        }
        square += 1;
    }
    highest as usize
}

// Quantisation constants. These MUST match the bullet trainer configuration that produced the net.
pub const QA: i32 = 255; // Feature-transformer weight/activation scale (SCReLU clamp).
pub const QB: i32 = 64; // Output-layer weight scale.
pub const SCALE: i32 = 400; // Maps the raw network output to centipawns.

/// Bit 0 of the net header's `flags` byte: a perspective whose king stands on files e-h has its
/// squares horizontally mirrored before indexing (see `NET_FLAGS`).
pub const NET_FLAG_HORIZONTAL_KING_MIRRORING: u8 = 1 << 0;

/// The `flags` byte this engine is compiled for, **derived from the shape**: bit 0 is set exactly
/// when king mirroring is compiled in, so a mirrored net cannot load into an unmirrored build or
/// the reverse — the two read entirely different feature rows for the same position.
pub const NET_FLAGS: u8 = if IS_KING_MIRRORING_ENABLED {
    NET_FLAG_HORIZONTAL_KING_MIRRORING
} else {
    0
};

/// The net header's `activation` value for SCReLU, the only activation this engine implements.
pub const NET_ACTIVATION_SCRELU: u8 = 0;

/// Element counts of the four payload tensors, in the trainer's save order. The payload sizes and
/// the weight arrays are all derived from these, so the layout is written down once.
pub const FEATURE_WEIGHT_COUNT: usize = FEATURE_ROWS * HIDDEN_SIZE;
pub const FEATURE_BIAS_COUNT: usize = HIDDEN_SIZE;
pub const OUTPUT_WEIGHT_COUNT: usize = NUM_OUTPUT_BUCKETS * 2 * HIDDEN_SIZE;
pub const OUTPUT_BIAS_COUNT: usize = NUM_OUTPUT_BUCKETS;

/// Total size of the quantised weight payload this engine expects: the feature weights, the
/// feature bias and the output weights as little-endian int16, the output biases as little-endian
/// int32, tightly packed in that order with no padding anywhere.
pub const NET_PAYLOAD_BYTES: usize =
    (FEATURE_WEIGHT_COUNT + FEATURE_BIAS_COUNT + OUTPUT_WEIGHT_COUNT) * 2 + OUTPUT_BIAS_COUNT * 4;

/// Size of the same weights as the bullet trainer writes them, whose output biases are int16.
/// Only `stamp_net` reads a trainer file; it widens each bias to the container's int32.
pub const NET_TRAINER_PAYLOAD_BYTES: usize =
    (FEATURE_WEIGHT_COUNT + FEATURE_BIAS_COUNT + OUTPUT_WEIGHT_COUNT) * 2 + OUTPUT_BIAS_COUNT * 2;

/// Magic identifying a net in the shared Reckless NNUE container: "RNNUE1" and two zero bytes.
pub const NET_MAGIC: [u8; 8] = *b"RNNUE1\0\0";

/// Container format version, **2 when mirroring is on and 1 when it is off** — derived from the
/// shape, never hardcoded, and derived from the same input in every port. That is not a quirk of
/// the switch: the map's *contents* are part of the architecture, and turning mirroring on changes
/// what a bucket *means* rather than only how many there are. v12's map pairs adjacent files
/// (`half * 4 + file / 2`); the mirrored map is per-file (`half * 4 + min(file, 7 - file)`)
/// because mirroring has already folded e-h onto d-a. By the container's own rule that is a format
/// change, so it bumps the version — which `king_bucket_map_crc` would catch anyway, but the
/// version says plainly that the two are different formats rather than two nets of one format.
///
/// Version 2 means mirroring on **and** eight output heads, together. A build that hardcoded 2
/// while it had mirroring but not the heads would stamp a version-2 net another engine's version-2
/// build read with a different number of heads; deriving the version from the mirroring switch —
/// the thing that redefines a bucket — means the version cannot disagree with the shape.
pub const NET_FORMAT_VERSION: u16 = if IS_KING_MIRRORING_ENABLED { 2 } else { 1 };

/// On-disk header size, and the offset the payload starts at.
pub const NET_HEADER_BYTES: usize = 64;

/// Bytes of the header the `header_crc` field covers: everything before it.
pub const NET_HEADER_CHECKSUMMED_BYTES: usize = 56;

/// The self-describing 64-byte header of the shared `RNNUE1` container, little-endian throughout.
/// Every architecture field must equal this engine's compile-time constants and
/// `king_bucket_map_crc` the checksum of this engine's own `KING_BUCKET_MAP`, so a net of another
/// shape — or of this shape with the buckets meaning something else — is refused rather than
/// silently misread. Raw (headerless) trainer output is refused too: stamp it with `stamp_net`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct NetHeader {
    pub magic: [u8; 8],           // NET_MAGIC, at offset 0.
    pub format_version: u16,      // NET_FORMAT_VERSION, at 8.
    pub header_bytes: u16,        // NET_HEADER_BYTES, at 10.
    pub features_per_bucket: u16, // INPUT_SIZE, at 12.
    pub king_buckets: u16,        // NUM_KING_BUCKETS, at 14.
    pub hidden_size: u16,         // HIDDEN_SIZE, at 16.
    pub output_buckets: u16,      // NUM_OUTPUT_BUCKETS, at 18.
    pub flags: u8,                // NET_FLAGS, at 20.
    pub activation: u8,           // NET_ACTIVATION_SCRELU, at 21.
    pub qa: u16,                  // QA, at 22.
    pub qb: u16,                  // QB, at 24.
    pub eval_scale: u16,          // SCALE, at 26.
    pub reserved: u32,            // Zero, at 28.
    pub king_bucket_map_crc: u64, // CRC-64/XZ of the 64 map entries, one uint8 each, at 32.
    pub payload_bytes: u64,       // NET_PAYLOAD_BYTES, at 40.
    pub payload_crc: u64,         // CRC-64/XZ over the payload, at 48.
    pub header_crc: u64,          // CRC-64/XZ over bytes 0..55, at 56.
}

impl NetHeader {
    /// Serialize to the packed little-endian 64-byte on-disk layout, every field exactly as it
    /// stands (`header_crc` included, so the layout round-trips; `with_header_crc` fills it).
    pub fn to_bytes(&self) -> [u8; NET_HEADER_BYTES] {
        let mut bytes = [0u8; NET_HEADER_BYTES];
        bytes[0..8].copy_from_slice(&self.magic);
        bytes[8..10].copy_from_slice(&self.format_version.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.header_bytes.to_le_bytes());
        bytes[12..14].copy_from_slice(&self.features_per_bucket.to_le_bytes());
        bytes[14..16].copy_from_slice(&self.king_buckets.to_le_bytes());
        bytes[16..18].copy_from_slice(&self.hidden_size.to_le_bytes());
        bytes[18..20].copy_from_slice(&self.output_buckets.to_le_bytes());
        bytes[20] = self.flags;
        bytes[21] = self.activation;
        bytes[22..24].copy_from_slice(&self.qa.to_le_bytes());
        bytes[24..26].copy_from_slice(&self.qb.to_le_bytes());
        bytes[26..28].copy_from_slice(&self.eval_scale.to_le_bytes());
        bytes[28..32].copy_from_slice(&self.reserved.to_le_bytes());
        bytes[32..40].copy_from_slice(&self.king_bucket_map_crc.to_le_bytes());
        bytes[40..48].copy_from_slice(&self.payload_bytes.to_le_bytes());
        bytes[48..56].copy_from_slice(&self.payload_crc.to_le_bytes());
        bytes[56..64].copy_from_slice(&self.header_crc.to_le_bytes());
        bytes
    }

    /// Parse from the packed little-endian 64-byte on-disk layout. `bytes` must be at least
    /// `NET_HEADER_BYTES` long, which the loader checks before calling this.
    pub fn from_bytes(bytes: &[u8]) -> NetHeader {
        let word = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let long = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        NetHeader {
            magic: bytes[0..8].try_into().unwrap(),
            format_version: word(8),
            header_bytes: word(10),
            features_per_bucket: word(12),
            king_buckets: word(14),
            hidden_size: word(16),
            output_buckets: word(18),
            flags: bytes[20],
            activation: bytes[21],
            qa: word(22),
            qb: word(24),
            eval_scale: word(26),
            reserved: u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
            king_bucket_map_crc: long(32),
            payload_bytes: long(40),
            payload_crc: long(48),
            header_crc: long(56),
        }
    }

    /// This header with `header_crc` set to the checksum of its own first 56 bytes — what
    /// `stamp_net` writes and what the loader recomputes to detect a corrupted header.
    pub fn with_header_crc(&self) -> NetHeader {
        let mut stamped = *self;
        stamped.header_crc = crc64::checksum(&self.to_bytes()[..NET_HEADER_CHECKSUMMED_BYTES]);
        stamped
    }
}

/// CRC-64/XZ of this engine's king-bucket map, one `uint8` per square, which the header carries so
/// that a net whose buckets *mean* something else is refused even though their count matches.
pub fn king_bucket_map_crc() -> u64 {
    let mut entries = [0u8; 64];
    for (square, &bucket) in KING_BUCKET_MAP.iter().enumerate() {
        entries[square] = bucket as u8;
    }
    crc64::checksum(&entries)
}

/// The header describing the architecture this engine was compiled for. `payload_crc` and
/// `header_crc` are left zero: they belong to one particular file, not to the architecture.
pub fn expected_net_header() -> NetHeader {
    NetHeader {
        magic: NET_MAGIC,
        format_version: NET_FORMAT_VERSION,
        header_bytes: NET_HEADER_BYTES as u16,
        features_per_bucket: INPUT_SIZE as u16,
        king_buckets: NUM_KING_BUCKETS as u16,
        hidden_size: HIDDEN_SIZE as u16,
        output_buckets: NUM_OUTPUT_BUCKETS as u16,
        flags: NET_FLAGS,
        activation: NET_ACTIVATION_SCRELU,
        qa: QA as u16,
        qb: QB as u16,
        eval_scale: SCALE as u16,
        reserved: 0,
        king_bucket_map_crc: king_bucket_map_crc(),
        payload_bytes: NET_PAYLOAD_BYTES as u64,
        payload_crc: 0,
        header_crc: 0,
    }
}

/// A complete `RNNUE1` image — the 64-byte header this build stamps, then the payload — at **this
/// build's shape**, with deterministic pseudo-random weights.
///
/// At a shape with no trained net in the tree — which, since release 2.0.0 removed v12's net, is
/// what the retired v12 shape always is — the shipped net is a different shape and is rightly
/// refused, so `bench` cannot run. This is what lets such a shape be tested and *measured* with no
/// net of its own, and it is how the shared shape was measured before v20 existed for it.
///
/// Two uses, and both need the bytes to be identical everywhere:
///
/// - `tests/nnue_incremental_test.rs` checks the incremental accumulator against a from-scratch
///   rebuild over it. Weight *quality* is irrelevant there — the property under test holds for any
///   weights at all — and writing the payload out by hand is a second opinion on the layout
///   `load_from_memory` reads.
/// - `write_synthetic_net` writes it to a file, so `PAWNSTAR_EVALFILE=<it> pawnstar` can search a
///   real net at a fixed depth. The node counts that come out are the new shape's **parity
///   signature**, which is what the three bit-exact ports compare instead of a bench signature
///   (`bench` cannot run at a shape whose net does not exist, and a depth-13 search over random
///   weights barely prunes and does not terminate in useful time).
///
/// The weights come from a hand-seeded xorshift64* through fixed integer arithmetic, so the bytes
/// are identical on every platform and in every port — the same net, byte for byte, as the C++
/// engine's `--write-synthetic-net` writes.
pub fn synthetic_net_image() -> Vec<u8> {
    let mut image = Vec::with_capacity(NET_HEADER_BYTES + NET_PAYLOAD_BYTES);
    image.resize(NET_HEADER_BYTES, 0);

    // xorshift64*, seeded by hand: the same net on every machine and every run, so a failure is
    // reproducible rather than something that happened once.
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next_value = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };

    // feature_weights, then feature_bias: small enough that an accumulator of up to 32 columns
    // stays well inside int16, so the synthetic net exercises the same arithmetic a trained one
    // does.
    for _ in 0..FEATURE_WEIGHT_COUNT + FEATURE_BIAS_COUNT {
        let value = (next_value() >> 56) as i32 - 128;
        image.extend_from_slice(&(value as i16).to_le_bytes());
    }
    // output_weights: inside int8 range so load's requantisation is near-lossless, which keeps the
    // incremental test about the accumulator and not about requantisation.
    for _ in 0..OUTPUT_WEIGHT_COUNT {
        let value = (next_value() >> 56) as i32 - 128 + 1;
        image.extend_from_slice(&(value as i16).to_le_bytes());
    }
    // output_bias: int32 per output head, in QA*QB units.
    for _ in 0..OUTPUT_BIAS_COUNT {
        let bias = (next_value() >> 50) as i32 - 4096;
        image.extend_from_slice(&bias.to_le_bytes());
    }
    assert_eq!(
        image.len(),
        NET_HEADER_BYTES + NET_PAYLOAD_BYTES,
        "the synthetic payload is the size the container declares"
    );

    let mut header = expected_net_header();
    header.payload_crc = crc64::checksum(&image[NET_HEADER_BYTES..]);
    let header = header.with_header_crc();
    image[..NET_HEADER_BYTES].copy_from_slice(&header.to_bytes());
    image
}

/// What a load attempt did, so a caller can tell a net that was *refused* — one that failed a
/// container check, and must never be played with — from a file that simply was not there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NetLoadOutcome {
    /// The net passed both checks and is the network's weights now.
    Loaded,
    /// The file could not be opened or read. Nothing is known about its contents.
    FileUnavailable,
    /// The file failed one of the container's checks. The reason is on **both** stdout (as an
    /// `info string`, for a GUI) and stderr (bare, for a match harness, which drops info lines),
    /// the previously loaded network (if any) is untouched, and **no fallback net may be
    /// substituted**: a refused net must not be quietly replaced by another, or a match reports a
    /// plausible Elo for a net that never played a move.
    Refused,
}

impl NetLoadOutcome {
    /// Whether the net loaded.
    pub fn is_loaded(self) -> bool {
        self == NetLoadOutcome::Loaded
    }
}

/// Print one line about a net on **both** streams, on purpose: `info string <message>` on stdout,
/// which is what a UCI GUI reads, and the bare line on stderr, which is what a match harness keeps
/// — harnesses routinely drop `info` lines. A refusal only a GUI can see is a refusal nobody reads
/// when it matters, which is during an unattended match: a harness that sets `EvalFile`, sees
/// `readyok` and runs a thousand games then reports a plausible Elo for a net that never played a
/// move. The C++ engine's `RefuseNet` writes to the two streams the same way.
pub fn print_net_message_on_both_streams(message: &str) {
    uci_println!("info string {}", message);
    eprintln!("{}", message);
}

/// Report a refused net loudly on both streams, naming the field that disagreed and both values,
/// and return the outcome that forbids any fallback. The values are pre-formatted so a count reads
/// as a count and a checksum as hexadecimal.
fn refuse_net(origin: &str, field: &str, file_value: &str, engine_value: &str) -> NetLoadOutcome {
    print_net_message_on_both_streams(&format!(
        "NNUE: net '{}' refused: {} is {} in the file but {} in this engine",
        origin, field, file_value, engine_value
    ));
    NetLoadOutcome::Refused
}

/// The two perspective accumulators (each the feature-transformer output, width HIDDEN_SIZE).
/// Maintained incrementally across make/undo so a node's evaluation does not rebuild it from
/// scratch.
#[derive(Clone, Copy)]
#[repr(align(64))]
pub struct Accumulator {
    pub white: [i16; HIDDEN_SIZE], // White-perspective accumulator.
    pub black: [i16; HIDDEN_SIZE], // Black-perspective accumulator.
}

impl Default for Accumulator {
    fn default() -> Accumulator {
        Accumulator {
            white: [0; HIDDEN_SIZE],
            black: [0; HIDDEN_SIZE],
        }
    }
}

/// Squared clipped ReLU: clamp to [0, QA] then square (mirrors bullet's `simple` activation).
/// `x` is an accumulator value (already in QA units); result is in QA*QA units.
#[inline(always)]
pub fn screlu(x: i16) -> i32 {
    let y: i32 = if x < 0 {
        0
    } else if x as i32 > QA {
        QA
    } else {
        x as i32
    };
    y * y
}

// The feature column add/sub maintain the int16 accumulator from the int16 weight columns.
// The wrapping 16-bit add matches the C++ (scalar and AVX2 alike) exactly.

/// Add a feature column to a single-perspective accumulator.
#[inline(always)]
fn add_column(acc: &mut [i16; HIDDEN_SIZE], column: &[i16; HIDDEN_SIZE]) {
    for i in 0..HIDDEN_SIZE {
        acc[i] = acc[i].wrapping_add(column[i]);
    }
}

/// Subtract a feature column from a single-perspective accumulator (the inverse of add_column).
#[inline(always)]
fn sub_column(acc: &mut [i16; HIDDEN_SIZE], column: &[i16; HIDDEN_SIZE]) {
    for i in 0..HIDDEN_SIZE {
        acc[i] = acc[i].wrapping_sub(column[i]);
    }
}

/// Right shift applied to the SCReLU activation (clamp(x,0,QA)^2, in [0, QA^2=65025]) to fit
/// uint8. S=9 maps the range onto [0,127], which keeps the SIMD kernels saturation-free and
/// bit-identical to this scalar path. The eval scale folds 2^INT8_SHIFT back out.
pub const INT8_SHIFT: i32 = 9;

/// v12's king-square -> weight-bank map (bullet `ChessBuckets`): four file-*pairs* by board-half,
/// the `0,0,1,1,2,2,3,3` pattern. Indexed by a square in the *perspective's own* orientation
/// (white king square directly; black king square ^ RANK_FLIP), so the same 64-entry map serves
/// both perspectives. MUST be byte-identical to the array passed to ChessBuckets::new in the
/// bullet trainer. It is not mirror-symmetric, so it has no 32-entry form at all.
#[rustfmt::skip]
pub const KING_BUCKET_MAP_UNMIRRORED: [i32; 64] = [
    0, 0, 1, 1, 2, 2, 3, 3,
    0, 0, 1, 1, 2, 2, 3, 3,
    0, 0, 1, 1, 2, 2, 3, 3,
    0, 0, 1, 1, 2, 2, 3, 3,
    4, 4, 5, 5, 6, 6, 7, 7,
    4, 4, 5, 5, 6, 6, 7, 7,
    4, 4, 5, 5, 6, 6, 7, 7,
    4, 4, 5, 5, 6, 6, 7, 7,
];

/// The mirrored shape's map over **half** a board: 32 entries indexed `rank * 4 + mirrored_file`,
/// where `mirrored_file = min(file, 7 - file)`, giving `half * 4 + mirrored_file` with
/// `half = rank / 4`.
///
/// **This is the one place the mirrored bucketing is written down**, and the engine's 64-entry map
/// is generated from it by `expand_mirrored_king_bucket_map` using bullet's own expansion
/// arithmetic. These are the same 32 numbers `ChessBucketsMirrored::new` is handed in the trainer
/// (`tools/bullet/pawnstar.rs` in the C++ repository, which is where the trainer lives), so
/// trainer and engine read one array rather than two 64-entry ones compared by eye — a map that
/// drifts trains one network and runs another, and `king_bucket_map_crc` catches that only after
/// the training run is already spent.
#[rustfmt::skip]
pub const KING_BUCKET_MAP_HALF: [i32; 32] = [
    0, 1, 2, 3,
    0, 1, 2, 3,
    0, 1, 2, 3,
    0, 1, 2, 3,
    4, 5, 6, 7,
    4, 5, 6, 7,
    4, 5, 6, 7,
    4, 5, 6, 7,
];

/// Expand the 32-entry half-board map to the 64-entry map the engine indexes, exactly as bullet's
/// `ChessBucketsMirrored::new` does:
/// `expanded[sq] = half[(sq / 8) * 4 + [0,1,2,3,3,2,1,0][sq % 8]]`, where that eight-entry table
/// is `min(file, 7 - file)`. The expansion folds the file, so the expanded map gives the same bank
/// for a square and its mirror — which is what makes it safe to index with the king's *unmirrored*
/// square, as bullet does.
pub const fn expand_mirrored_king_bucket_map() -> [i32; 64] {
    let mut expanded = [0i32; 64];
    let mut square = 0;
    while square < expanded.len() {
        let file = square % 8;
        let mirrored_file = if file < 4 { file } else { 7 - file };
        expanded[square] = KING_BUCKET_MAP_HALF[(square / 8) * 4 + mirrored_file];
        square += 1;
    }
    expanded
}

/// The map this build indexes. Mirroring halves the king-placement space, so the two shapes need
/// genuinely different maps: the mirrored one folds the file (four mirrored files x two halves),
/// while v12's pairs adjacent files (four file-pairs x two halves). Which one is compiled is
/// stamped into the header twice over — in `flags` and in `king_bucket_map_crc` — so a net trained
/// for one can never load into a build of the other.
pub const KING_BUCKET_MAP: [i32; 64] = if IS_KING_MIRRORING_ENABLED {
    expand_mirrored_king_bucket_map()
} else {
    KING_BUCKET_MAP_UNMIRRORED
};

/// Everything one perspective needs to turn a (colour, piece, square) into a feature row: which
/// weight bank it reads, and how a square and a colour are oriented into its own view.
///
/// **The whole point of gathering these three into one value is the refresh rule.** A perspective
/// can only be updated incrementally while all three stay put; if any changes, every feature row
/// it reads moves and the perspective must be rebuilt from scratch. Before mirroring there was one
/// way for that to happen — a king crossing a bucket boundary — and the code compared bucket
/// indices. Mirroring adds a second, and the two are not the same: on the mirrored map a king
/// stepping d1 -> e1 keeps bucket 3 (file 3 and file 4 both mirror to file 3) while `square_flip`
/// gains the file flip, so a comparison of buckets alone would diff a perspective whose every row
/// had just moved and be **silently** wrong — no crash, no assertion, just a wrong evaluation.
/// Comparing the whole `Perspective` cannot miss either case.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Perspective {
    /// First feature row of this perspective's bank: `bucket * INPUT_SIZE`.
    pub bank: usize,
    /// XOR applied to a square: RANK_FLIP for black's view, plus FILE_FLIP if mirrored.
    pub square_flip: usize,
    /// XOR applied to a piece's colour: 0 for white's view, 1 for black's.
    pub color_flip: usize,
}

/// One perspective's indexing state for `position`. The map is indexed by the king's square
/// oriented to this perspective but **not** mirrored, matching bullet: the expanded map already
/// folds the file, so a square and its mirror select the same bank.
#[inline(always)]
pub fn perspective_for(position: &Position, black: bool) -> Perspective {
    let color = if black { Color::Black } else { Color::White };
    let king = (position.pieces[Piece::King.index()] & position.colors[color.index()])
        .lsb()
        .index();
    let oriented = if black {
        king ^ RANK_FLIP as usize
    } else {
        king
    };
    // bullet's test is `ksq % 8 > 3` on the perspective's own king square. A file is unchanged by
    // the rank flip, so the raw square answers it for either perspective.
    let mirror = if IS_KING_MIRRORING_ENABLED && (king & FILE_FLIP) >= 4 {
        FILE_FLIP
    } else {
        0
    };
    Perspective {
        bank: KING_BUCKET_MAP[oriented] as usize * INPUT_SIZE,
        square_flip: (if black { RANK_FLIP as usize } else { 0 }) | mirror,
        color_flip: usize::from(black),
    }
}

/// Feature row for a piece in one perspective's bank:
/// `bank + (colour ^ colour_flip)*384 + (type - 1)*64 + (square ^ square_flip)`.
#[inline(always)]
fn feature_row(perspective: &Perspective, color: usize, piece: usize, sq: usize) -> usize {
    perspective.bank
        + (color ^ perspective.color_flip) * 384
        + (piece - Piece::Pawn.index()) * 64
        + (sq ^ perspective.square_flip)
}

/// Element offset into feature_weights of a piece's column in one perspective's bank.
#[inline(always)]
fn row(perspective: &Perspective, color: usize, piece: usize, sq: usize) -> usize {
    feature_row(perspective, color, piece, sq) * HIDDEN_SIZE
}

/// The output head `position` selects, matching bullet's `MaterialCount<N>` exactly: the divisor
/// is `ceil(32 / N)` and the bucket `(piece_count - 2) / divisor`, so at N = 8 it is
/// `(piece_count - 2) / 4`. At N = 1 the divisor is 32 and every position selects head 0.
#[inline(always)]
pub fn output_bucket(position: &Position) -> usize {
    const DIVISOR: i32 = 32_usize.div_ceil(NUM_OUTPUT_BUCKETS) as i32;
    ((position.occupied_squares().pop_count() - 2) / DIVISOR) as usize
}

/// Shared dequantisation tail: SCReLU leaves the dot in QA*QA*QB units; reduce one QA, add the
/// bias (QA*QB units), apply the eval scale, then remove the remaining QA*QB quantisation ->
/// centipawns. The result is clamped to ±MAX_EVALUATION: pathological max-material positions can
/// push the raw net output past int16, which would wrap when stored in the int16 eval cache and
/// could collide with the mate band. Clamping keeps |eval| below WIN_THRESHOLD so eval and mate
/// scores never mix.
#[inline]
pub fn dequant(mut output: i64, bias: i32) -> i32 {
    output /= QA as i64;
    output += bias as i64;
    output *= SCALE as i64;
    output /= (QA * QB) as i64;
    output.clamp(-(MAX_EVALUATION as i64), MAX_EVALUATION as i64) as i32
}

/// The feature-transformer weight bank. Kept as its own 64-byte-aligned POD so the whole 12.6 MB
/// array lives in one heap allocation (never on the stack).
#[repr(align(64))]
pub struct FeatureWeights(pub [i16; FEATURE_ROWS * HIDDEN_SIZE]);

/// A loaded, quantised NNUE network plus its accumulator/evaluation operations.
/// Owned by the Game; read-only after load() and shared by all Lazy SMP threads (only shared
/// methods are called concurrently). Default-constructed instances are unloaded (`loaded == false`).
pub struct Network {
    /// Feature-transformer weights, column-major by feature then hidden:
    /// index = feature * HIDDEN_SIZE + i. Feature rows are laid out bucket-major.
    feature_weights: Box<FeatureWeights>,
    feature_bias: [i16; HIDDEN_SIZE], // Feature-transformer biases (always int16).
    /// Output weights, head-major: each output head is a contiguous 2 * HIDDEN_SIZE run, the
    /// side-to-move accumulator's half first, then the other.
    output_weights: [i16; OUTPUT_WEIGHT_COUNT],
    /// Output weights requantised to int8 for the shipped int8 forward pass (Network::evaluate).
    /// w8 = round(output_weight / output_w_scale) clamped to [-127,127]; the scale keeps the
    /// largest output weight in int8 range (1 for nets whose |weight| <= 127, e.g. the shipped
    /// net). Populated by load so the int8 kernel needs no per-eval setup; the int16 reference
    /// (evaluate_exact) uses output_weights instead.
    output_weights_i8: [i8; OUTPUT_WEIGHT_COUNT],
    output_w_scale: i32, // Divisor mapping the int16 output weights into int8 range.
    output_bias: [i32; OUTPUT_BIAS_COUNT], // Output bias per head, in QA*QB units (the container's int32).
    pub loaded: bool,                      // Whether load() has succeeded.
    /// Where the net now loaded came from: the path it was read from, or `embedded` for the copy
    /// built into the binary. Empty while nothing has loaded. Remembered so a *refused* net can be
    /// turned down by name while naming the net that is still playing — "keeping the net already
    /// loaded ('x'); 'y' was refused" tells a match harness which weights its games were played
    /// on, which "the net already loaded stays in use" did not.
    loaded_net_origin: String,
}

impl Default for Network {
    fn default() -> Network {
        Network::new()
    }
}

impl Network {
    /// The feature column at element offset `r`, as a fixed-size array reference (so the
    /// accumulator add/sub loops vectorize without per-element bounds checks).
    #[inline(always)]
    fn column(&self, r: usize) -> &[i16; HIDDEN_SIZE] {
        (&self.feature_weights.0[r..r + HIDDEN_SIZE])
            .try_into()
            .unwrap()
    }

    /// Construct an unloaded network (weights zeroed; the C++ leaves them uninitialised).
    pub fn new() -> Network {
        // One heap allocation for the 12.6 MB FT bank, zero-initialised without a stack transit.
        // SAFETY: FeatureWeights is a repr(align(64)) POD array wrapper, for which all-zero bytes
        // are a valid value; alloc_zeroed with its exact Layout produces a properly aligned,
        // fully initialised instance.
        let feature_weights: Box<FeatureWeights> = unsafe {
            let layout = std::alloc::Layout::new::<FeatureWeights>();
            let ptr = std::alloc::alloc_zeroed(layout) as *mut FeatureWeights;
            assert!(!ptr.is_null(), "FeatureWeights allocation failed");
            Box::from_raw(ptr)
        };
        Network {
            feature_weights,
            feature_bias: [0; HIDDEN_SIZE],
            output_weights: [0; OUTPUT_WEIGHT_COUNT],
            output_weights_i8: [0; OUTPUT_WEIGHT_COUNT],
            output_w_scale: 1,
            output_bias: [0; OUTPUT_BIAS_COUNT],
            loaded: false,
            loaded_net_origin: String::new(),
        }
    }

    /// Where the net now loaded came from — the path it was read from, or `embedded` — or an empty
    /// string while nothing has loaded. A refused load leaves it naming the net still playing.
    pub fn loaded_net_origin(&self) -> &str {
        &self.loaded_net_origin
    }

    /// Load quantised weights from an `RNNUE1` net file. A refusal is reported on both stdout and
    /// stderr (see `print_net_message_on_both_streams`) and leaves the network exactly as it was,
    /// `loaded_net_origin` still naming the net that is playing. Reads the file into memory and
    /// defers to load_from_memory.
    pub fn load(&mut self, path: &str) -> NetLoadOutcome {
        // Distinguish open failure from a mid-read I/O error, like the C++ (ifstream open vs read).
        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => {
                eprintln!("info string NNUE: cannot open net file '{}'", path);
                return NetLoadOutcome::FileUnavailable;
            }
        };
        let mut buffer = Vec::new();
        use std::io::Read;
        if file.read_to_end(&mut buffer).is_err() {
            eprintln!("info string NNUE: error reading net file '{}'", path);
            return NetLoadOutcome::FileUnavailable;
        }
        self.load_from_memory(&buffer, path)
    }

    /// Load quantised weights from an in-memory image of an `RNNUE1` net file, making the two
    /// checks the container mandates, in order: the header (magic, its own checksum, every
    /// architecture field against this engine's compile-time constants, and the king-bucket map's
    /// checksum against this engine's own map), then the payload (its declared size against what
    /// remains of the image, then its checksum). Also the loader for the engine's embedded net.
    ///
    /// Nothing is written into the network until every check has passed, so a refused net leaves a
    /// previously loaded one untouched and playable rather than half-overwritten or unloaded.
    pub fn load_from_memory(&mut self, data: &[u8], origin: &str) -> NetLoadOutcome {
        // --- Check 1: the header. ---
        if data.len() < NET_HEADER_BYTES || data[0..8] != NET_MAGIC {
            print_net_message_on_both_streams(&format!(
                "NNUE: net '{}' refused: it is not an RNNUE1 net (the first {} bytes are not the magic 'RNNUE1\\0\\0') — stamp a trainer net with stamp_net",
                origin, NET_MAGIC.len()
            ));
            return NetLoadOutcome::Refused;
        }
        let header = NetHeader::from_bytes(data);

        let header_crc = crc64::checksum(&data[..NET_HEADER_CHECKSUMMED_BYTES]);
        if header.header_crc != header_crc {
            return refuse_net(
                origin,
                "header_crc",
                &format!("{:#018x}", header.header_crc),
                &format!("{:#018x}", header_crc),
            );
        }

        // Every architecture field against this engine's own compile-time constants.
        let expected = expected_net_header();
        let fields: [(&str, u64, u64); 11] = [
            (
                "format_version",
                header.format_version as u64,
                expected.format_version as u64,
            ),
            (
                "header_bytes",
                header.header_bytes as u64,
                expected.header_bytes as u64,
            ),
            (
                "features_per_bucket",
                header.features_per_bucket as u64,
                expected.features_per_bucket as u64,
            ),
            (
                "king_buckets",
                header.king_buckets as u64,
                expected.king_buckets as u64,
            ),
            (
                "hidden_size",
                header.hidden_size as u64,
                expected.hidden_size as u64,
            ),
            (
                "output_buckets",
                header.output_buckets as u64,
                expected.output_buckets as u64,
            ),
            ("flags", header.flags as u64, expected.flags as u64),
            (
                "activation",
                header.activation as u64,
                expected.activation as u64,
            ),
            ("qa", header.qa as u64, expected.qa as u64),
            ("qb", header.qb as u64, expected.qb as u64),
            (
                "eval_scale",
                header.eval_scale as u64,
                expected.eval_scale as u64,
            ),
        ];
        for (field, file_value, engine_value) in fields {
            if file_value != engine_value {
                return refuse_net(
                    origin,
                    field,
                    &file_value.to_string(),
                    &engine_value.to_string(),
                );
            }
        }
        if header.reserved != expected.reserved {
            return refuse_net(
                origin,
                "reserved",
                &header.reserved.to_string(),
                &expected.reserved.to_string(),
            );
        }

        // The map's contents are part of the architecture: the same number of king buckets
        // meaning different squares would load cleanly and read the wrong weight bank.
        if header.king_bucket_map_crc != expected.king_bucket_map_crc {
            return refuse_net(
                origin,
                "king_bucket_map_crc",
                &format!("{:#018x}", header.king_bucket_map_crc),
                &format!("{:#018x}", expected.king_bucket_map_crc),
            );
        }

        // --- Check 2: the payload. ---
        if header.payload_bytes != expected.payload_bytes {
            return refuse_net(
                origin,
                "payload_bytes",
                &header.payload_bytes.to_string(),
                &expected.payload_bytes.to_string(),
            );
        }
        let remaining_bytes = (data.len() - NET_HEADER_BYTES) as u64;
        if remaining_bytes != header.payload_bytes {
            return refuse_net(
                origin,
                "payload length after the header",
                &remaining_bytes.to_string(),
                &header.payload_bytes.to_string(),
            );
        }
        let payload = &data[NET_HEADER_BYTES..];
        let payload_crc = crc64::checksum(payload);
        if header.payload_crc != payload_crc {
            return refuse_net(
                origin,
                "payload_crc",
                &format!("{:#018x}", header.payload_crc),
                &format!("{:#018x}", payload_crc),
            );
        }

        // Both checks passed: copy in the trainer's save order — feature_weights, feature_bias,
        // output_weights (own perspective's half first), output_bias.
        let mut p = 0;
        let read_i16 = |bytes: &[u8], at: usize| i16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let read_i32 = |bytes: &[u8], at: usize| {
            i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        for i in 0..FEATURE_WEIGHT_COUNT {
            self.feature_weights.0[i] = read_i16(payload, p + 2 * i);
        }
        p += FEATURE_WEIGHT_COUNT * 2;
        for i in 0..FEATURE_BIAS_COUNT {
            self.feature_bias[i] = read_i16(payload, p + 2 * i);
        }
        p += FEATURE_BIAS_COUNT * 2;
        for i in 0..OUTPUT_WEIGHT_COUNT {
            self.output_weights[i] = read_i16(payload, p + 2 * i);
        }
        p += OUTPUT_WEIGHT_COUNT * 2;
        for i in 0..OUTPUT_BIAS_COUNT {
            self.output_bias[i] = read_i32(payload, p + 4 * i);
        }

        // Requantise the output weights to int8 for the int8 forward pass. The scale keeps the
        // largest-magnitude output weight inside [-127, 127]; for nets whose weights already fit
        // (|w| <= 127, e.g. the shipped net) the scale is 1 and the int8 copy is lossless.
        let mut max_ow: i32 = 0;
        for &v in self.output_weights.iter() {
            max_ow = max_ow.max((v as i32).abs());
        }
        self.output_w_scale = 1.max((max_ow + 126) / 127);
        for i in 0..OUTPUT_WEIGHT_COUNT {
            // std::lround: round half away from zero.
            let q = lround(self.output_weights[i] as f64 / self.output_w_scale as f64);
            self.output_weights_i8[i] = q.clamp(-127, 127) as i8;
        }

        self.loaded = true;
        self.loaded_net_origin = origin.to_string();
        // On both streams, as a refusal is: `info string` on stdout is the protocol message a UCI GUI
        // reads, and the bare line on stderr is what a match harness keeps. It used to be printed on
        // stderr alone, with the `info string` prefix on it, so no GUI ever saw which net was playing.
        print_net_message_on_both_streams(&format!("NNUE: loaded net '{}'", origin));
        NetLoadOutcome::Loaded
    }

    /// Rebuild a single perspective from scratch (bias + every piece) through its indexing.
    fn refresh_side(
        &self,
        side: &mut [i16; HIDDEN_SIZE],
        position: &Position,
        perspective: &Perspective,
    ) {
        *side = self.feature_bias;
        for color in [Color::White, Color::Black] {
            let friendly = position.colors[color.index()];
            for piece in Piece::ALL {
                for s in position.pieces[piece.index()] & friendly {
                    let r = row(perspective, color.index(), piece.index(), s.index());
                    add_column(side, self.column(r));
                }
            }
        }
    }

    /// Apply the piece-placement delta from `from` to `to` to a single perspective, whose indexing
    /// is unchanged across the move.
    fn diff_side(
        &self,
        side: &mut [i16; HIDDEN_SIZE],
        from: &Position,
        to: &Position,
        perspective: &Perspective,
    ) {
        for color in [Color::White, Color::Black] {
            for piece in Piece::ALL {
                let from_bb = from.pieces[piece.index()] & from.colors[color.index()];
                let to_bb = to.pieces[piece.index()] & to.colors[color.index()];
                for s in from_bb & !to_bb {
                    let r = row(perspective, color.index(), piece.index(), s.index());
                    sub_column(side, self.column(r));
                }
                for s in to_bb & !from_bb {
                    let r = row(perspective, color.index(), piece.index(), s.index());
                    add_column(side, self.column(r));
                }
            }
        }
    }

    /// Rebuild both perspectives of `acc` from scratch for `position` (seed bias + every piece).
    pub fn refresh(&self, acc: &mut Accumulator, position: &Position) {
        // Each perspective uses its own king's bucket bank; seed bias and add every piece.
        self.refresh_side(&mut acc.white, position, &perspective_for(position, false));
        self.refresh_side(&mut acc.black, position, &perspective_for(position, true));
    }

    /// Incrementally move `acc` from matching `from` to matching `to` by applying only the
    /// feature deltas for pieces whose placement differs. Reversible (swap `from` / `to` to undo)
    /// and a no-op when placements are identical (e.g. a null move). Robust to captures/castling/
    /// EP/promotion.
    pub fn update(&self, acc: &mut Accumulator, from: &Position, to: &Position) {
        // A king move that changes its perspective's bank or its mirroring moves every feature row
        // that perspective reads, so refresh it from scratch; otherwise apply only the
        // piece-placement delta within the unchanged indexing. At most one king moves per ply, so
        // at most one perspective refreshes.
        let wf = perspective_for(from, false);
        let wt = perspective_for(to, false);
        let bf = perspective_for(from, true);
        let bt = perspective_for(to, true);

        // Common case (~98%): neither king changed bucket. Find the changed squares directly from
        // the two colour bitboards (a move touches only 2-4 squares — quiet, capture, en passant,
        // castling and promotion are all handled uniformly) instead of scanning all 12 piece
        // types. For each square a colour vacated we subtract the piece that was there (read from
        // `from`); for each it now occupies we add the piece now there (read from `to`) — so a
        // promotion's pawn->queen and a capture's two pieces fall out for free. Both perspectives
        // are updated together. A null move (identical placements) does no work. This is
        // bit-identical to a per-piece-type diff: the same set of feature columns is added/
        // subtracted, and int16 accumulator add/sub is order-independent.
        if wf == wt && bf == bt {
            for color in [Color::White, Color::Black] {
                let from_bb = from.colors[color.index()];
                let to_bb = to.colors[color.index()];
                for s in from_bb & !to_bb {
                    let piece = from.squares[s.index()].index();
                    let rw = row(&wt, color.index(), piece, s.index());
                    sub_column(&mut acc.white, self.column(rw));
                    let rb = row(&bt, color.index(), piece, s.index());
                    sub_column(&mut acc.black, self.column(rb));
                }
                for s in to_bb & !from_bb {
                    let piece = to.squares[s.index()].index();
                    let rw = row(&wt, color.index(), piece, s.index());
                    add_column(&mut acc.white, self.column(rw));
                    let rb = row(&bt, color.index(), piece, s.index());
                    add_column(&mut acc.black, self.column(rb));
                }
            }
            return;
        }

        // Rare: one king changed its perspective's bank or its mirroring. Refresh that
        // perspective; diff the other. Both faults are caught by the one comparison — see
        // `Perspective`.
        if wf != wt {
            self.refresh_side(&mut acc.white, to, &wt);
        } else {
            self.diff_side(&mut acc.white, from, to, &wt);
        }
        if bf != bt {
            self.refresh_side(&mut acc.black, to, &bt);
        } else {
            self.diff_side(&mut acc.black, from, to, &bt);
        }
    }

    /// Evaluate from a maintained accumulator using the **int8** output layer — the engine's
    /// shipped evaluator, accurate to a bounded ~26 cp of evaluate_exact. `position` is the one
    /// `acc` was maintained for: it supplies the side to move, which selects which perspective
    /// feeds the first half of the output layer, and the piece count, which selects the output
    /// head — both are properties of the position, not of the accumulator, so passing the position
    /// is what stops the two being read from different places. Returns centipawns from the
    /// side-to-move's perspective.
    pub fn evaluate(&self, acc: &Accumulator, position: &Position) -> i32 {
        // Scalar int8 kernel, bit-identical to the C++ AVX2/VNNI paths: clamp [0,QA], square,
        // round, >>S to uint8; dot with int8 weights, accumulate exactly.
        let white_to_move = position.color_to_move == Color::White;
        let stm_acc = if white_to_move {
            &acc.white
        } else {
            &acc.black
        };
        let ntm_acc = if white_to_move {
            &acc.black
        } else {
            &acc.white
        };
        // The output head the piece count selects. The weights are head-major, so a head is a
        // contiguous 2 * HIDDEN_SIZE run and selecting one is an offset the kernel never sees.
        let bucket = output_bucket(position);
        let head = bucket * 2 * HIDDEN_SIZE;

        #[inline(always)]
        fn u8_act(x: i16) -> i32 {
            let c: i32 = if x < 0 {
                0
            } else if x as i32 > QA {
                QA
            } else {
                x as i32
            };
            let a = (c * c + (1 << (INT8_SHIFT - 1))) >> INT8_SHIFT;
            if a > 255 {
                255
            } else {
                a
            }
        }

        // Accumulate in i32: |dot8| <= 2 * HIDDEN_SIZE * 255 * 127 ≈ 66M << 2^31, so this is
        // bit-identical to the C++ scalar's int64 accumulation (and matches its AVX2 kernel,
        // which also accumulates in int32) while letting LLVM vectorize the loop.
        let w_stm: &[i8; HIDDEN_SIZE] = (&self.output_weights_i8[head..head + HIDDEN_SIZE])
            .try_into()
            .unwrap();
        let w_ntm: &[i8; HIDDEN_SIZE] = (&self.output_weights_i8
            [head + HIDDEN_SIZE..head + 2 * HIDDEN_SIZE])
            .try_into()
            .unwrap();
        let mut dot8: i32 = 0;
        for i in 0..HIDDEN_SIZE {
            dot8 += u8_act(stm_acc[i]) * w_stm[i] as i32;
            dot8 += u8_act(ntm_acc[i]) * w_ntm[i] as i32;
        }
        let output = dot8 as i64 * self.output_w_scale as i64 * (1i64 << INT8_SHIFT);
        dequant(output, self.output_bias[bucket])
    }

    /// Evaluate a position with a full accumulator refresh, int8 output layer (the engine eval).
    pub fn evaluate_position(&self, position: &Position) -> i32 {
        let mut acc = Accumulator::default();
        self.refresh(&mut acc, position);
        self.evaluate(&acc, position)
    }

    /// Exact int16 SCReLU forward pass — the **reference** evaluator that reproduces the
    /// trainer's eval (the NNUE test cross-checks it to bullet within ±2 cp) and that the int8
    /// evaluate approximates. Not used on the search hot path.
    pub fn evaluate_exact(&self, acc: &Accumulator, position: &Position) -> i32 {
        let white_to_move = position.color_to_move == Color::White;
        let stm_acc = if white_to_move {
            &acc.white
        } else {
            &acc.black
        };
        let ntm_acc = if white_to_move {
            &acc.black
        } else {
            &acc.white
        };
        let bucket = output_bucket(position);
        let head = bucket * 2 * HIDDEN_SIZE;

        let mut output: i64 = 0;
        for i in 0..HIDDEN_SIZE {
            output += (screlu(stm_acc[i]) * self.output_weights[head + i] as i32) as i64;
            output +=
                (screlu(ntm_acc[i]) * self.output_weights[head + HIDDEN_SIZE + i] as i32) as i64;
        }
        dequant(output, self.output_bias[bucket])
    }

    /// Exact int16 evaluation with a full accumulator refresh (reference / diagnostics).
    pub fn evaluate_exact_position(&self, position: &Position) -> i32 {
        let mut acc = Accumulator::default();
        self.refresh(&mut acc, position);
        self.evaluate_exact(&acc, position)
    }
}

/// `std::lround` semantics: round half away from zero.
#[inline]
fn lround(x: f64) -> i64 {
    if x >= 0.0 {
        (x + 0.5).floor() as i64
    } else {
        (x - 0.5).ceil() as i64
    }
}
