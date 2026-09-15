#!/bin/bash
set -e

# Anicat macOS installer. Downloads the latest release, installs it into
# /Applications and clears the quarantine flag the ad-hoc signature would
# otherwise trip over.
#
#   curl -fsSL https://raw.githubusercontent.com/bonkedbythonk/anicat/master/scripts/install_macos.sh | bash
#
# The nightly -- rebuilt from the development branch each night it has new
# commits, and not tested before it goes out -- is opt-in:
#
#   curl -fsSL https://raw.githubusercontent.com/bonkedbythonk/anicat/master/scripts/install_macos.sh | bash -s -- --nightly
#
# Running the plain command again moves a nightly install back to stable.

REPO="bonkedbythonk/anicat"
NIGHTLY=""
for arg in "$@"; do
    case "$arg" in
        --nightly) NIGHTLY=1 ;;
        *) echo "Unknown option: $arg (the only option is --nightly)" >&2; exit 1 ;;
    esac
done
APP_NAME="Anicat.app"
INSTALL_PATH="/Applications/$APP_NAME"

if [ "$(uname -m)" != "arm64" ]; then
    echo "Anicat is built for Apple silicon only; this machine reports $(uname -m)."
    echo "Build from source: https://github.com/bonkedbythonk/anicat#building-from-source"
    exit 1
fi

# The bundle's LSMinimumSystemVersion is 15.0. On an older macOS the install
# below succeeds and the app then refuses to open with a generic "cannot be
# used on this version" dialog, which is a worse place to learn it than here.
MACOS_MAJOR="$(sw_vers -productVersion | cut -d. -f1)"
if [ "${MACOS_MAJOR:-0}" -lt 15 ]; then
    echo "Anicat needs macOS 15 (Sequoia) or later; this Mac runs $(sw_vers -productVersion)."
    exit 1
fi

# A standard (non-admin) account cannot write /Applications, and `rm -rf`
# there fails after the download instead of before it. ~/Applications is
# where macOS itself puts per-user apps and Launchpad/Spotlight index it too.
if [ ! -w "$(dirname "$INSTALL_PATH")" ]; then
    INSTALL_PATH="$HOME/Applications/$APP_NAME"
    mkdir -p "$(dirname "$INSTALL_PATH")"
    echo "/Applications is not writable by this account; installing to $INSTALL_PATH instead."
fi

if [ -n "$NIGHTLY" ]; then
    # One rolling pre-release under a fixed tag with a fixed asset name
    # (.github/workflows/nightly.yml), so the URL is known without asking the
    # API, and the API's rate limit cannot get in the way.
    echo "Step 1: Using the nightly build..."
    DOWNLOAD_URL="https://github.com/$REPO/releases/download/nightly/Anicat-nightly-macos-arm64.zip"
else
    echo "Step 1: Finding the latest version..."
    # Deliberately no python3 here. A stock macOS has no usable interpreter --
    # /usr/bin/python3 is a stub that prompts for a multi-GB Xcode Command Line
    # Tools install -- and this script has to work on a machine with nothing but
    # Terminal. /releases/latest already excludes drafts and prereleases, so the
    # asset URL can be pulled straight out with grep. Everything else this script
    # uses (curl, ditto, xattr, osascript) ships with the base system.
    DOWNLOAD_URL=$(curl -sSL "https://api.github.com/repos/$REPO/releases/latest" \
        | grep -o "https://github.com/$REPO/releases/download/[^\"]*macos-arm64\.zip" \
        | head -n 1)

    if [ -z "$DOWNLOAD_URL" ]; then
        # The API allows 60 anonymous requests an hour per public IP, shared by
        # everyone behind the same NAT (a campus, an office, a CGNAT carrier), and
        # a rate-limited answer is a JSON error with no asset in it. The web
        # redirect for /releases/latest has no such limit and names the tag, and
        # publish-release.sh names the asset from the version, so the URL can be
        # rebuilt from the tag alone.
        TAG="$(curl -sSI "https://github.com/$REPO/releases/latest" \
            | grep -i '^location:' \
            | grep -o '/releases/tag/v[0-9][^[:space:]]*' \
            | sed 's|/releases/tag/||' \
            | head -n 1)"
        if [ -n "$TAG" ]; then
            DOWNLOAD_URL="https://github.com/$REPO/releases/download/$TAG/Anicat-${TAG#v}-macos-arm64.zip"
        fi
    fi
fi

if [ -z "$DOWNLOAD_URL" ]; then
    echo "Couldn't find a download link. The latest release might still be building."
    echo "Try again in a few minutes, or download manually from:"
    echo "  https://github.com/$REPO/releases"
    exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
TMP_ZIP="$TMP_DIR/anicat.zip"

