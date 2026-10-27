//! CRC-64/XZ, the integrity check of the shared Reckless NNUE net container.
//!
//! Also called CRC-64/GO-ECMA: polynomial `0x42F0E1EBA9EA3693`, reflected (so the table is built
//! from `0xC96C5795D7870F42`), initial value and final XOR both `0xFFFFFFFFFFFFFFFF`, reflected
//! in and out. A 256-entry table built at compile time, no intrinsics and no dependency, so every
//! engine sharing the container computes the same value from the same bytes.
//!
//! The check value every implementation of this algorithm must reproduce — the checksum of the
//! nine ASCII bytes `123456789` — is [`CHECK_VALUE`], asserted by the NNUE container test suite
//! before anything else.

/// The reflected form of the CRC-64/XZ polynomial `0x42F0E1EBA9EA3693`, used to build the table.
const REFLECTED_POLYNOMIAL: u64 = 0xC96C_5795_D787_0F42;

/// The algorithm's initial register value.
const INITIAL_VALUE: u64 = 0xFFFF_FFFF_FFFF_FFFF;

/// The value the register is XORed with to produce the checksum.
const FINAL_EXCLUSIVE_OR: u64 = 0xFFFF_FFFF_FFFF_FFFF;

/// CRC-64/XZ of the nine ASCII bytes `123456789`, the algorithm's published check value.
pub const CHECK_VALUE: u64 = 0x995D_C9BB_DF19_39FA;

/// One table entry per possible byte: the register contribution of that byte, reflected.
const fn build_table() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut index = 0;
    while index < 256 {
        let mut entry = index as u64;
        let mut bit = 0;
        while bit < 8 {
            entry = if entry & 1 == 0 {
                entry >> 1
            } else {
                (entry >> 1) ^ REFLECTED_POLYNOMIAL
            };
            bit += 1;
        }
        table[index] = entry;
        index += 1;
    }
    table
}

/// The byte-at-a-time table, built at compile time.
static TABLE: [u64; 256] = build_table();

/// Fold `data` into a running register. Start from [`INITIAL_VALUE`] (what [`checksum`] does) and
/// finish with [`finish`]; this split lets a caller checksum bytes it never holds contiguously.
pub fn update(register: u64, data: &[u8]) -> u64 {
    let mut register = register;
    for &byte in data {
        register = TABLE[((register ^ byte as u64) & 0xFF) as usize] ^ (register >> 8);
    }
    register
}

/// The register a run of [`update`] calls that began at [`INITIAL_VALUE`] left, as a checksum.
pub fn finish(register: u64) -> u64 {
    register ^ FINAL_EXCLUSIVE_OR
}

/// CRC-64/XZ of `data`.
pub fn checksum(data: &[u8]) -> u64 {
    finish(update(INITIAL_VALUE, data))
}

/// CRC-64/XZ of several runs of bytes, as though they were one. The net container's payload
/// checksum is taken over the payload as a whole, which the stamping tool assembles in pieces.
pub fn checksum_of_parts(parts: &[&[u8]]) -> u64 {
    let mut register = INITIAL_VALUE;
    for part in parts {
        register = update(register, part);
    }
    finish(register)
}
