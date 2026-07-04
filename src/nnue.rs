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
use crate::position::Position;

pub const INPUT_SIZE: usize = 768; // Features per perspective per bucket: 2 colors * 6 types * 64 squares.
pub const NUM_KING_BUCKETS: usize = 8; // King-square weight banks (1 = single bank, i.e. no king buckets).
pub const FEATURE_ROWS: usize = INPUT_SIZE * NUM_KING_BUCKETS; // Feature-transformer input rows (bucket-major).
pub const HIDDEN_SIZE: usize = 1024; // Feature-transformer / accumulator width per perspective.

// Quantisation constants. These MUST match the bullet trainer configuration that produced the net.
pub const QA: i32 = 255; // Feature-transformer weight/activation scale (SCReLU clamp).
pub const QB: i32 = 64; // Output-layer weight scale.
pub const SCALE: i32 = 400; // Maps the raw network output to centipawns.

/// Total size of the quantised weight payload: every weight a little-endian int16, tightly packed.
pub const NET_FILE_BYTES: usize =
    (FEATURE_ROWS * HIDDEN_SIZE + HIDDEN_SIZE + 2 * HIDDEN_SIZE + 1) * 2;

/// Magic identifying a Pawnstar-stamped NNUE file ("PSN" + format version byte).
pub const NET_MAGIC: [u8; 4] = *b"PSN1";
pub const NET_FORMAT_VERSION: u16 = 1; // Header format version (bump on incompatible header changes).

/// Self-describing 32-byte header prepended to a shipped net so an incompatible file is rejected
/// rather than silently misread. The architecture fields must equal the engine's compile-time
/// constants; the loader checks every one. Raw (headerless) bullet output is rejected — stamp it
/// with stamp_net.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct NetHeader {
    pub magic: [u8; 4],      // NET_MAGIC.
    pub format_version: u16, // NET_FORMAT_VERSION.
    pub input_size: u16,     // INPUT_SIZE (features per perspective per bucket).
    pub king_buckets: u16,   // NUM_KING_BUCKETS.
    pub hidden_size: u16,    // HIDDEN_SIZE.
    pub qa: i16,             // QA.
    pub qb: i16,             // QB.
    pub scale: i16,          // SCALE.
                             // + 14 zero-padding bytes to a round 32 on disk.
}

/// On-disk header size.
pub const NET_HEADER_BYTES: usize = 32;

impl NetHeader {
    /// Serialize to the packed little-endian 32-byte on-disk layout.
    pub fn to_bytes(&self) -> [u8; NET_HEADER_BYTES] {
        let mut b = [0u8; NET_HEADER_BYTES];
        b[0..4].copy_from_slice(&self.magic);
        b[4..6].copy_from_slice(&self.format_version.to_le_bytes());
        b[6..8].copy_from_slice(&self.input_size.to_le_bytes());
        b[8..10].copy_from_slice(&self.king_buckets.to_le_bytes());
        b[10..12].copy_from_slice(&self.hidden_size.to_le_bytes());
        b[12..14].copy_from_slice(&self.qa.to_le_bytes());
        b[14..16].copy_from_slice(&self.qb.to_le_bytes());
        b[16..18].copy_from_slice(&self.scale.to_le_bytes());
        b
    }

    /// Parse from the packed little-endian 32-byte on-disk layout.
    pub fn from_bytes(b: &[u8]) -> NetHeader {
        NetHeader {
            magic: [b[0], b[1], b[2], b[3]],
            format_version: u16::from_le_bytes([b[4], b[5]]),
            input_size: u16::from_le_bytes([b[6], b[7]]),
            king_buckets: u16::from_le_bytes([b[8], b[9]]),
            hidden_size: u16::from_le_bytes([b[10], b[11]]),
            qa: i16::from_le_bytes([b[12], b[13]]),
            qb: i16::from_le_bytes([b[14], b[15]]),
            scale: i16::from_le_bytes([b[16], b[17]]),
        }
    }
}

