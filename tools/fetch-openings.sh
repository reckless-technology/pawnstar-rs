#!/usr/bin/env bash
# Print the path of the pinned opening set (tools/openings.pin), fetching it over https if it is not
# cached, and verifying its SHA-256 every time. Exits non-zero, printing nothing on stdout, if the file
# cannot be fetched or its hash differs from the pin.
#
# Usage: tools/fetch-openings.sh            -> prints the cached file's path
#        OPENINGS=<file> tools/fetch-openings.sh   -> verifies that file against the pin instead
# Env:
#   OPENINGS_CACHE  directory for the download (default ${XDG_CACHE_HOME:-~/.cache}/pawnstar-rs)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=openings.pin
. "$here/openings.pin"
: "${REPO:?openings.pin has no REPO}" "${COMMIT:?no COMMIT}" "${PATH_IN_REPO:?no PATH_IN_REPO}" "${SHA256:?no SHA256}"

die() { echo "error: $*" >&2; exit 1; }

sha256_of() {
    if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null; then shasum -a 256 "$1" | cut -d' ' -f1
    else die "neither sha256sum nor shasum is available"; fi
}

verify() {
    local actual
    actual="$(sha256_of "$1")"
    [ "$actual" = "$SHA256" ] || die "opening set $1 has SHA-256 $actual, but tools/openings.pin pins $SHA256 ($REPO@${COMMIT:0:12} $PATH_IN_REPO)"
}

if [ -n "${OPENINGS:-}" ]; then
    [ -r "$OPENINGS" ] || die "openings file not readable: $OPENINGS"
    verify "$OPENINGS"
    echo "$OPENINGS"
    exit 0
fi

cache="${OPENINGS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/pawnstar-rs}"
file="$cache/openings-${COMMIT:0:12}.epd"
if [ ! -r "$file" ] || [ "$(sha256_of "$file")" != "$SHA256" ]; then
    command -v curl >/dev/null || die "curl is not available to fetch the opening set"
    mkdir -p "$cache"
    tmp="$(mktemp "$cache/openings.XXXXXX")"
    trap 'rm -f "$tmp"' EXIT
    url="https://raw.githubusercontent.com/$REPO/$COMMIT/$PATH_IN_REPO"
    echo "fetching $url" >&2
    curl -fsSL --retry 3 -o "$tmp" "$url" || die "could not fetch $url"
    verify "$tmp"
    mv "$tmp" "$file"
fi
echo "$file"
