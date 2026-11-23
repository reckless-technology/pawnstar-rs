#!/usr/bin/env python3
"""Tests for tools/match_parity.py, on small PGNs in fastchess's shape. Standard library only, so they hold on every
CI platform: `python3 -m unittest discover -s test -p 'test_*.py'`."""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "tools"))
import match_parity  # noqa: E402

FEN = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"


def game(round_name, white, black, result, moves, fen=FEN):
    """moves: [(san, comment)]"""
    text = [f'[Round "{round_name}"]', f'[White "{white}"]', f'[Black "{black}"]', f'[Result "{result}"]',
            '[Termination "adjudication"]', f'[FEN "{fen}"]', '[SetUp "1"]', ""]
    body = []
    for index, (san, comment) in enumerate(moves):
        if index % 2 == 0:
            body.append(f"{index // 2 + 1}.")
        body.append(san)
        if comment:
            body.append("{" + comment + "}")
    text.append(" ".join(body) + f" {result}")
    return "\n".join(text) + "\n\n"


SAME = [("e4", "+0.30/8 0.011s, n=1000, sd=12, nps=90000"), ("e5", "-0.20/8 0.012s, n=2000, sd=11, nps=91000")]


def write(directory, name, *games):
    path = os.path.join(directory, name)
    with open(path, "w") as handle:
        handle.write("".join(games))
    return path


class ParityTest(unittest.TestCase):
    def run_check(self, *games, expect=None):
        with tempfile.TemporaryDirectory() as directory:
            path = write(directory, "chunk-0.pgn", *games)
            return match_parity.check(match_parity.parse_games(path), "candidate", expect)[0]

    def test_a_pair_of_identical_games_holds(self):
        self.assertEqual([], self.run_check(
            game(1, "candidate", "baseline", "1-0", SAME), game(1, "baseline", "candidate", "1-0", SAME), expect=2))

    def test_time_and_nps_are_not_compared(self):
        other = [(SAME[0][0], "+0.30/8 9.999s, n=1000, sd=12, nps=1"), SAME[1]]
        self.assertEqual([], self.run_check(
            game(1, "candidate", "baseline", "1/2-1/2", SAME), game(1, "baseline", "candidate", "1/2-1/2", other)))

    def test_a_different_move_is_reported_at_its_ply(self):
        other = [SAME[0], ("c5", SAME[1][1])]
        problems = self.run_check(
            game(1, "candidate", "baseline", "1-0", SAME), game(1, "baseline", "candidate", "1-0", other))
        self.assertTrue(any("diverges at ply 2" in p for p in problems), problems)

    def test_the_same_move_found_by_a_different_tree_is_a_divergence(self):
        other = [SAME[0], ("e5", "-0.20/8 0.012s, n=2001, sd=11, nps=91000")]
        problems = self.run_check(
            game(1, "candidate", "baseline", "1-0", SAME), game(1, "baseline", "candidate", "1-0", other))
        self.assertTrue(any("diverges at ply 2" in p for p in problems), problems)

    def test_a_pair_with_one_game_missing_fails(self):
        self.assertTrue(self.run_check(game(1, "candidate", "baseline", "1-0", SAME)))

    def test_a_short_game_count_fails(self):
        problems = self.run_check(
            game(1, "candidate", "baseline", "1-0", SAME), game(1, "baseline", "candidate", "1-0", SAME), expect=4)
        self.assertTrue(any("2 games in the PGNs, 4 expected" in p for p in problems), problems)

    def test_a_pair_where_the_candidate_won_both_fails(self):
        problems = self.run_check(
            game(1, "candidate", "baseline", "1-0", SAME), game(1, "baseline", "candidate", "0-1", SAME))
        self.assertTrue(any("not exactly 1 of 2" in p for p in problems), problems)

    def test_the_main_function_exits_nonzero_on_a_divergence(self):
        other = [SAME[0], ("c5", SAME[1][1])]
        with tempfile.TemporaryDirectory() as directory:
            path = write(directory, "chunk-0.pgn", game(1, "candidate", "baseline", "1-0", SAME),
                         game(1, "baseline", "candidate", "1-0", other))
            self.assertEqual(1, match_parity.main([path]))
            ok = write(directory, "chunk-1.pgn", game(1, "candidate", "baseline", "1-0", SAME),
                       game(1, "baseline", "candidate", "1-0", SAME))
            self.assertEqual(0, match_parity.main([ok, "--games", "2"]))


if __name__ == "__main__":
    unittest.main()
