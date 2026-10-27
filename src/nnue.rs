//! NNUE (efficiently-updatable neural network) evaluation — the engine's only evaluator.
//! (Port of `nnue.h`.)
//!
//! The network is a "perspective" net with bullet's `ChessBuckets` feature set:
//!
//! ```text
//!   768 inputs per (perspective, king bucket)  ->  HIDDEN_SIZE feature transformer  (x2 perspectives)
//!   SCReLU activation, concat[own | opponent] = 2*HIDDEN_SIZE  ->  1 output (centipawns, stm relative)
//! ```
//!
//! The shipped net (**v12**) is **1024-wide with NUM_KING_BUCKETS = 8** — eight king-square weight
//! banks selected by king file-pair × board-half (`KING_BUCKET_MAP`). Each perspective selects one
//! bank by *its own* king's square (in that perspective's orientation), the feature row being
//! `bucket * 768 + chess768_index`, matching bullet's `ChessBuckets`.
//!
//! This port ships the scalar kernels, which the C++ documents (and tests) as bit-identical to its
//! AVX2/VNNI kernels; x86 SIMD is a later parity-locked perf phase.

use crate::constants::{Color, Piece, MAX_EVALUATION, RANK_FLIP};
use crate::crc64;
use crate::position::Position;

pub const INPUT_SIZE: usize = 768; // Features per perspective per bucket: 2 colors * 6 types * 64 squares.
pub const NUM_KING_BUCKETS: usize = 8; // King-square weight banks (1 = single bank, i.e. no king buckets).
pub const FEATURE_ROWS: usize = INPUT_SIZE * NUM_KING_BUCKETS; // Feature-transformer input rows (bucket-major).
pub const HIDDEN_SIZE: usize = 1024; // Feature-transformer / accumulator width per perspective.

// Quantisation constants. These MUST match the bullet trainer configuration that produced the net.
pub const QA: i32 = 255; // Feature-transformer weight/activation scale (SCReLU clamp).
pub const QB: i32 = 64; // Output-layer weight scale.
pub const SCALE: i32 = 400; // Maps the raw network output to centipawns.

/// Output-layer weight banks, selected by material count in the shared architecture. This
/// engine's network has exactly one, so every position reads the same output head; material
/// output buckets are a separate, strength-affecting change owed to all three ports at once.
pub const NUM_OUTPUT_BUCKETS: usize = 1;

/// Bit 0 of the net header's `flags` byte: a perspective whose king stands on files e-h has its
/// squares horizontally mirrored before indexing. Off in this engine (see `NET_FLAGS`).
pub const NET_FLAG_HORIZONTAL_KING_MIRRORING: u8 = 1 << 0;

/// The `flags` byte this engine is compiled for. Horizontal king mirroring is off: like the
/// material output buckets it is a separate, strength-affecting change owed to all three ports at
/// once, so a net that asks for it is refused rather than evaluated with the wrong indexing.
pub const NET_FLAGS: u8 = 0;

/// The net header's `activation` value for SCReLU, the only activation this engine implements.
pub const NET_ACTIVATION_SCRELU: u8 = 0;

/// Total size of the quantised weight payload this engine expects: the feature weights, the
/// feature bias and the output weights as little-endian int16, the output biases as little-endian
/// int32, tightly packed in that order with no padding anywhere.
pub const NET_PAYLOAD_BYTES: usize = FEATURE_ROWS * HIDDEN_SIZE * 2
    + HIDDEN_SIZE * 2
    + NUM_OUTPUT_BUCKETS * 2 * HIDDEN_SIZE * 2
    + NUM_OUTPUT_BUCKETS * 4;

/// Size of the same weights as the bullet trainer writes them, whose output biases are int16.
/// Only `stamp_net` reads a trainer file; it widens each bias to the container's int32.
pub const NET_TRAINER_PAYLOAD_BYTES: usize = FEATURE_ROWS * HIDDEN_SIZE * 2
    + HIDDEN_SIZE * 2
    + NUM_OUTPUT_BUCKETS * 2 * HIDDEN_SIZE * 2
    + NUM_OUTPUT_BUCKETS * 2;

/// Magic identifying a net in the shared Reckless NNUE container: "RNNUE1" and two zero bytes.
pub const NET_MAGIC: [u8; 8] = *b"RNNUE1\0\0";

/// Container format version. The map's *contents* are part of the architecture, so a change of
/// what a king bucket means bumps this even when the bucket count is unchanged.
pub const NET_FORMAT_VERSION: u16 = 1;

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

