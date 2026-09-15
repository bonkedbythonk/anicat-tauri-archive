#!/usr/bin/env bash
# bump-version.sh — Single source of truth for version bumps.
#
# Usage:  bash scripts/bump-version.sh 5.9.0
#
# Updates version.txt (canonical), core/Cargo.toml and the iOS target's
# CFBundleShortVersionString. The macOS .app packaging scripts read
# version.txt at build time, but AnicatApple/project.yml cannot: xcodegen
# has no way to read a file, so the iOS version is a literal in the spec
# and has to be rewritten here or it silently keeps shipping the old one.

set -euo pipefail

NEW_VERSION="${1:-}"

if ! echo "$NEW_VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "Usage: $0 <MAJOR.MINOR.PATCH>"
    echo "  e.g. $0 5.8.1  (bug fix)"
    echo "       $0 5.9.0  (new feature)"
    echo "       $0 6.0.0  (breaking change)"
    exit 1
fi

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

echo "$NEW_VERSION" > version.txt
echo "[1/3] version.txt  -> $NEW_VERSION"

# Only the first `version =` line: dependency tables further down also
# carry the key. awk rather than sed's `0,/re/` range, which is a GNU
# extension: on macOS's BSD sed it matched nothing and this script
# reported a bump it had not made.
awk -v v="$NEW_VERSION" '
    /^version = / && !done { sub(/"[^"]*"/, "\"" v "\""); done = 1 }
    { print }
' core/Cargo.toml > core/Cargo.toml.tmp && mv core/Cargo.toml.tmp core/Cargo.toml
grep -q "^version = \"$NEW_VERSION\"" core/Cargo.toml || { echo "core/Cargo.toml was not updated" >&2; exit 1; }
echo "[2/3] core/Cargo.toml  -> $NEW_VERSION"

# The iOS app's marketing version. Anchored to the key's own line so the
# CFBundleVersion build number just below it is left alone.
awk -v v="$NEW_VERSION" '
    /^ *CFBundleShortVersionString: / { sub(/: .*/, ": " v) }
    { print }
' AnicatApple/project.yml > AnicatApple/project.yml.tmp && mv AnicatApple/project.yml.tmp AnicatApple/project.yml
grep -q "CFBundleShortVersionString: $NEW_VERSION" AnicatApple/project.yml || { echo "AnicatApple/project.yml was not updated" >&2; exit 1; }
echo "[3/3] AnicatApple/project.yml  -> $NEW_VERSION"

# Cargo records the crate version in the lock file too.
(cd core && cargo update -p anicat-core --offline >/dev/null 2>&1 || cargo update -p anicat-core >/dev/null)

echo ""
echo "All files bumped to $NEW_VERSION."
echo "Review with: git diff"
