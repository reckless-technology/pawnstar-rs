//! Offline generator for the magic-bitboard multipliers baked into `generated_data.rs`
//! (BISHOP_MAGIC_NUMBERS / ROOK_MAGIC_NUMBERS). (Port of `tools/dump_magics.cpp`.)
//!
//! The engine itself does NOT search for magics at startup — it fills its attack tables from the
//! baked constants. This tool runs the one-off randomised search that found those constants and
//! prints them as ready-to-paste array initialisers. Regenerate (and paste the output back into
//! generated_data.rs) only if the occupancy-mask or attack generators change; the search is
//! deterministic (fixed-seed std::mt19937_64, reproduced exactly below), so an unchanged engine
//! reproduces byte-identical magics — which also makes this a cross-check that the ported
//! generators match the C++.

use pawnstar::generated_data::{
    bishop_attacks_gen, bishop_occupancy_mask, enumerate_mask_combinations, rook_attacks_gen,
    rook_occupancy_mask,
};

/// The C++ tool draws its candidates from `std::mt19937_64`; this is that generator (the standard
/// MT19937-64 of Matsumoto & Nishimura), reproduced so the fixed seed yields the identical magic
/// constants.
struct Mt19937_64 {
    mt: [u64; 312],
    mti: usize,
}

impl Mt19937_64 {
    fn new(seed: u64) -> Mt19937_64 {
        let mut mt = [0u64; 312];
        mt[0] = seed;
        for i in 1..312 {
            mt[i] = 6364136223846793005u64
                .wrapping_mul(mt[i - 1] ^ (mt[i - 1] >> 62))
                .wrapping_add(i as u64);
        }
        Mt19937_64 { mt, mti: 312 }
    }

    fn next(&mut self) -> u64 {
        const MATRIX_A: u64 = 0xB5026F5AA96619E9;
        const UM: u64 = 0xFFFFFFFF80000000; // most significant 33 bits
        const LM: u64 = 0x7FFFFFFF; // least significant 31 bits
        if self.mti >= 312 {
            for i in 0..312 {
                let x = (self.mt[i] & UM) | (self.mt[(i + 1) % 312] & LM);
                self.mt[i] =
                    self.mt[(i + 156) % 312] ^ (x >> 1) ^ if x & 1 != 0 { MATRIX_A } else { 0 };
            }
            self.mti = 0;
        }
        let mut x = self.mt[self.mti];
        self.mti += 1;
        x ^= (x >> 29) & 0x5555555555555555;
        x ^= (x << 17) & 0x71D67FFFEDA60000;
        x ^= (x << 37) & 0xFFF7EEE000000000;
        x ^= x >> 43;
        x
    }
}

/// A sparse magic candidate (AND of three draws) — sparse multipliers scatter occupancy bits into
/// the high word far more often than dense ones, so the search converges in a handful of attempts.
fn sparse_magic_candidate(rng: &mut Mt19937_64) -> u64 {
    rng.next() & rng.next() & rng.next()
}

/// Find a collision-free magic for one square by randomised search.
fn find_magic(
    sq: u8,
    mask_fn: fn(u8) -> u64,
    attack_fn: fn(u64, u8) -> u64,
    rng: &mut Mt19937_64,
) -> u64 {
    let mask = mask_fn(sq);
    let bits = mask.count_ones();
    let shift = 64 - bits;

    let occupancies = enumerate_mask_combinations(mask);
    let reference: Vec<u64> = occupancies.iter().map(|&occ| attack_fn(occ, sq)).collect();

    let mut table = vec![0u64; 1 << bits];
    let mut used = vec![0u64; 1 << bits]; // epoch stamp per slot (0 = never written)
    let mut epoch: u64 = 1;
    loop {
        let magic = sparse_magic_candidate(rng);
        if (mask.wrapping_mul(magic) & 0xFF00000000000000).count_ones() < 6 {
            continue; // cheap spread reject
        }
        let mut ok = true;
        for (i, &occ) in occupancies.iter().enumerate() {
            let idx = (occ.wrapping_mul(magic) >> shift) as usize;
            if used[idx] != epoch {
                used[idx] = epoch;
                table[idx] = reference[i];
            } else if table[idx] != reference[i] {
                ok = false;
                break;
            }
        }
        if ok {
            return magic;
        }
        epoch += 1;
    }
}

fn dump(name: &str, mask_fn: fn(u8) -> u64, attack_fn: fn(u64, u8) -> u64) {
    let mut rng = Mt19937_64::new(0xD1CE5EED5EED5EED); // fixed seed → deterministic, reproducible magics
    println!("pub const {}: [u64; 64] = [", name);
    let mut slots: usize = 0;
    for sq in 0u8..64 {
        let magic = find_magic(sq, mask_fn, attack_fn, &mut rng);
        slots += 1usize << mask_fn(sq).count_ones();
        if sq % 4 == 0 {
            print!("    ");
        }
        print!("0x{:016x},{}", magic, if sq % 4 == 3 { "\n" } else { " " });
    }
    println!("];");
    // `slots` is the total hash range; the engine compresses each slot to a 1-byte index into a
    // small de-duplicated attack table, so this is the size of the indices arrays.
    eprintln!(
        "{}: {} hash slots ({:.1} KB of 1-byte indices)",
        name,
        slots,
        slots as f64 / 1024.0
    );
}

fn main() {
    dump(
        "BISHOP_MAGIC_NUMBERS",
        bishop_occupancy_mask,
        bishop_attacks_gen,
    );
    dump("ROOK_MAGIC_NUMBERS", rook_occupancy_mask, rook_attacks_gen);
}
