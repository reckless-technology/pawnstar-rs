#!/usr/bin/env bash
# Set up a Linux or macOS machine to build, test, debug and measure pawnstar-rs: the toolchain, the tools and the
# editor. Each component is checked first and installed only when it is missing, so the script is safe to run again at
# any time. Rust and its tools go into the user's own profile (~/.cargo, ~/.rustup); the system packages come from apt
# (Debian, Ubuntu, Mint: sudo asks for a password) or Homebrew (macOS). tools/setup-dev.ps1 is the same for Windows.
# VS Code's "setup environment" and "check environment" tasks run the one for the platform.
#
# What it sets up:
#   - a C toolchain for the linker (build-essential; Xcode's command-line tools on macOS)
#   - bash >= 4 for the UCI shell suite (macOS's own is 3.2: Homebrew's)
#   - rustup with the stable toolchain (rust-toolchain.toml) and rustfmt, clippy and llvm-tools
#   - cargo-llvm-cov, for line coverage
#   - the repo's git hooks (core.hooksPath .githooks) and the VS Code extensions .vscode/extensions.json recommends
#
#   tools/setup-dev.sh                # set up
#   tools/setup-dev.sh --check        # report only; exit 1 if anything is missing
#   tools/setup-dev.sh --no-persist   # add nothing to the shell profile's PATH (a test run in a scratch HOME)
set -euo pipefail
cd "$(dirname "$0")/.."

check=0
persist=1
for arg in "$@"; do
    case "$arg" in
        --check) check=1 ;;
        --no-persist) persist=0 ;;
        -h | --help) sed -n '2,19p' "$0"; exit 0 ;;
        *) echo "unknown argument: $arg" >&2; exit 2 ;;
    esac
done

# ---------------------------------------------------------------------------------------------------------------------
# Helpers (the same in every reckless-technology repo's setup script).
missing=0
ok() { printf '  \033[32mok\033[0m       %-24s %s\n' "$1" "$2"; }
miss() { missing=$((missing + 1)); printf '  \033[33mmissing\033[0m  %-24s %s\n' "$1" "$2"; }
installing() { printf '  \033[36minstall\033[0m  %-24s %s\n' "$1" "$2"; }
have() { command -v "$1" >/dev/null 2>&1; }
os=$(uname -s)

# Put a directory on this shell's PATH and, unless --no-persist or --check (which changes nothing), on every new shell's
# (the profiles a login shell, bash and zsh read).
add_to_path() {
    case ":$PATH:" in *":$1:"*) ;; *) PATH="$1:$PATH"; export PATH ;; esac
    [ "$persist" = 1 ] && [ "$check" = 0 ] || return 0
    local line="export PATH=\"$1:\$PATH\""
    local profiles=("$HOME/.profile" "$HOME/.bashrc")
    [ "$os" = Darwin ] && profiles+=("$HOME/.zprofile")
    for f in "${profiles[@]}"; do
        grep -qxF "$line" "$f" 2>/dev/null || printf '\n%s\n' "$line" >>"$f"
    done
}

