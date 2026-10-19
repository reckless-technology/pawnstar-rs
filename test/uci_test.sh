#!/usr/bin/env bash
# UCI protocol regression test. Drives the actual engine binary over stdin/stdout with synchronized I/O
# (read the engine's responses before sending the next command, so `quit` never cancels a running search)
# and asserts on the protocol behaviour. Usage: test/uci_test.sh [path-to-engine]   (default build/pawnstar)
#
# Why a script and not a C++ unit test: these features (seldepth / currmove / searchmoves, the uciok/readyok
# handshake) are about the engine's behaviour *as a UCI process*, so the only faithful test drives the
# built binary the way a GUI does.
set -u

ENGINE="${1:-build/pawnstar}"
if [ ! -x "$ENGINE" ]; then
    echo "[FAIL] UCI: engine binary '$ENGINE' not found or not executable (run 'make' first)"
    exit 1
fi

fails=0
pass() { echo "[PASS] $1"; }
fail() {
    echo "[FAIL] $1"
    fails=$((fails + 1))
}

# Drive the engine through a coprocess so responses can be read synchronously.
coproc ENG { "$ENGINE" 2>/dev/null; }
send() { printf '%s\n' "$1" >&"${ENG[1]}"; }

# read_until <extended-regex> <timeout-seconds>: echo every line received until one matches the regex
# (then stop). Returns non-zero if the timeout elapses without a match.
read_until() {
    local re="$1" timeout="${2:-15}" line
    while IFS= read -r -t "$timeout" line <&"${ENG[0]}"; do
        printf '%s\n' "$line"
        [[ "$line" =~ $re ]] && return 0
    done
    return 1
}

# 1) uci handshake: id + option advertisements + uciok
send "uci"
out=$(read_until '^uciok' 10)
echo "$out" | grep -q '^id name Pawnstar' && pass "uci: id name" || fail "uci: missing 'id name Pawnstar'"
echo "$out" | grep -q '^uciok' && pass "uci: uciok" || fail "uci: missing uciok"
echo "$out" | grep -q 'option name Threads' && pass "uci: advertises Threads option" || fail "uci: no Threads option"

# 2) isready / readyok handshake
send "isready"
read_until '^readyok' 10 >/dev/null && pass "isready: readyok" || fail "isready: no readyok"

# Deterministic, real searches: single thread and no opening book (otherwise `go` returns a book move
# instantly, before any search, and emits no info lines).
send "setoption name Threads value 1"
send "setoption name OwnBook value false"

# 3) info seldepth: present on a real search and >= the nominal depth (quiescence extends beyond it).
send "ucinewgame"
send "position startpos"
send "go depth 12"
out=$(read_until '^bestmove' 120)
# The last iteration's summary line: once a search runs past the ~3 s currmove gate (a slow or
# instrumented build), `info depth N currmove ...` lines follow it and carry no seldepth.
last_info=$(echo "$out" | grep '^info depth.* seldepth ' | tail -1)
if [ -n "$last_info" ]; then
    d=$(echo "$last_info" | sed -E 's/.*info depth +([0-9]+).*/\1/')
    s=$(echo "$last_info" | sed -E 's/.*seldepth +([0-9]+).*/\1/')
    if [ -n "$s" ] && [ "$s" -ge "$d" ]; then
        pass "info seldepth present and >= depth (seldepth $s >= depth $d)"
    else
        fail "info seldepth bad/absent: '$last_info'"
    fi
else
    fail "go depth 12 produced no info lines (book not disabled? search not run?)"
fi

# 4) go searchmoves restricts the root. Force a move the engine would never choose (a2a3); if the filter
#    works the bestmove must be exactly that. (A normal search from startpos returns a real developing move.)
send "position startpos"
send "go searchmoves a2a3 depth 10"
out=$(read_until '^bestmove' 120)
bm=$(echo "$out" | grep '^bestmove')
echo "$bm" | grep -q '^bestmove a2a3' && pass "go searchmoves restricts root (forced a2a3)" || fail "searchmoves: expected 'bestmove a2a3', got '$bm'"