/// What a load attempt did, so a caller can tell a net that was *refused* — one that failed a
/// container check, and must never be played with — from a file that simply was not there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NetLoadOutcome {
    /// The net passed both checks and is the network's weights now.
    Loaded,
    /// The file could not be opened or read. Nothing is known about its contents.
    FileUnavailable,
    /// The file failed one of the container's checks. The reason is on stderr, the previously
    /// loaded network (if any) is untouched, and **no fallback net may be substituted**: a
    /// refused net must not be quietly replaced by another, or a match reports a plausible Elo
    /// for a net that never played a move.
    Refused,
}

impl NetLoadOutcome {
    /// Whether the net loaded.
    pub fn is_loaded(self) -> bool {
        self == NetLoadOutcome::Loaded
    }
}

/// Report a refused net loudly on stderr, naming the field that disagreed and both values, and
/// return the outcome that forbids any fallback. The values are pre-formatted so a count reads as
/// a count and a checksum as hexadecimal.
fn refuse_net(origin: &str, field: &str, file_value: &str, engine_value: &str) -> NetLoadOutcome {
    eprintln!(
        "info string NNUE: net '{}' refused: {} is {} in the file but {} in this engine",
        origin, field, file_value, engine_value
    );
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

/// King-square -> weight-bank map (bullet `ChessBuckets`). Indexed by a square in the
/// *perspective's own* orientation (white king square directly; black king square ^ RANK_FLIP),
/// so the same 64-entry map serves both perspectives. MUST be byte-identical to the array passed
/// to ChessBuckets::new in the bullet trainer.
#[rustfmt::skip]
pub const KING_BUCKET_MAP: [i32; 64] = [
    0, 0, 1, 1, 2, 2, 3, 3,
    0, 0, 1, 1, 2, 2, 3, 3,
    0, 0, 1, 1, 2, 2, 3, 3,
    0, 0, 1, 1, 2, 2, 3, 3,
    4, 4, 5, 5, 6, 6, 7, 7,
    4, 4, 5, 5, 6, 6, 7, 7,
    4, 4, 5, 5, 6, 6, 7, 7,
    4, 4, 5, 5, 6, 6, 7, 7,
];

/// Feature row for a piece in one perspective's bucket bank.
///   white perspective: bucket*768 + color*384 + (type-1)*64 + square
///   black perspective: bucket*768 + (1-color)*384 + (type-1)*64 + (square ^ RANK_FLIP)
#[inline(always)]
fn feature_row(bucket: i32, color: usize, piece: usize, sq: usize, black: bool) -> usize {
    let type_offset = (piece - Piece::Pawn.index()) * 64;
    let base = if black {
        (1 - color) * 384 + type_offset + (sq ^ RANK_FLIP as usize)
    } else {
        color * 384 + type_offset + sq
    };
    bucket as usize * INPUT_SIZE + base
}

/// Element offset into feature_weights of a piece's column in one perspective's bucket bank.
#[inline(always)]
fn row(bucket: i32, color: usize, piece: usize, sq: usize, black: bool) -> usize {
    feature_row(bucket, color, piece, sq, black) * HIDDEN_SIZE
}

/// A perspective's king bucket: index the map with that perspective's king square, oriented to it.
#[inline(always)]
pub fn white_bucket(p: &Position) -> i32 {
    KING_BUCKET_MAP[(p.pieces[Piece::King.index()] & p.colors[Color::White.index()])
        .lsb()
        .index()]
}

#[inline(always)]
pub fn black_bucket(p: &Position) -> i32 {
    KING_BUCKET_MAP[(p.pieces[Piece::King.index()] & p.colors[Color::Black.index()])
        .lsb()
        .index()
        ^ RANK_FLIP as usize]
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
    /// Output weights: first HIDDEN_SIZE weight the side-to-move accumulator, next the other.
    output_weights: [i16; 2 * HIDDEN_SIZE],
    /// Output weights requantised to int8 for the shipped int8 forward pass (Network::evaluate).
    /// w8 = round(output_weight / output_w_scale) clamped to [-127,127]; the scale keeps the
    /// largest output weight in int8 range (1 for nets whose |weight| <= 127, e.g. the shipped
    /// net). Populated by load so the int8 kernel needs no per-eval setup; the int16 reference
    /// (evaluate_exact) uses output_weights instead.
    output_weights_i8: [i8; 2 * HIDDEN_SIZE],
    output_w_scale: i32, // Divisor mapping the int16 output weights into int8 range.
    output_bias: i32,    // Output bias, in QA*QB units (the container's int32).
    pub loaded: bool,    // Whether load() has succeeded.
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
            output_weights: [0; 2 * HIDDEN_SIZE],
            output_weights_i8: [0; 2 * HIDDEN_SIZE],
            output_w_scale: 1,
            output_bias: 0,
            loaded: false,
        }
    }

    /// Load quantised weights from an `RNNUE1` net file. On any failure logs to stderr and leaves
    /// the network exactly as it was. Reads the file into memory and defers to load_from_memory.
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
            eprintln!(
                "info string NNUE: net '{}' refused: it is not an RNNUE1 net (the first {} bytes are not the magic 'RNNUE1\\0\\0') — stamp a trainer net with stamp_net",
                origin, NET_MAGIC.len()
            );
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
        for i in 0..FEATURE_ROWS * HIDDEN_SIZE {
            self.feature_weights.0[i] = read_i16(payload, p + 2 * i);
        }
        p += FEATURE_ROWS * HIDDEN_SIZE * 2;
        for i in 0..HIDDEN_SIZE {
            self.feature_bias[i] = read_i16(payload, p + 2 * i);
        }
        p += HIDDEN_SIZE * 2;
        for i in 0..2 * HIDDEN_SIZE {
            self.output_weights[i] = read_i16(payload, p + 2 * i);
        }
        p += 2 * HIDDEN_SIZE * 2;
        self.output_bias = read_i32(payload, p);

        // Requantise the output weights to int8 for the int8 forward pass. The scale keeps the
        // largest-magnitude output weight inside [-127, 127]; for nets whose weights already fit
        // (|w| <= 127, e.g. the shipped net) the scale is 1 and the int8 copy is lossless.
        let mut max_ow: i32 = 0;
        for &v in self.output_weights.iter() {
            max_ow = max_ow.max((v as i32).abs());
        }
        self.output_w_scale = 1.max((max_ow + 126) / 127);
        for i in 0..2 * HIDDEN_SIZE {
            // std::lround: round half away from zero.
            let q = lround(self.output_weights[i] as f64 / self.output_w_scale as f64);
            self.output_weights_i8[i] = q.clamp(-127, 127) as i8;
        }

        self.loaded = true;
        eprintln!("info string NNUE: loaded net '{}'", origin);
        NetLoadOutcome::Loaded
    }

    /// Rebuild a single perspective from scratch (bias + every piece) using its king bucket.
    fn refresh_side(
        &self,
        side: &mut [i16; HIDDEN_SIZE],
        position: &Position,
        bucket: i32,
        black: bool,
    ) {
        *side = self.feature_bias;
        for color in [Color::White, Color::Black] {
            let friendly = position.colors[color.index()];
            for piece in Piece::ALL {
                for s in position.pieces[piece.index()] & friendly {
                    let r = row(bucket, color.index(), piece.index(), s.index(), black);
                    add_column(side, self.column(r));
                }
            }
        }
    }

    /// Apply the piece-placement delta from `from` to `to` to a single perspective, within one
    /// (unchanged) king bucket.
    fn diff_side(
        &self,
        side: &mut [i16; HIDDEN_SIZE],
        from: &Position,
        to: &Position,
        bucket: i32,
        black: bool,
    ) {
        for color in [Color::White, Color::Black] {
            for piece in Piece::ALL {
                let from_bb = from.pieces[piece.index()] & from.colors[color.index()];
                let to_bb = to.pieces[piece.index()] & to.colors[color.index()];
                for s in from_bb & !to_bb {
                    let r = row(bucket, color.index(), piece.index(), s.index(), black);
                    sub_column(side, self.column(r));
                }
                for s in to_bb & !from_bb {
                    let r = row(bucket, color.index(), piece.index(), s.index(), black);
                    add_column(side, self.column(r));
                }
            }
        }
    }

    /// Rebuild both perspectives of `acc` from scratch for `position` (seed bias + every piece).
    pub fn refresh(&self, acc: &mut Accumulator, position: &Position) {
        // Each perspective uses its own king's bucket bank; seed bias and add every piece.
        self.refresh_side_white(acc, position);
        self.refresh_side_black(acc, position);
    }

    #[inline]
    fn refresh_side_white(&self, acc: &mut Accumulator, position: &Position) {
        self.refresh_side(&mut acc.white, position, white_bucket(position), false);
    }

    #[inline]
    fn refresh_side_black(&self, acc: &mut Accumulator, position: &Position) {
        self.refresh_side(&mut acc.black, position, black_bucket(position), true);
    }

    /// Incrementally move `acc` from matching `from` to matching `to` by applying only the
    /// feature deltas for pieces whose placement differs. Reversible (swap `from` / `to` to undo)
    /// and a no-op when placements are identical (e.g. a null move). Robust to captures/castling/
    /// EP/promotion.
    pub fn update(&self, acc: &mut Accumulator, from: &Position, to: &Position) {
        // A king move that crosses a bucket boundary changes that whole perspective's bank, so
        // refresh it from scratch; otherwise apply only the piece-placement delta within the
        // (unchanged) bank. At most one king moves per ply, so at most one perspective refreshes.
        let wf = white_bucket(from);
        let wt = white_bucket(to);
        let bf = black_bucket(from);
        let bt = black_bucket(to);

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
                    let rw = row(wt, color.index(), piece, s.index(), false);
                    sub_column(&mut acc.white, self.column(rw));
                    let rb = row(bt, color.index(), piece, s.index(), true);
                    sub_column(&mut acc.black, self.column(rb));
                }
                for s in to_bb & !from_bb {
                    let piece = to.squares[s.index()].index();
                    let rw = row(wt, color.index(), piece, s.index(), false);
                    add_column(&mut acc.white, self.column(rw));
                    let rb = row(bt, color.index(), piece, s.index(), true);
                    add_column(&mut acc.black, self.column(rb));
                }
            }
            return;
        }

        // Rare: one king crossed a bucket boundary. Refresh that perspective; diff the other.
        if wf != wt {
            self.refresh_side(&mut acc.white, to, wt, false);
        } else {
            self.diff_side(&mut acc.white, from, to, wt, false);
        }
        if bf != bt {
            self.refresh_side(&mut acc.black, to, bt, true);
        } else {
            self.diff_side(&mut acc.black, from, to, bt, true);
        }
    }

    /// Evaluate from a maintained accumulator using the **int8** output layer — the engine's
    /// shipped evaluator, accurate to a bounded ~26 cp of evaluate_exact. `side_to_move` selects
    /// which perspective feeds the first half of the output layer. Returns centipawns from the
    /// side-to-move's perspective.
    pub fn evaluate(&self, acc: &Accumulator, side_to_move: Color) -> i32 {
        // Scalar int8 kernel, bit-identical to the C++ AVX2/VNNI paths: clamp [0,QA], square,
        // round, >>S to uint8; dot with int8 weights, accumulate exactly.
        let white_to_move = side_to_move == Color::White;
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
        let w_stm: &[i8; HIDDEN_SIZE] =
            (&self.output_weights_i8[..HIDDEN_SIZE]).try_into().unwrap();
        let w_ntm: &[i8; HIDDEN_SIZE] =
            (&self.output_weights_i8[HIDDEN_SIZE..]).try_into().unwrap();
        let mut dot8: i32 = 0;
        for i in 0..HIDDEN_SIZE {
            dot8 += u8_act(stm_acc[i]) * w_stm[i] as i32;
            dot8 += u8_act(ntm_acc[i]) * w_ntm[i] as i32;
        }
        let output = dot8 as i64 * self.output_w_scale as i64 * (1i64 << INT8_SHIFT);
        dequant(output, self.output_bias)
    }

    /// Evaluate a position with a full accumulator refresh, int8 output layer (the engine eval).
    pub fn evaluate_position(&self, position: &Position) -> i32 {
        let mut acc = Accumulator::default();
        self.refresh(&mut acc, position);
        self.evaluate(&acc, position.color_to_move)
    }

    /// Exact int16 SCReLU forward pass — the **reference** evaluator that reproduces the
    /// trainer's eval (the NNUE test cross-checks it to bullet within ±2 cp) and that the int8
    /// evaluate approximates. Not used on the search hot path.
    pub fn evaluate_exact(&self, acc: &Accumulator, side_to_move: Color) -> i32 {
        let white_to_move = side_to_move == Color::White;
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

        let mut output: i64 = 0;
        for i in 0..HIDDEN_SIZE {
            output += (screlu(stm_acc[i]) * self.output_weights[i] as i32) as i64;
            output += (screlu(ntm_acc[i]) * self.output_weights[HIDDEN_SIZE + i] as i32) as i64;
        }
        dequant(output, self.output_bias)
    }

    /// Exact int16 evaluation with a full accumulator refresh (reference / diagnostics).
    pub fn evaluate_exact_position(&self, position: &Position) -> i32 {
        let mut acc = Accumulator::default();
        self.refresh(&mut acc, position);
        self.evaluate_exact(&acc, position.color_to_move)
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