sha256_of() { if have sha256sum; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

# Download a file and refuse it unless its SHA-256 is the one given.
fetch_verified() {
    curl -fsSL "$1" -o "$3"
    local got
    got=$(sha256_of "$3")
    if [ "$got" != "$2" ]; then rm -f "$3"; echo "SHA-256 mismatch for $1 (got $got, want $2)" >&2; exit 1; fi
}

sudo_cmd() { if [ "$(id -u)" = 0 ]; then "$@"; else sudo "$@"; fi; }

# apt packages that are not installed yet, installed together (Debian, Ubuntu, Mint).
apt_packages() {
    local need=()
    for p in "$@"; do
        # A here-string, not a pipe into grep -q: under pipefail a writer cut off by grep's early exit fails the test.
        grep -q 'install ok installed' <<<"$(dpkg-query -W -f='${Status}' "$p" 2>/dev/null || true)" || need+=("$p")
    done
    if [ ${#need[@]} = 0 ]; then ok 'apt packages' "$*"; return; fi
    if [ "$check" = 1 ]; then miss 'apt packages' "${need[*]}"; return; fi
    installing 'apt packages' "${need[*]}"
    sudo_cmd apt-get update -q
    sudo_cmd env DEBIAN_FRONTEND=noninteractive apt-get install -y -q "${need[@]}"
}

# Homebrew formulae that are not installed yet.
brew_packages() {
    local need=()
    for p in "$@"; do brew list --formula "$p" >/dev/null 2>&1 || need+=("$p"); done
    if [ ${#need[@]} = 0 ]; then ok 'Homebrew packages' "$*"; return; fi
    if [ "$check" = 1 ]; then miss 'Homebrew packages' "${need[*]}"; return; fi
    installing 'Homebrew packages' "${need[*]}"
    brew install "${need[@]}"
}

# macOS: Xcode's command-line tools (the compilers, git) and Homebrew.
macos_base() {
    if xcode-select -p >/dev/null 2>&1; then ok 'Xcode command-line tools' "$(xcode-select -p)"
    elif [ "$check" = 1 ]; then miss 'Xcode command-line tools' 'xcode-select --install'
    else
        installing 'Xcode command-line tools' 'a dialog opens: install, then run this script again'
        xcode-select --install || true
        exit 0
    fi
    if ! have brew; then
        for b in /opt/homebrew/bin/brew /usr/local/bin/brew; do [ -x "$b" ] && eval "$("$b" shellenv)" && break; done
    fi
    if have brew; then ok 'Homebrew' "$(brew --version | head -1)"
    elif [ "$check" = 1 ]; then miss 'Homebrew' 'https://brew.sh'
    else
        installing 'Homebrew' 'the official installer (asks for your password)'
        /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
        for b in /opt/homebrew/bin/brew /usr/local/bin/brew; do [ -x "$b" ] && eval "$("$b" shellenv)" && break; done
    fi
}

linux_base() {
    if have apt-get; then apt_packages "$@"
    else
        echo "  This is not a Debian-family Linux: install the equivalents of these packages, then run again: $*" >&2
        if [ "$check" = 1 ]; then missing=$((missing + 1)); fi
    fi
}

set_git_hooks() {
    [ -d .githooks ] || return 0
    if [ "$(git config core.hooksPath || true)" = .githooks ]; then ok 'git hooks' 'core.hooksPath .githooks'
    elif [ "$check" = 1 ]; then miss 'git hooks' 'core.hooksPath is not .githooks'
    else git config core.hooksPath .githooks; installing 'git hooks' 'core.hooksPath .githooks'; fi
}

# The extensions .vscode/extensions.json recommends, when VS Code's command line is on the PATH.
install_vscode_extensions() {
    have code && [ -f .vscode/extensions.json ] || return 0
    local installed
    installed=$(code --list-extensions 2>/dev/null || true)
    for ext in $(sed -e 's://.*$::' .vscode/extensions.json | tr -d '\n' | sed -e 's/.*"recommendations"[^[]*\[//' -e 's/\].*//' | tr -d '" ' | tr ',' ' '); do
        if grep -qixF "$ext" <<<"$installed"; then ok 'VS Code extension' "$ext"
        elif [ "$check" = 1 ]; then miss 'VS Code extension' "$ext"
        else installing 'VS Code extension' "$ext"; code --install-extension "$ext" --force >/dev/null; fi
    done
}

# ---------------------------------------------------------------------------------------------------------------------
echo "pawnstar-rs: $([ "$check" = 1 ] && echo checking || echo 'setting up') the $os development environment"

case "$os" in
    Linux) linux_base build-essential curl git ;;
    Darwin) macos_base; have brew && brew_packages bash ;;
    *) echo "unsupported system: $os (tools/setup-dev.ps1 is the Windows script)" >&2; exit 2 ;;
esac

# bash >= 4, which the UCI shell suite's coproc needs.
bash4=""
for b in /opt/homebrew/bin/bash /usr/local/bin/bash /bin/bash /usr/bin/bash; do
    if [ -x "$b" ] && "$b" -c '[[ ${BASH_VERSINFO[0]} -ge 4 ]]' 2>/dev/null; then bash4=$b; break; fi
done
if [ -n "$bash4" ]; then ok 'bash >= 4' "$bash4 ($("$bash4" -c 'echo $BASH_VERSION'))"; else miss 'bash >= 4' 'for the UCI shell suite'; fi

# rustup and the stable toolchain.
cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
[ -x "$cargo_bin/rustup" ] && add_to_path "$cargo_bin"
if have rustup; then ok 'rustup' "$(rustup --version 2>/dev/null | head -1)"
elif [ "$check" = 1 ]; then miss 'rustup' 'and the stable toolchain'
else
    case "$os-$(uname -m)" in
        Linux-x86_64) triple=x86_64-unknown-linux-gnu ;;
        Linux-aarch64) triple=aarch64-unknown-linux-gnu ;;
        Darwin-arm64) triple=aarch64-apple-darwin ;;
        Darwin-x86_64) triple=x86_64-apple-darwin ;;
        *) echo "no rustup-init for $os $(uname -m)" >&2; exit 1 ;;
    esac
    installing 'rustup' "rustup-init ($triple) from static.rust-lang.org"
    scratch=$(mktemp -d)
    url="https://static.rust-lang.org/rustup/dist/$triple/rustup-init"
    fetch_verified "$url" "$(curl -fsSL "$url.sha256" | cut -d' ' -f1)" "$scratch/rustup-init"
    chmod +x "$scratch/rustup-init"
    "$scratch/rustup-init" -y --default-toolchain none --no-modify-path >/dev/null
    rm -rf "$scratch"
    add_to_path "$cargo_bin"
fi

if have rustup; then
    components=(rustfmt clippy llvm-tools)
    installed=$(rustup component list --installed --toolchain stable 2>/dev/null || true)
    lacking=()
    for c in "${components[@]}"; do grep -q "^$c" <<<"$installed" || lacking+=("$c"); done
    if [ ${#lacking[@]} = 0 ]; then ok 'Rust stable' "$(rustc +stable --version) with ${components[*]}"
    elif [ "$check" = 1 ]; then miss 'Rust stable' "needs ${lacking[*]}"
    else
        installing 'Rust stable' "with ${components[*]}"
        rustup toolchain install stable --profile minimal --component "$(IFS=,; echo "${components[*]}")"
    fi

    if have cargo-llvm-cov; then ok 'cargo-llvm-cov' "$(cargo llvm-cov --version)"
    elif [ "$check" = 1 ]; then miss 'cargo-llvm-cov' 'line coverage'
    else installing 'cargo-llvm-cov' 'cargo install --locked'; cargo +stable install cargo-llvm-cov --locked; fi
fi

set_git_hooks
install_vscode_extensions

if [ "$check" = 1 ]; then
    if [ "$missing" -gt 0 ]; then echo "$missing missing: run tools/setup-dev.sh (VS Code: the 'setup environment' task)."; exit 1; fi
    echo 'Everything is in place.'
else
    echo 'Done. Open a new terminal (or restart VS Code) for PATH changes to reach it, then: cargo test --release'
fi
