#!/usr/bin/env bash
# Publishes the current version as a GitHub release: builds the release
# bundle, zips it, and creates the release with notes from RELEASE_NOTES.md
# if that file exists, otherwise from the commits since the previous tag.
#
#   bash scripts/publish-release.sh            # draft release
#   bash scripts/publish-release.sh --publish  # public release
#
# The bundle is ad-hoc signed and not notarized, so the notes carry the
# Privacy & Security unblock instructions. Not right-click-Open: macOS Sequoia
# removed that shortcut for exactly this class of app, and the advice sends
# people to a menu item that no longer does anything. Never run this while an
# Anicat is playing: the package step rebuilds dist/Anicat.app underneath a
# copy launched from there.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(tr -d '[:space:]' < "$ROOT/version.txt")"
TAG="v$VERSION"
ZIP="$ROOT/AnicatApple/dist/Anicat-${VERSION}-macos-arm64.zip"
DMG="$ROOT/AnicatApple/dist/Anicat-${VERSION}-macos-arm64.dmg"
# The iPhone build is attached when one has been made. It is not built here:
# it needs xcodegen and a device SDK, and a Mac release should not fail because
# the phone half did not compile. Run scripts/package-anicat-ios-ipa.sh first
# when the release is meant to carry it.
IPA="$ROOT/AnicatApple/dist/Anicat-${VERSION}-ios.ipa"
ASSETS=("$ZIP" "$DMG")
[ -f "$IPA" ] && ASSETS+=("$IPA")
DRAFT="--draft"
[ "${1:-}" = "--publish" ] && DRAFT=""

if git -C "$ROOT" rev-parse "$TAG" >/dev/null 2>&1; then
    echo "publish-release: tag $TAG already exists; bump version.txt first (scripts/bump-version.sh)" >&2
    exit 1
fi

# Before the build, not after: the notices file is compiled into the bundle, so
# a stale one found after packaging means packaging again. It goes stale
# whenever Cargo.lock changes, and nothing else would notice. The fetch first
# because the generator reads license files out of the cargo registry, and on
# a machine that has not built this lock file yet they are not there.
(cd "$ROOT/core" && cargo fetch --locked)
python3 "$ROOT/scripts/generate-third-party-notices.py" --check

# Ad-hoc, never the machine's own identity. The package script auto-detects an
# "Apple Development" certificate when none is given, and that signature carries
# the developer's name, email and Team ID into a zip anybody can `codesign -dv`.
# A development certificate also expires in a year and is not a distribution
# certificate, so it buys the download nothing in exchange.
ANICAT_CODESIGN_IDENTITY="-" bash "$ROOT/scripts/package-anicat-macos-app.sh" release zip
# Both formats, because they answer different questions: the zip is what
# `install_macos.sh` resolves out of the API by name, and the dmg is what a
# person downloading by hand expects to drag. The second run reuses the same
# build directory, so it repackages rather than recompiling.
ANICAT_CODESIGN_IDENTITY="-" bash "$ROOT/scripts/package-anicat-macos-app.sh" release dmg

NOTES="$(mktemp)"
if [ -f "$ROOT/RELEASE_NOTES.md" ]; then
    # Committed notes satisfy this -f check forever, so without the heading
    # match the next version ships the previous version's notes verbatim.
    if ! head -1 "$ROOT/RELEASE_NOTES.md" | grep -q "$VERSION"; then
        echo "publish-release: RELEASE_NOTES.md heading is not $VERSION; rewrite it or delete it to fall back to the commit log" >&2
        exit 1
    fi
    cat "$ROOT/RELEASE_NOTES.md" > "$NOTES"
else
    # --match 'v*': the newest tag by topology is `legacy/tauri`, so a bare
    # describe made the generated notes span the entire Swift rewrite.
    PREV="$(git -C "$ROOT" describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)"
    {
        echo "## Anicat $VERSION"
        echo
        echo "Native macOS build (Apple silicon, macOS 15 or later)."
        echo
        echo "### Changes"
        echo
        if [ -n "$PREV" ]; then
            git -C "$ROOT" log --no-merges --pretty='- %s' "$PREV..HEAD" | grep -vE '^- (docs|ci|build|chore|test)' || true
        else
            git -C "$ROOT" log --no-merges --pretty='- %s' -50
        fi
    } > "$NOTES"
fi
{
    echo
    echo "### Install"
    echo
    echo "Easiest, and it clears the quarantine flag for you so macOS does not"
    echo "block the app -- paste into Terminal:"
    echo
    # A fenced block, never an indented one with backticks inside: indented
    # code renders its backticks literally, a copy took them along, and zsh
    # ran the installer as a command substitution and then tried to execute
    # its output ("zsh: command not found: Step"). 6.0.0 and 6.0.1 shipped so.
    echo '```bash'
    echo 'curl -fsSL https://raw.githubusercontent.com/bonkedbythonk/anicat/master/scripts/install_macos.sh | bash'
    echo '```'
    echo
    echo "By hand: open the .dmg and drag Anicat to Applications (the .zip holds"
    echo "the same app). macOS then refuses to open it once -- click Done, then"
    echo "System Settings > Privacy & Security > Open Anyway. Right-clicking the"
    echo "app and choosing Open has not worked since macOS Sequoia."
    if [ -f "$IPA" ]; then
        echo
        echo "The .ipa is the iPhone build. iPhones install only from the App Store"
        echo "unless you sideload: AltStore or Sideloadly on a computer re-signs it"
        echo "with your own Apple ID, and on a free account that lasts seven days."
    fi
} >> "$NOTES"

git -C "$ROOT" tag -a "$TAG" -m "Anicat $VERSION"
git -C "$ROOT" push origin "$TAG"
gh release create "$TAG" "${ASSETS[@]}" --title "Anicat $VERSION" --notes-file "$NOTES" $DRAFT
rm -f "$NOTES"
echo "publish-release: $TAG ${DRAFT:+(draft) }created"
