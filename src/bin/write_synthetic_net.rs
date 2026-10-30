//! Write the synthetic NNUE net this build's shape would use, and stop.
//! (The C++ equivalent is `test_nnue_incremental --write-synthetic-net <path>`.)
//!
//! This is what gives a shape nothing has been trained for a **node-count signature** before it
//! has a net. At such a shape the engine rightly refuses the shipped net, so `bench` cannot run,
//! and over random weights a depth-13 search barely prunes and does not finish in any useful time
//! — which left the shape with no node count at all, and so no way to check that two platforms,
//! or the three bit-exact ports, agree on it. It is how the shared shape was measured before v20
//! was trained for it, and it is the only node-count gate the retired v12 shape now has, v12's own
//! net having been removed in release 2.0.0.
//!
//! The net comes from a fixed seed through fixed integer arithmetic (`nnue::synthetic_net_image`),
//! so its bytes are identical on every platform and in every port, and
//! `PAWNSTAR_EVALFILE=<it> pawnstar` then searches a real net at a fixed depth. See
//! `nnue/README.md`, "The two network shapes", for the six positions and the expected results.
//!
//! Usage: write_synthetic_net <out.bin>

use pawnstar::nnue;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: {} <out.bin>", args[0]);
        std::process::exit(2);
    }
    let image = nnue::synthetic_net_image();
    if std::fs::write(&args[1], &image).is_err() {
        eprintln!("write_synthetic_net: cannot write '{}'", args[1]);
        std::process::exit(1);
    }
    println!(
        "wrote '{}': {} bytes — {} king buckets, {} output head(s), mirroring {}, format_version {}, map crc {:#018x}",
        args[1],
        image.len(),
        nnue::NUM_KING_BUCKETS,
        nnue::NUM_OUTPUT_BUCKETS,
        if nnue::IS_KING_MIRRORING_ENABLED { "on" } else { "off" },
        nnue::NET_FORMAT_VERSION,
        nnue::king_bucket_map_crc()
    );
}