echo "Step 2: Downloading... (this might take a minute)"
# -f: without it a 404 (no nightly published yet) is saved as the zip and
# only surfaces later as "the archive did not contain Anicat.app".
if [ -t 2 ]; then
    curl -fL -o "$TMP_ZIP" "$DOWNLOAD_URL" --progress-bar
else
    curl -fL -sS -o "$TMP_ZIP" "$DOWNLOAD_URL"
fi

echo "Step 3: Installing..."
# A running copy holds its own executable open; replacing the bundle under it
# leaves a half-old app that crashes on the next window. Quit it first.
osascript -e 'tell application "Anicat" to quit' 2>/dev/null || true
sleep 2

# ditto, not unzip: unzip drops the symlinks and extended attributes inside a
# .app bundle, which breaks the signature and Gatekeeper rejects the result.
ditto -x -k "$TMP_ZIP" "$TMP_DIR/extracted"

if [ ! -d "$TMP_DIR/extracted/$APP_NAME" ]; then
    echo "The downloaded archive did not contain $APP_NAME."
    exit 1
fi

# A plain `rm -rf` can leave files behind without saying so -- one locked or
# root-owned leftover from a previous botched install (the 5.x app's own
# updater is known to copy its new bundle *over* the old one instead of
# replacing it) is enough, and the ditto below would then land the new app
# on top of the remainder instead of a clean directory, reproducing the same
# merged, invalid bundle. Confirm removal actually worked before trusting it.
rm -rf "$INSTALL_PATH"
if [ -e "$INSTALL_PATH" ]; then
    echo "Could not fully remove the existing $INSTALL_PATH -- some of its files" >&2
    echo "would have been left behind and merged with the new install, which is" >&2
    echo "the exact corruption an old, broken update can cause. Remove it by hand" >&2
    echo "first, e.g.:" >&2
    echo "  sudo rm -rf \"$INSTALL_PATH\"" >&2
    exit 1
fi
ditto "$TMP_DIR/extracted/$APP_NAME" "$INSTALL_PATH"

# The bundle is ad-hoc signed, not notarized, so without this macOS refuses to
# open it and offers only "Move to Trash".
xattr -r -d com.apple.quarantine "$INSTALL_PATH" 2>/dev/null || true

# A merged/corrupted bundle from a bad previous install, or a download that
# lost the SwiftPM resource bundle in transit, crashes on launch with an
# obscure NSBundle.module assertion instead of failing here where it is easy
# to explain. Checking for it beats opening a fresh crash report.
if [ ! -d "$INSTALL_PATH/Contents/Resources/AnicatApple_AnicatUI.bundle" ]; then
    echo "The installed app is missing Contents/Resources/AnicatApple_AnicatUI.bundle" >&2
    echo "and will crash on launch. The download may have been interrupted --" >&2
    echo "delete $INSTALL_PATH and run this installer again." >&2
    exit 1
fi
if ! codesign --verify "$INSTALL_PATH" 2>/dev/null; then
    echo "The installed app's signature doesn't verify, which usually means its" >&2
    echo "bundle has stray files mixed in from a previous install. Remove it by" >&2
    echo "hand and run this installer again:" >&2
    echo "  rm -rf \"$INSTALL_PATH\"" >&2
    exit 1
fi

echo "Step 4: Opening Anicat..."
open "$INSTALL_PATH"

# Quoted delimiter: unquoted, the backslash ending the ears line is a line
# continuation, and the cat printed as "/\_/   ( ^.^ )" with no ears row.
cat <<'EOF'

    /\_/\
   ( ^.^ )   Anicat is ready!
    > ^ <

EOF
echo "The app should open now."
echo "If not, open your Applications folder and click Anicat."
echo ""
echo "Connect your AniList account from Settings to sync your library."

# An upgrade from the 5.x Tauri app needs no uninstall step -- same name, same
# bundle id, same path, so the copy above overwrote it -- but its data stays
# behind, and on a machine that ran 5.x for a while the dead web view cache
# alone is around 90 MB. Say so here rather than in the README, where nobody
# who ran a one-line installer will look.
LEGACY_TOTAL="$(du -sck \
    "$HOME/Library/Application Support/Anicat/registry.db" \
    "$HOME/Library/Application Support/Anicat/registry.json" \
    "$HOME/Library/Application Support/Anicat/covers" \
    "$HOME/Library/Caches/com.anicat.app/WebKit" \
    "$HOME/Library/WebKit/com.anicat.app" \
    2>/dev/null | tail -1 | cut -f1)"
if [ -n "$LEGACY_TOTAL" ] && [ "$LEGACY_TOTAL" -gt 1024 ]; then
    echo ""
    echo "The old version left about $((LEGACY_TOTAL / 1024)) MB of data behind."
    echo "Nothing needs it. To list it and move it to the Trash, run:"
    echo "  curl -fsSLO https://raw.githubusercontent.com/$REPO/master/scripts/cleanup_legacy_macos.sh"
    echo "  bash cleanup_legacy_macos.sh"
fi
