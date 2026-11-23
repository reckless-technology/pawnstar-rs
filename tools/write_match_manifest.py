#!/usr/bin/env python3
"""Write manifest.json for the shared match workflow (reckless-technology/pawnstar, .github/workflows/match-core.yml).

    write_match_manifest.py INPUTS --candidate-name NAME --baseline-name NAME [--note TEXT ...]

INPUTS is the directory the caller is about to upload as the match artifact. It must already hold:

    candidate/pawnstar, candidate.bin    the candidate engine and the net it plays
    baseline/pawnstar,  baseline.bin     the baseline engine and the net it plays
    openings.epd                         the opening set (the core verifies the SHA-256 written here)

Each side is given its net twice, as the EvalFile option and as PAWNSTAR_EVALFILE, which this engine, Pawnstar and
Pawnstar Go all read at start-up, so a binary that is slow to reach `setoption` still plays the right net. Both
sides are pinned to one search thread (PAWNSTAR_THREADS), and OwnBook is off for both because the book move is
weighted-random. Standard library only.
"""

import argparse
import hashlib
import json
import os
import sys


def side(role, name):
    net = "{inputs}/%s.bin" % role
    return {
        "name": name,
        "command": "{inputs}/%s/pawnstar" % role,
        "options": {"EvalFile": net},
        "env": {"PAWNSTAR_EVALFILE": net, "PAWNSTAR_THREADS": "1"},
    }


def build_manifest(openings_sha256, candidate_name, baseline_name, notes):
    return {
        "format": 1,
        "candidate": side("candidate", candidate_name),
        "baseline": side("baseline", baseline_name),
        "each_options": {"OwnBook": "false"},
        "openings": {"path": "openings.epd", "sha256": openings_sha256},
        "notes": list(notes),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("inputs")
    parser.add_argument("--candidate-name", required=True)
    parser.add_argument("--baseline-name", required=True)
    parser.add_argument("--note", action="append", default=[])
    args = parser.parse_args(argv)
    for relative in ("candidate/pawnstar", "candidate.bin", "baseline/pawnstar", "baseline.bin", "openings.epd"):
        if not os.path.isfile(os.path.join(args.inputs, relative)):
            print(f"error: {os.path.join(args.inputs, relative)} is missing", file=sys.stderr)
            return 1
    with open(os.path.join(args.inputs, "openings.epd"), "rb") as handle:
        digest = hashlib.sha256(handle.read()).hexdigest()
    manifest = build_manifest(digest, args.candidate_name, args.baseline_name, args.note)
    with open(os.path.join(args.inputs, "manifest.json"), "w") as handle:
        json.dump(manifest, handle, indent=2)
        handle.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
