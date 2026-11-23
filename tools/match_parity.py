#!/usr/bin/env python3
"""Assert that a fixed-depth match between this engine and Pawnstar (C++) is game-for-game identical.

    match_parity.py PGN... [--candidate candidate] [--games N]

The two engines are bit-exact at a fixed depth on one thread, with the same net and the same hash size, so a match
between them from the same openings is deterministic, and the PGNs the shared match workflow's chunks leave
(`<tag>-chunk-N` artifacts, `chunk-N.pgn`) must show, for every opening, the same game twice:

  * fastchess plays each opening twice with the colours reversed (`-games 2 -repeat`: one `[Round]`). Since both
    engines choose the same move in the same position, the colour-reversed game is the same game move for move, and
    so the same result: the candidate scores 1.0 in one and 0.0 in the other, or 0.5 in both, and in every case
    the pair is exactly 50 %.
  * so per opening the two games must have the same moves, the same `[Result]` and `[Termination]`, and the same
    search report on every move (score and depth, nodes `n=` and seldepth `sd=`; the time and nps differ run to
    run and are left out). The nodes are the strongest part: a move can be the same while the tree that found
    it was not.

Any other outcome is a divergence: the first one in each failing opening is printed with its ply, the move and
search report each side gave, and the exit status is 1. A pair that is missing, a game count different from
`--games`, or a total score other than exactly half, also fails. Standard library only.
"""

import argparse
import re
import sys
from collections import defaultdict

TAG = re.compile(r'^\[(\w+)\s+"(.*)"\]\s*$')
TOKEN = re.compile(r"\{[^{}]*\}|[^\s{}]+")
MOVE_NUMBER = re.compile(r"^\d+\.(\.\.)?$")
RESULTS = {"1-0", "0-1", "1/2-1/2", "*"}
#: `{+0.79/10 0.182s, n=594048, sd=29, nps=1302736}`, or `{+M3/8 0.01s, n=...}`: what fastchess writes per move.
REPORT = re.compile(r"^\{\s*([^\s/]+)/(\d+)\s+[0-9.]+s,\s*n=(\d+),\s*sd=(\d+)")


class Game:
    def __init__(self, source, tags, plies):
        self.source = source
        self.tags = tags
        self.plies = plies  # [(move, report)] where report is (score, depth, nodes, seldepth) or the raw comment

    @property
    def key(self):
        return (self.source, self.tags.get("Round", "?"))


def parse_report(comment):
    found = REPORT.match(comment)
    return found.groups() if found else comment


def parse_games(path):
    """Yield the games of one PGN file: tags, then (move, report) per ply."""
    with open(path, encoding="utf-8") as handle:
        lines = handle.read().splitlines()
    games, tags, body, in_body = [], {}, [], False

    def finish():
        if tags or body:
            plies = []
            for token in TOKEN.findall(" ".join(body)):
                if token.startswith("{"):
                    if plies:
                        plies[-1] = (plies[-1][0], parse_report(token))
                elif not MOVE_NUMBER.match(token) and token not in RESULTS:
                    plies.append((token, None))
            games.append(Game(path, dict(tags), plies))

    for line in lines:
        found = TAG.match(line)
        if found:
            if in_body:
                finish()
                tags, body, in_body = {}, [], False
            tags[found.group(1)] = found.group(2)
        elif line.strip():
            in_body = True
            body.append(line)
    finish()
    return games


def first_difference(a, b):
    for ply, (left, right) in enumerate(zip(a.plies, b.plies), start=1):
        if left != right:
            return ply, left, right
    if len(a.plies) != len(b.plies):
        ply = min(len(a.plies), len(b.plies)) + 1
        return ply, (a.plies[ply - 1] if ply <= len(a.plies) else None), (b.plies[ply - 1] if ply <= len(b.plies) else None)
    return None


def candidate_points(game, candidate):
    result, white = game.tags.get("Result"), game.tags.get("White")
    if result == "1/2-1/2":
        return 0.5
    if result in ("1-0", "0-1"):
        return 1.0 if (result == "1-0") == (white == candidate) else 0.0
    return None


def check(games, candidate, expect_games):
    """Return (problems, pairs_checked). Each problem is a line of text."""
    problems = []
    if expect_games is not None and len(games) != expect_games:
        problems.append(f"{len(games)} games in the PGNs, {expect_games} expected")
    pairs = defaultdict(list)
    for game in games:
        pairs[game.key].append(game)
    points = 0.0
    for (source, round_name), pair in sorted(pairs.items()):
        label = f"{source.rsplit('/', 1)[-1]} round {round_name}"
        if len(pair) != 2:
            problems.append(f"{label}: {len(pair)} game(s), a colour-reversed pair is two")
            continue
        first, second = pair
        fen = first.tags.get("FEN")
        if fen != second.tags.get("FEN"):
            problems.append(f"{label}: the two games started from different positions")
        if first.tags.get("White") == second.tags.get("White"):
            problems.append(f"{label}: the colours were not reversed")
        difference = first_difference(first, second)
        if difference:
            ply, left, right = difference
            problems.append(f"{label} (opening {fen}): diverges at ply {ply}: "
                            f"{first.tags.get('White')}'s game has {left}, {second.tags.get('White')}'s has {right}")
        for name in ("Result", "Termination"):
            if first.tags.get(name) != second.tags.get(name):
                problems.append(f"{label}: {name} {first.tags.get(name)!r} against {second.tags.get(name)!r}")
        pair_points = [candidate_points(g, candidate) for g in pair]
        if None in pair_points or sum(pair_points) != 1.0:
            problems.append(f"{label}: the candidate scored {pair_points} over the pair, not exactly 1 of 2")
        else:
            points += sum(pair_points)
    if not problems and games and points * 2 != len(games):
        problems.append(f"the candidate scored {points} of {len(games)}, not exactly half")
    return problems, len(pairs)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("pgn", nargs="+")
    parser.add_argument("--candidate", default="candidate", help="the name fastchess gave the candidate")
    parser.add_argument("--games", type=int, default=None, help="the number of games the match was to play")
    args = parser.parse_args(argv)
    games = [game for path in sorted(args.pgn) for game in parse_games(path)]
    problems, pairs = check(games, args.candidate, args.games)
    moves = sum(len(g.plies) for g in games)
    print(f"{len(games)} games, {pairs} colour-reversed pairs, {moves} plies compared (move, score, depth, nodes, "
          "seldepth)")
    if problems:
        for problem in problems[:50]:
            print(f"::error::{problem}")
        print(f"PARITY FAILED: {len(problems)} problem(s)" + (" (first 50 shown)" if len(problems) > 50 else ""))
        return 1
    print(f"PARITY HOLDS: every pair of every opening is the same game move for move, node for node, and the "
          f"score is exactly 50% ({len(games) / 2:g} of {len(games)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