/// The header describing the architecture this engine was compiled for.
pub fn expected_net_header() -> NetHeader {
    NetHeader {
        magic: NET_MAGIC,
        format_version: NET_FORMAT_VERSION,
        input_size: INPUT_SIZE as u16,
        king_buckets: NUM_KING_BUCKETS as u16,
        hidden_size: HIDDEN_SIZE as u16,
        qa: QA as i16,
        qb: QB as i16,
        scale: SCALE as i16,
    }
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
pub fn dequant(mut output: i64, bias: i16) -> i32 {
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
    output_bias: i16,    // Output bias, in QA*QB units.
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

    /// Load quantised weights from a bullet native .bin file. On failure logs to stderr and stays
    /// unloaded. Reads the file into memory and defers to load_from_memory.
    pub fn load(&mut self, path: &str) -> bool {
        self.loaded = false;
        let buffer = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => {
                eprintln!("info string NNUE: cannot open net file '{}'", path);
                return false;
            }
        };
        self.load_from_memory(&buffer, path)
    }

    /// Load quantised weights from an in-memory image of a net file, using the same header
    /// detection and architecture validation as load. Used for the engine's embedded fallback net.
    pub fn load_from_memory(&mut self, data: &[u8], origin: &str) -> bool {
        self.loaded = false;

        // Only a Pawnstar-stamped image is accepted: a 32-byte NetHeader (magic + architecture)
        // immediately followed by the payload. Every architecture field is checked against this
        // engine's constants, so an incompatible — or an unstamped/raw bullet — net is rejected
        // with a clear message instead of being silently misread.
        if data.len() < NET_HEADER_BYTES || data[0..4] != NET_MAGIC {
            eprintln!(
                "info string NNUE: net '{}' is not a stamped Pawnstar net (missing '{}' header) — stamp it with stamp_net",
                origin,
                std::str::from_utf8(&NET_MAGIC).unwrap()
            );
            return false;
        }
        let header = NetHeader::from_bytes(data);

        // Validate the stamped architecture fields against this build.
        let expected = expected_net_header();
        if header.format_version != expected.format_version
            || header.input_size != expected.input_size
            || header.king_buckets != expected.king_buckets
            || header.hidden_size != expected.hidden_size
            || header.qa != expected.qa
            || header.qb != expected.qb
            || header.scale != expected.scale
        {
            eprintln!(
                "info string NNUE: net '{}' is incompatible — file is v{}/in{}/buckets{}/hidden{}/qa{}/qb{}/scale{}, \
                 engine expects v{}/in{}/buckets{}/hidden{}/qa{}/qb{}/scale{}",
                origin,
                header.format_version,
                header.input_size,
                header.king_buckets,
                header.hidden_size,
                header.qa,
                header.qb,
                header.scale,
                expected.format_version,
                expected.input_size,
                expected.king_buckets,
                expected.hidden_size,
                expected.qa,
                expected.qb,
                expected.scale
            );
            return false;
        }
        let payload_offset = NET_HEADER_BYTES;

        // The packed payload (feature_weights, feature_bias, output_weights, output_bias) is
        // exactly NET_FILE_BYTES; the image may have a header before it and bullet padding after.
        if data.len() < payload_offset + NET_FILE_BYTES {
            eprintln!("info string NNUE: net '{}' truncated", origin);
            return false;
        }

        // Copy in bullet's save order: feature_weights, feature_bias, output_weights, output_bias.
        let mut p = payload_offset;
        let read_i16 = |bytes: &[u8], at: usize| i16::from_le_bytes([bytes[at], bytes[at + 1]]);
        for i in 0..FEATURE_ROWS * HIDDEN_SIZE {
            self.feature_weights.0[i] = read_i16(data, p + 2 * i);
        }
        p += FEATURE_ROWS * HIDDEN_SIZE * 2;
        for i in 0..HIDDEN_SIZE {
            self.feature_bias[i] = read_i16(data, p + 2 * i);
        }
        p += HIDDEN_SIZE * 2;
        for i in 0..2 * HIDDEN_SIZE {
            self.output_weights[i] = read_i16(data, p + 2 * i);
        }
        p += 2 * HIDDEN_SIZE * 2;
        self.output_bias = read_i16(data, p);

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
        true
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