# 5) info currmove / currmovenumber: emitted once a search runs past the ~3s reporting gate. Use a movetime
#    well beyond the gate (6s) so there is a comfortable ~3s window of root iterations emitting currmove — a
#    tight window can be straddled by a single deep root move and emit nothing.
send "position startpos"
send "go movetime 6000"
out=$(read_until '^bestmove' 120)
echo "$out" | grep -q 'currmove [a-h][1-8][a-h][1-8].* currmovenumber [0-9]' &&
    pass "info currmove/currmovenumber emitted on a long search" || fail "currmove/currmovenumber not emitted"

# 5b) A `stop` written straight after `go`, with no pause, ends the search every time. The root search once
#     cleared the cancel flag as it began, so a stop that reached the engine before its worker got that far was
#     lost and the engine never answered (every iteration hung). A lost stop wedges the engine, so it is killed.
is_engine_wedged=0
stops_answered=0
for _ in $(seq 1 200); do
    send "position startpos"
    send "go infinite"
    send "stop"
    if ! read_until '^bestmove' 30 >/dev/null; then
        is_engine_wedged=1
        break
    fi
    stops_answered=$((stops_answered + 1))
done
[ "$stops_answered" -eq 200 ] && pass "go infinite + stop at once: 200/200 answered" ||
    fail "go infinite + stop at once: only $stops_answered/200 answered before a stop was lost"

# 5c) A `go` value that is not an integer, or missing, is ignored with its keyword - it once ended the engine
#     (std::stoi threw) - so each of these is a plain `go`, which `stop` ends.
for line in "go depth abc" "go movetime" "go wtime x btime y" "go nodes 12x"; do
    [ "$is_engine_wedged" -eq 0 ] || break
    send "position startpos"
    send "$line"
    send "stop"
    if read_until '^bestmove' 30 >/dev/null; then
        pass "$line: the engine survives and stop answers"
    else
        fail "$line: no bestmove after stop (the engine died or hung)"
        is_engine_wedged=1
    fi
done
send "isready"
[ "$is_engine_wedged" -eq 0 ] && read_until '^readyok' 10 >/dev/null && pass "still responsive after malformed go values" ||
    fail "not responsive after the stop and malformed-value checks"

if [ "$is_engine_wedged" -eq 0 ]; then
    send "quit"
    wait "$ENG_PID" 2>/dev/null
else
    kill -9 "$ENG_PID" 2>/dev/null
fi

# 5d) `quit` written straight after `go infinite`: the engine exits every time (a lost quit left it searching for
#     ever). A fresh process each time, killed after 30 s if it has not exited; the first that hangs ends the loop.
quits_exited=0
for _ in $(seq 1 20); do
    printf 'setoption name OwnBook value false\nposition startpos\ngo infinite\nquit\n' | "$ENGINE" >/dev/null 2>&1 &
    quit_pid=$!
    for _ in $(seq 1 300); do
        kill -0 "$quit_pid" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$quit_pid" 2>/dev/null; then
        kill -9 "$quit_pid" 2>/dev/null
        wait "$quit_pid" 2>/dev/null
        break
    fi
    wait "$quit_pid" 2>/dev/null
    quits_exited=$((quits_exited + 1))
done
[ "$quits_exited" -eq 20 ] && pass "go infinite + quit at once: the engine exits, 20/20" ||
    fail "go infinite + quit at once: an engine never exited (after $quits_exited that did)"

# 6) bench determinism: single-threaded bench is deterministic, so two runs must agree (guards the
#    uninitialised-read class of bug that made node counts wobble). Run as a plain pipe — bench is
#    synchronous on the main thread and quits cleanly.
b1=$(printf 'bench\nquit\n' | "$ENGINE" 2>/dev/null | grep -oE '^[0-9]+ nodes' | grep -oE '^[0-9]+')
b2=$(printf 'bench\nquit\n' | "$ENGINE" 2>/dev/null | grep -oE '^[0-9]+ nodes' | grep -oE '^[0-9]+')
if [ -n "$b1" ] && [ "$b1" = "$b2" ]; then
    pass "bench is deterministic across runs ($b1 nodes)"
else
    fail "bench nondeterministic: run1='$b1' run2='$b2'"
fi

if [ "$fails" -eq 0 ]; then
    echo "[PASS] UCI: protocol handshakes, seldepth, searchmoves, currmove, stop and quit at once, malformed go values, bench determinism"
    exit 0
fi
echo "[FAIL] UCI: $fails check(s) failed"
exit 1
