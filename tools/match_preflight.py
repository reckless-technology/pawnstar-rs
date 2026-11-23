#!/usr/bin/env python3
"""Start both sides of a hosted match the way fastchess will, and refuse a run whose nets did not load.

    match_preflight.py INPUTS [--depth N]

Run in every chunk by the shared match workflow's `preflight` input, in place of its generic probe. For each side
in INPUTS/manifest.json it starts the engine with the side's environment, sends the side's UCI options, searches
the start position and Kiwipete to a fixed depth, and requires a `bestmove`. It then
fails on any line the engine prints when a net was not what the match asked for:

    "keeping the net already loaded"   a refused `setoption name EvalFile`; the engine plays the OLD net
    "falling back to the embedded net" the file was not found at start-up; the engine plays its own net
    "FATAL" / "refused"                a net that failed a container check

The generic probe cannot see any of those: the engine still starts, still answers and still searches, with the
wrong net. Standard library only.
"""

import argparse
import json
import os
import re
import shlex
import subprocess
import sys

POSITIONS = ["position startpos",
             "position fen r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1"]
BAD_OUTPUT = re.compile(r"keeping the net already loaded|falling back to the embedded net|FATAL|refused|"
                        r"no NNUE net loaded", re.IGNORECASE)


def expand(text, inputs):
    return str(text).replace("{inputs}", os.path.abspath(inputs))


def run_side(role, side, each_options, inputs, depth):
    command = [expand(side["command"], inputs)] + [expand(a, inputs) for a in side.get("args", [])]
    environment = dict(os.environ)
    environment.update({key: expand(value, inputs) for key, value in side.get("env", {}).items()})
    options = {"Threads": "1", "Hash": "64"}
    options.update({k: expand(v, inputs) for k, v in each_options.items()})
    options.update({k: expand(v, inputs) for k, v in side.get("options", {}).items()})
    script = "uci\n" + "".join(f"setoption name {k} value {v}\n" for k, v in options.items()) + "isready\n"
    for position in POSITIONS:
        script += f"ucinewgame\nisready\n{position}\ngo depth {depth}\n"
    script += "quit\n"
    result = subprocess.run(command, input=script, capture_output=True, text=True, env=environment, timeout=300,
                            cwd=os.path.dirname(command[0]))
    text = result.stdout + result.stderr
    problems = []
    if text.count("bestmove") != len(POSITIONS):
        problems.append(f"{text.count('bestmove')} bestmove lines, {len(POSITIONS)} expected")
    if "uciok" not in text or text.count("readyok") != 1 + len(POSITIONS):
        problems.append("uciok/readyok not seen as expected")
    problems += [f"engine said: {line.strip()}" for line in text.splitlines() if BAD_OUTPUT.search(line)]
    if problems:
        print(f"::error::preflight: the {role} engine ({shlex.join(command)}) is not fit to play:\n  "
              + "\n  ".join(problems) + "\nits output:\n" + text)
        return False
    print(f"preflight: {role}: started with its net, searched {len(POSITIONS)} positions to depth {depth}, "
          "said nothing about a refused or substituted net: OK")
    return True


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("inputs")
    parser.add_argument("--depth", type=int, default=4)
    args = parser.parse_args(argv)
    with open(os.path.join(args.inputs, "manifest.json")) as handle:
        manifest = json.load(handle)
    results = [run_side(role, manifest[role], manifest.get("each_options", {}), args.inputs, args.depth)
               for role in ("candidate", "baseline")]
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
