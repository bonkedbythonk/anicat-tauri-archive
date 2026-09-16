#!/usr/bin/env python3
"""Writes THIRD_PARTY_NOTICES.txt, the license notices shipped inside the app.

    python3 scripts/generate-third-party-notices.py          # rewrite the file
    python3 scripts/generate-third-party-notices.py --check  # exit 1 if stale
    python3 scripts/generate-third-party-notices.py --output PATH  # write elsewhere

The Rust half comes from `cargo metadata` over core/Cargo.lock and
server/Cargo.lock (the Windows build's crate), so a dependency change in
either is picked up by running this again. The native half is two hand-kept
tables: MPVKit's prebuilt libraries for the Apple builds, and the mpv.exe the
Windows zip carries. Nothing in SwiftPM's resolution or in a prebuilt mpv.exe
says what license a binary is under.

One file serves every target: the Windows packaging script copies this same
output into its zip. So a change to server/Cargo.lock, or to the mpv pinned in
scripts/package-anicat-windows.ps1, makes the committed file stale, and
publish-release.sh's `--check` then refuses to build the Mac release until it
is regenerated and committed.

No network access. Every text not found in the local cargo registry is kept in
scripts/third-party-licenses/, so a release worktree reproduces the file byte
for byte -- which is what `--check` in publish-release.sh relies on.
"""

import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TEXTS = os.path.join(ROOT, "scripts", "third-party-licenses")
UI_RESOURCES = os.path.join(ROOT, "AnicatApple", "Sources", "AnicatUI", "Resources")
OUT = os.path.join(UI_RESOURCES, "Legal", "THIRD_PARTY_NOTICES.txt")

# Per lock file, the targets that lock file ships for. Both Apple targets for
# core, because the iPhone build links the same engine for another target and
# a platform-gated dependency present on only one of them would otherwise be
# missing from the notices of that one. The server crate ships only as the
# Windows exe; it also runs on a Mac for development, but nothing built there
# is distributed, and naming darwin here would list crates no zip contains.
LOCKFILES = [
    ("core", ["aarch64-apple-darwin", "aarch64-apple-ios"]),
    ("server", ["x86_64-pc-windows-msvc"]),
]

# When a crate offers a choice ("MIT OR Apache-2.0"), the alternative taken is
# the one ranked first here. MIT first because its text is a few lines and
# carries the copyright holder; Apache-2.0's is 11 KB of identical boilerplate.
PREFERENCE = [
    "MIT", "ISC", "Zlib", "BSD-2-Clause", "BSD-3-Clause", "0BSD", "MIT-0",
    "Unlicense", "CC0-1.0", "Unicode-3.0", "Apache-2.0", "BSL-1.0", "MPL-2.0",
    "CDLA-Permissive-2.0", "LGPL-2.1-or-later",
]

# Filename fragments that identify which license a file in a crate holds, for
# crates that ship one file per alternative (LICENSE-MIT, LICENSE-APACHE).
FILE_HINTS = {
    "MIT": "MIT", "Apache-2.0": "APACHE", "BSD-2-Clause": "BSD",
    "BSD-3-Clause": "BSD", "ISC": "ISC", "Zlib": "ZLIB", "Unlicense": "UNLICENSE",
    "Unicode-3.0": "UNICODE", "MPL-2.0": "MPL", "CC0-1.0": "CC0", "BSL-1.0": "BOOST",
    "0BSD": "0BSD",
}

# 22 crates in the lock file publish no license file in their package. Each is
# mapped to its upstream repository's text, kept in scripts/third-party-licenses.
# A crate family shares its parent repository's file. Exact names, not
# prefixes: a new crate called uniffi-something from another author would
# otherwise inherit Mozilla's text silently instead of failing below.
_RQBIT = ["rqbit-LICENSE.txt", "Apache-2.0.txt"]
_UNIFFI = ["MPL-2.0.txt"]
NO_FILE_FALLBACK = {
    **{name: _RQBIT for name in [
        "librqbit", "librqbit-bencode", "librqbit-buffers", "librqbit-clone-to-owned",
        "librqbit-core", "librqbit-dht", "librqbit-peer-protocol",
        "librqbit-sha1-wrapper", "librqbit-tracker-comms", "librqbit-upnp",
    ]},
    **{name: _UNIFFI for name in [
        "uniffi", "uniffi_bindgen", "uniffi_build", "uniffi_checksum_derive",
        "uniffi_core", "uniffi_macros", "uniffi_meta", "uniffi_testing", "uniffi_udl",
    ]},
    "governor": ["governor-LICENSE.txt"],
    "assert_cfg": ["assert_cfg-LICENSE-ZLIB.md"],
    # MIT OR Apache-2.0 with neither file packaged, and the MIT one upstream
    # names no holder beyond "The Knurling-rs developers": Apache-2.0 taken.
    "defmt-parser": ["Apache-2.0.txt"],
}

LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|COPYRIGHT|UNLICENSE|NOTICE)", re.I)

# MPVKit 1.0.0's prebuilt libraries, as linked through its MPVKit-GPL product.
# Versions are the pinned releases in MPVKit's
# Sources/BuildScripts/XCFrameworkBuild/main.swift. The libass-build and
# gnutls-build repositories each package several libraries under one release
# number, which is the number given for all of them.
NATIVE = [
    {
        "name": "mpv", "version": "0.41.0",
        "license": "GPL-2.0-or-later (built with -Dgpl=true), used here under GPL-3.0",
        "source": ["https://github.com/mpv-player/mpv/tree/v0.41.0"],
        "texts": ["mpv-Copyright.txt"],
    },
    {
        "name": "FFmpeg", "version": "8.1.2",
        "license": "GPL-3.0-or-later (built with --enable-gpl --enable-version3)",
        "source": ["https://github.com/FFmpeg/FFmpeg/tree/n8.1.2"],
        "texts": ["FFmpeg-LICENSE.md"],
    },
    {
        "name": "Samba (libsmbclient)", "version": "4.15.13-2512",
        "license": "GPL-3.0-or-later",
        "source": ["https://github.com/samba-team/samba", "https://github.com/mpvkit/libsmbclient-build"],
        "texts": [],
    },
    {
        "name": "GnuTLS", "version": "3.8.11",
        "license": "LGPL-2.1-or-later",
        "source": ["https://gitlab.com/gnutls/gnutls", "https://github.com/mpvkit/gnutls-build"],
        "texts": [],
    },
    {
        "name": "Nettle and Hogweed", "version": "3.8.11",
        "license": "LGPL-3.0-or-later (offered as LGPL-3.0-or-later or GPL-2.0-or-later; LGPL-3.0 taken)",
        "source": ["https://git.lysator.liu.se/nettle/nettle", "https://github.com/mpvkit/gnutls-build"],
        "texts": [],
    },
    {
        "name": "GMP", "version": "3.8.11",
        "license": "LGPL-3.0-or-later (offered as LGPL-3.0-or-later or GPL-2.0-or-later; LGPL-3.0 taken)",
        "source": ["https://gmplib.org", "https://github.com/mpvkit/gnutls-build"],
        "texts": [],
    },
    {
        "name": "libbluray", "version": "1.4.0",
        "license": "LGPL-2.1-or-later",
        "source": ["https://code.videolan.org/videolan/libbluray", "https://github.com/mpvkit/libbluray-build"],
        "texts": [],
    },
    {
        "name": "uchardet", "version": "0.0.8",
        "license": "LGPL-2.1-or-later (offered as MPL-1.1, GPL-2.0-or-later or LGPL-2.1-or-later; LGPL-2.1 taken)",
        "source": ["https://gitlab.freedesktop.org/uchardet/uchardet", "https://github.com/mpvkit/libuchardet-build"],
        "texts": [],
    },
    {
        "name": "libplacebo", "version": "7.360.1",
        "license": "LGPL-2.1-or-later",
        "source": ["https://github.com/haasn/libplacebo", "https://github.com/mpvkit/libplacebo-build"],
        "texts": [],
    },
    {
        "name": "FriBidi", "version": "0.17.5",
        "license": "LGPL-2.1-or-later",
        "source": ["https://github.com/fribidi/fribidi", "https://github.com/mpvkit/libass-build"],
        "texts": [],
    },
    {
        "name": "libass", "version": "0.17.5",
        "license": "ISC",
        "source": ["https://github.com/libass/libass", "https://github.com/mpvkit/libass-build"],
        "texts": ["libass-COPYING.txt"],
    },
    {
        "name": "FreeType", "version": "0.17.5",
        "license": "FreeType License (offered as FTL or GPL-2.0; FTL taken)",
        "source": ["https://gitlab.freedesktop.org/freetype/freetype", "https://github.com/mpvkit/libass-build"],
        # The FTL's one condition beyond keeping the text: this credit line.
        "note": "Portions of this software are copyright (c) The FreeType Project (www.freetype.org). All rights reserved.",
        "texts": ["FreeType-FTL.txt"],
    },
    {
        "name": "HarfBuzz", "version": "0.17.5",
        "license": "MIT (\"Old MIT\")",
        "source": ["https://github.com/harfbuzz/harfbuzz", "https://github.com/mpvkit/libass-build"],
        "texts": ["HarfBuzz-COPYING.txt"],
    },
    {
        "name": "libunibreak", "version": "0.17.5",
        "license": "Zlib",
        "source": ["https://github.com/adah1972/libunibreak", "https://github.com/mpvkit/libass-build"],
        "texts": ["libunibreak-LICENCE.txt"],
    },
    {
        "name": "OpenSSL", "version": "3.3.5",
        "license": "Apache-2.0",
        "source": ["https://github.com/openssl/openssl", "https://github.com/mpvkit/openssl-build"],
        "texts": [],
    },
    {
        "name": "dav1d", "version": "1.5.3",
        "license": "BSD-2-Clause",
        "source": ["https://code.videolan.org/videolan/dav1d", "https://github.com/mpvkit/libdav1d-build"],
        "texts": ["dav1d-COPYING.txt"],
    },
    {
        "name": "Little CMS", "version": "2.17.0",
        "license": "MIT",
        "source": ["https://github.com/mm2/Little-CMS", "https://github.com/mpvkit/lcms2-build"],
        "texts": ["LittleCMS-LICENSE.txt"],
    },
    {
        "name": "libdovi (dovi_tool)", "version": "3.3.2",
        "license": "MIT",
        "source": ["https://github.com/quietvoid/dovi_tool", "https://github.com/mpvkit/libdovi-build"],
        "texts": ["dovi_tool-LICENSE.txt"],
    },
    {
        "name": "MoltenVK", "version": "1.4.2",
        "license": "Apache-2.0",
        "source": ["https://github.com/KhronosGroup/MoltenVK", "https://github.com/mpvkit/moltenvk-build"],
        "texts": [],
    },
    {
        "name": "shaderc, with glslang and SPIRV-Tools", "version": "2025.5.0",
        "license": "Apache-2.0; glslang under the licenses reproduced here",
        "source": ["https://github.com/google/shaderc", "https://github.com/KhronosGroup/glslang",
                   "https://github.com/KhronosGroup/SPIRV-Tools", "https://github.com/mpvkit/libshaderc-build"],
        "texts": ["glslang-LICENSE.txt"],
    },
    {
        "name": "uavs3d", "version": "1.2.1",
        "license": "BSD-3-Clause",
        "source": ["https://github.com/uavs3/uavs3d", "https://github.com/mpvkit/libuavs3d-build"],
        "texts": ["uavs3d-COPYING.txt"],
    },
    {
        "name": "LuaJIT (macOS build only)", "version": "2.1.0",
        "license": "MIT",
        "source": ["https://github.com/LuaJIT/LuaJIT", "https://github.com/mpvkit/libluajit-build"],
        "texts": ["LuaJIT-COPYRIGHT.txt"],
    },
    {
        "name": "MPVKit (build scripts and patches)", "version": "1.0.0",
        "license": "LGPL-3.0",
        "source": ["https://github.com/mpvkit/MPVKit/tree/1.0.0"],
        "texts": [],
    },
]

# mpv.exe in the Windows zip: shinchiro's tagged release build of mpv 0.41.0
# (mpv-0.41.0-x86_64.7z, pinned by hash in scripts/package-anicat-windows.ps1).
# All of it is linked statically into the one exe, except d3dcompiler_43.dll,
# which the archive carries beside it.
#
# How this list was derived, and how to re-derive it on an mpv bump: the
# binary's own feature list (`strings mpv.exe | grep 'List of enabled'`) for
# what mpv links, and the `--enable-lib*` flags in shinchiro's
# packages/ffmpeg.cmake for what FFmpeg links. That repository's packages/
# directory is its build system's whole package set and names libraries this
# exe does not contain, so it is not the source. The builds take each
# library's git head at build time and record no per-library version, hence
# the archive's build date where a version would go.
_WIN_BUILD = "(shinchiro build of 2025-12-25)"
# Shipped as plain files in the Windows zip's mpv\ folder rather than linked
# into anything: the on-screen controller the player loads, and the icon font
# it draws its buttons with. Anime4K is listed by anime4k_notice() with the
# Mac app's copy of the same shaders, which is where the Windows zip takes
# them from.
WINDOWS_SCRIPTS = [
    {"name": 'ModernZ (mpv on-screen controller, modified for Anicat)', "version": '0.3.3', "license": 'LGPL-2.1-or-later (a derivative of mpv\'s osc.lua by way of mpv-osc-modern, ModernX and its forks)', "source": ['https://github.com/Samillion/ModernZ'], "texts": []},
    {"name": 'Fluent UI System Icons (the icon font ModernZ draws its buttons with)', "version": '(bundled with ModernZ 0.3.3)', "license": 'MIT', "source": ['https://github.com/microsoft/fluentui-system-icons'], "texts": ['fluentui-system-icons-LICENSE.txt']},
]

WINDOWS_NATIVE = [
    {"name": 'mpv', "version": "0.41.0", "license": 'GPL-2.0-or-later (built with gpl); linked with the FFmpeg below, the executable as a whole is GPL-3.0-or-later', "source": ['https://github.com/mpv-player/mpv/tree/v0.41.0', 'https://github.com/shinchiro/mpv-winbuild-cmake'], "texts": ['mpv-Copyright.txt']},
    {"name": 'FFmpeg', "version": _WIN_BUILD, "license": 'GPL-3.0-or-later (built with --enable-gpl --enable-version3)', "source": ['https://github.com/FFmpeg/FFmpeg', 'https://github.com/shinchiro/mpv-winbuild-cmake/blob/master/packages/ffmpeg.cmake'], "texts": ['FFmpeg-LICENSE.md']},
    {"name": 'x264', "version": _WIN_BUILD, "license": 'GPL-2.0-or-later', "source": ['https://code.videolan.org/videolan/x264'], "texts": []},
    {"name": 'x265', "version": _WIN_BUILD, "license": 'GPL-2.0-or-later', "source": ['https://bitbucket.org/multicoreware/x265_git'], "texts": []},
    {"name": 'Rubber Band Library', "version": _WIN_BUILD, "license": 'GPL-2.0-or-later', "source": ['https://github.com/breakfastquay/rubberband'], "texts": []},
    {"name": 'libdvdnav and libdvdread', "version": _WIN_BUILD, "license": 'GPL-2.0-or-later', "source": ['https://code.videolan.org/videolan/libdvdnav', 'https://code.videolan.org/videolan/libdvdread'], "texts": []},
    {"name": 'libbluray', "version": _WIN_BUILD, "license": 'LGPL-2.1-or-later', "source": ['https://code.videolan.org/videolan/libbluray'], "texts": []},
    {"name": 'libplacebo', "version": _WIN_BUILD, "license": 'LGPL-2.1-or-later', "source": ['https://github.com/haasn/libplacebo'], "texts": []},
    {"name": 'FriBidi', "version": _WIN_BUILD, "license": 'LGPL-2.1-or-later', "source": ['https://github.com/fribidi/fribidi'], "texts": []},
    {"name": 'libsoxr', "version": _WIN_BUILD, "license": 'LGPL-2.1-or-later', "source": ['https://sourceforge.net/projects/soxr/'], "texts": []},
    {"name": 'libssh', "version": _WIN_BUILD, "license": 'LGPL-2.1-or-later', "source": ['https://www.libssh.org'], "texts": []},
    {"name": 'libzvbi', "version": _WIN_BUILD, "license": 'LGPL-2.1-or-later', "source": ['https://github.com/zapping-vbi/zvbi'], "texts": []},
    {"name": 'LAME', "version": _WIN_BUILD, "license": 'LGPL-2.0-or-later (LGPL-2.1 taken)', "source": ['https://lame.sourceforge.io'], "texts": []},
    {"name": 'OpenAL Soft', "version": _WIN_BUILD, "license": 'LGPL-2.0-or-later (LGPL-2.1 taken)', "source": ['https://github.com/kcat/openal-soft'], "texts": []},
    {"name": 'uchardet', "version": _WIN_BUILD, "license": 'LGPL-2.1-or-later (offered as MPL-1.1, GPL-2.0-or-later or LGPL-2.1-or-later; LGPL-2.1 taken)', "source": ['https://gitlab.freedesktop.org/uchardet/uchardet'], "texts": []},
    {"name": 'VapourSynth script interface (the runtime is not included)', "version": _WIN_BUILD, "license": 'LGPL-2.1-or-later', "source": ['https://github.com/vapoursynth/vapoursynth'], "texts": []},
    {"name": 'libass', "version": _WIN_BUILD, "license": 'ISC', "source": ['https://github.com/libass/libass'], "texts": ['libass-COPYING.txt']},
    {"name": 'FreeType', "version": _WIN_BUILD, "license": 'FreeType License (offered as FTL or GPL-2.0; FTL taken)', "source": ['https://gitlab.freedesktop.org/freetype/freetype'], "note": 'Portions of this software are copyright (c) The FreeType Project (www.freetype.org). All rights reserved.', "texts": ['FreeType-FTL.txt']},
    {"name": 'HarfBuzz', "version": _WIN_BUILD, "license": 'MIT ("Old MIT")', "source": ['https://github.com/harfbuzz/harfbuzz'], "texts": ['HarfBuzz-COPYING.txt']},
    {"name": 'Fontconfig', "version": _WIN_BUILD, "license": 'HPND-style (see text)', "source": ['https://gitlab.freedesktop.org/fontconfig/fontconfig'], "texts": ['fontconfig-COPYING.txt']},
    {"name": 'libunibreak', "version": _WIN_BUILD, "license": 'Zlib', "source": ['https://github.com/adah1972/libunibreak'], "texts": ['libunibreak-LICENCE.txt']},
    {"name": 'dav1d', "version": _WIN_BUILD, "license": 'BSD-2-Clause', "source": ['https://code.videolan.org/videolan/dav1d'], "texts": ['dav1d-COPYING.txt']},
    {"name": 'libaom', "version": _WIN_BUILD, "license": 'BSD-2-Clause', "source": ['https://aomedia.googlesource.com/aom'], "texts": ['aom-LICENSE.txt']},
    {"name": 'SVT-AV1', "version": _WIN_BUILD, "license": 'BSD-3-Clause-Clear', "source": ['https://gitlab.com/AOMediaCodec/SVT-AV1'], "texts": ['SVT-AV1-LICENSE.md']},
    {"name": 'libvpx', "version": _WIN_BUILD, "license": 'BSD-3-Clause', "source": ['https://chromium.googlesource.com/webm/libvpx'], "texts": ['libvpx-LICENSE.txt']},
    {"name": 'libwebp', "version": _WIN_BUILD, "license": 'BSD-3-Clause', "source": ['https://chromium.googlesource.com/webm/libwebp'], "texts": ['libwebp-COPYING.txt']},
    {"name": 'libjxl', "version": _WIN_BUILD, "license": 'BSD-3-Clause', "source": ['https://github.com/libjxl/libjxl'], "texts": ['libjxl-LICENSE.txt']},
    {"name": 'Opus', "version": _WIN_BUILD, "license": 'BSD-3-Clause', "source": ['https://github.com/xiph/opus'], "texts": ['opus-COPYING.txt']},
    {"name": 'Vorbis and Ogg', "version": _WIN_BUILD, "license": 'BSD-3-Clause', "source": ['https://github.com/xiph/vorbis', 'https://github.com/xiph/ogg'], "texts": ['vorbis-COPYING.txt']},
    {"name": 'Speex', "version": _WIN_BUILD, "license": 'BSD-3-Clause', "source": ['https://github.com/xiph/speex'], "texts": ['speex-COPYING.txt']},
    {"name": 'libopenmpt', "version": _WIN_BUILD, "license": 'BSD-3-Clause', "source": ['https://github.com/OpenMPT/openmpt'], "texts": ['libopenmpt-LICENSE.txt']},
    {"name": 'libmodplug', "version": _WIN_BUILD, "license": 'Public domain', "source": ['https://github.com/Konstanty/libmodplug'], "texts": []},
    {"name": 'libbs2b', "version": _WIN_BUILD, "license": 'MIT', "source": ['https://sourceforge.net/projects/bs2b/'], "texts": []},
    {"name": 'libmysofa', "version": _WIN_BUILD, "license": 'BSD-3-Clause', "source": ['https://github.com/hoene/libmysofa'], "texts": ['libmysofa-LICENSE.txt']},
    {"name": 'libaribcaption', "version": _WIN_BUILD, "license": 'MIT', "source": ['https://github.com/xqq/libaribcaption'], "texts": ['libaribcaption-LICENSE.txt']},
    {"name": 'libxml2', "version": _WIN_BUILD, "license": 'MIT', "source": ['https://gitlab.gnome.org/GNOME/libxml2'], "texts": ['libxml2-Copyright.txt']},
    {"name": 'Intel VPL dispatcher (libvpl)', "version": _WIN_BUILD, "license": 'MIT', "source": ['https://github.com/intel/libvpl'], "texts": ['libvpl-LICENSE.txt']},
    {"name": 'SRT', "version": _WIN_BUILD, "license": 'MPL-2.0', "source": ['https://github.com/Haivision/srt'], "texts": []},
    {"name": 'OpenSSL', "version": _WIN_BUILD, "license": 'Apache-2.0', "source": ['https://github.com/openssl/openssl'], "texts": []},
    {"name": 'Little CMS', "version": _WIN_BUILD, "license": 'MIT', "source": ['https://github.com/mm2/Little-CMS'], "texts": ['LittleCMS-LICENSE.txt']},
    {"name": 'zimg', "version": _WIN_BUILD, "license": 'WTFPL', "source": ['https://github.com/sekrit-twc/zimg'], "texts": ['zimg-COPYING.txt']},
    {"name": 'libjpeg-turbo', "version": _WIN_BUILD, "license": 'IJG AND BSD-3-Clause AND Zlib', "source": ['https://github.com/libjpeg-turbo/libjpeg-turbo'], "note": 'This software is based in part on the work of the Independent JPEG Group.', "texts": ['libjpeg-turbo-LICENSE.md']},
    {"name": 'libarchive', "version": _WIN_BUILD, "license": 'BSD-2-Clause', "source": ['https://github.com/libarchive/libarchive'], "texts": ['libarchive-COPYING.txt']},
    {"name": 'zlib', "version": _WIN_BUILD, "license": 'Zlib', "source": ['https://github.com/madler/zlib'], "texts": ['zlib-LICENSE.txt']},
    {"name": 'SDL2 (gamepad input)', "version": _WIN_BUILD, "license": 'Zlib', "source": ['https://github.com/libsdl-org/SDL'], "texts": ['SDL2-LICENSE.txt']},
    {"name": 'LuaJIT', "version": _WIN_BUILD, "license": 'MIT', "source": ['https://github.com/LuaJIT/LuaJIT'], "texts": ['LuaJIT-COPYRIGHT.txt']},
    {"name": 'MuJS', "version": _WIN_BUILD, "license": 'ISC', "source": ['https://codeberg.org/ccxvii/mujs'], "texts": ['mujs-COPYING.txt']},
    {"name": 'shaderc, with glslang and SPIRV-Tools', "version": _WIN_BUILD, "license": 'Apache-2.0; glslang under the licenses reproduced here', "source": ['https://github.com/google/shaderc', 'https://github.com/KhronosGroup/glslang', 'https://github.com/KhronosGroup/SPIRV-Tools'], "texts": ['glslang-LICENSE.txt']},
    {"name": 'SPIRV-Cross', "version": _WIN_BUILD, "license": 'Apache-2.0', "source": ['https://github.com/KhronosGroup/SPIRV-Cross'], "texts": []},
    {"name": 'Vulkan loader and headers', "version": _WIN_BUILD, "license": 'Apache-2.0', "source": ['https://github.com/KhronosGroup/Vulkan-Loader'], "texts": []},
    {"name": 'd3dcompiler_43.dll', "version": "June 2010 DirectX SDK", "license": "Microsoft DirectX SDK redistributable, shipped unmodified as shinchiro's archive carries it", "source": ['https://www.microsoft.com/en-us/download/details.aspx?id=6812'], "texts": []},
    {"name": 'mpv-winbuild-cmake (build scripts)', "version": _WIN_BUILD, "license": 'GPL-3.0', "source": ['https://github.com/shinchiro/mpv-winbuild-cmake'], "texts": []},
]

FULL_TEXTS = [
    ("GNU General Public License, version 3", os.path.join(ROOT, "LICENSE")),
    ("GNU Lesser General Public License, version 2.1", os.path.join(TEXTS, "LGPL-2.1.txt")),
    ("GNU Lesser General Public License, version 3", os.path.join(TEXTS, "LGPL-3.0.txt")),
    ("Apache License, version 2.0", os.path.join(TEXTS, "Apache-2.0.txt")),
    ("Mozilla Public License, version 2.0", os.path.join(TEXTS, "MPL-2.0.txt")),
]


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        # CRLF in a handful of crate files made identical licenses look
        # different to the dedupe below and doubled their blocks.
        return f.read().replace("\r\n", "\n").replace("\r", "\n").strip("\n")


def rule(title, char="="):
    return f"{title}\n{char * len(title)}"


def spdx_alternatives(expr):
    """Every way to satisfy an SPDX expression, each a set of license ids."""
    tokens = re.findall(r"\(|\)|[^\s()]+", expr.replace("/", " OR "))
    pos = 0

    def parse_or():
        nonlocal pos
        alts = parse_and()
        while pos < len(tokens) and tokens[pos] == "OR":
            pos += 1
            alts = alts + parse_and()
        return alts

    def parse_and():
        nonlocal pos
        alts = parse_atom()
        while pos < len(tokens) and tokens[pos] == "AND":
            pos += 1
            right = parse_atom()
            alts = [a | b for a in alts for b in right]
        return alts

    def parse_atom():
        nonlocal pos
        tok = tokens[pos]
        pos += 1
        if tok == "(":
            alts = parse_or()
            pos += 1  # ")"
            return alts
        if pos < len(tokens) and tokens[pos] == "WITH":
            pos += 2  # an exception only relaxes the license it attaches to
        return [frozenset([tok])]

    return parse_or()


def rank(license_id):
    return PREFERENCE.index(license_id) if license_id in PREFERENCE else len(PREFERENCE)


def reachable(crate_dir, target, packages):
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target],
        cwd=os.path.join(ROOT, crate_dir),
    ))
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    packages.update({p["id"]: p for p in meta["packages"]})
    stack, seen = [meta["resolve"]["root"]], set()
    while stack:
        node = stack.pop()
        if node in seen:
            continue
        seen.add(node)
        for dep in nodes[node]["deps"]:
            # Normal dependencies only. Build scripts and dev-dependencies
            # never reach the shipped binary.
            if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    return seen


def crate_closure():
    packages, unique = {}, {}
    for crate_dir, targets in LOCKFILES:
        for target in targets:
            for i in reachable(crate_dir, target, packages):
                pkg = packages[i]
                # Path crates (anicat-core, anicat-server) are ours and have no
                # source. Keyed by name and version, not package id: the two
                # lock files resolve from different directories, and the ids of
                # their shared crates are equal only while both happen to agree
                # on every source string.
                if pkg["source"] is not None:
                    unique.setdefault((pkg["name"], pkg["version"]), pkg)
    # Sorted, because the first crate in a group decides which copy of a
    # shared text is printed, and a set's order changes with every process.
    # Unsorted, two runs back to back printed differently indented copies of
    # the Apache license and `--check` failed on an unchanged lock file.
    return [unique[k] for k in sorted(unique)]


def crate_texts(pkg):
    directory = os.path.dirname(pkg["manifest_path"])
    if not os.path.isdir(directory):
        sys.exit(f"{pkg['name']} {pkg['version']} is not in the cargo registry; run `cargo fetch --locked` in core/ and server/ first")
    files = sorted(f for f in os.listdir(directory) if LICENSE_FILE.match(f) and os.path.isfile(os.path.join(directory, f)))
    expr = pkg.get("license") or ""
    chosen = min(spdx_alternatives(expr), key=lambda alt: sorted(rank(i) for i in alt)) if expr else frozenset()

    notices = [f for f in files if f.upper().startswith("NOTICE")]
    licenses = [f for f in files if f not in notices]
    matching = [f for f in licenses if any(FILE_HINTS.get(i, "\0") in f.upper() for i in chosen)]
    picked = (matching or licenses) + notices
    if picked:
        return chosen, [read(os.path.join(directory, f)) for f in picked]

    fallback = NO_FILE_FALLBACK.get(pkg["name"])
    if fallback:
        return chosen, [read(os.path.join(TEXTS, f)) for f in fallback]
    # Failing is the point: a crate listed with a license name and no text is
    # exactly the gap this file exists to close.
    sys.exit(f"{pkg['name']} {pkg['version']} ({expr}) ships no license file; add it to NO_FILE_FALLBACK")


def normalized(text):
    return re.sub(r"\s+", " ", text).strip()


def anime4k_notice():
    shaders = os.path.join(UI_RESOURCES, "Shaders")
    for name in sorted(os.listdir(shaders)):
        lines = read(os.path.join(shaders, name)).split("\n")
        if lines and lines[0].strip() == "// MIT License":
            header = []
            for line in lines:
                # The header's paragraphs are separated by empty lines, not
                # empty comments; stopping at the first one kept only the
                # words "MIT License" and dropped the notice itself.
                if line.strip() and (not line.startswith("//") or line.startswith("//!")):
                    break
                header.append(line[2:].lstrip())
            return "\n".join(header).strip("\n")
    sys.exit("no Anime4K shader carries its MIT header any more")


def add_library(add, lib):
    add("\n" + rule(f"{lib['name']} {lib['version']}", "-"))
    add(f"License: {lib['license']}")
    add("Source: " + "\n        ".join(lib["source"]))
    if lib.get("note"):
        add(lib["note"])
    for text in lib["texts"]:
        add("\n" + read(os.path.join(TEXTS, text)))


def build():
    out = []
    add = out.append

    add(rule("Anicat third-party notices"))
    add("""
Generated by scripts/generate-third-party-notices.py. Edit that script or
scripts/third-party-licenses/, never this file.

Anicat is free software, licensed under the GNU General Public License,
version 3 (full text in section 6). Its source code, at the exact revision of
every release (each release is a git tag), is at
https://github.com/bonkedbythonk/anicat.

Corresponding source for the GPL and LGPL libraries in section 1: MPVKit 1.0.0
at https://github.com/mpvkit/MPVKit/tree/1.0.0 names the upstream revision
each library is built from, and the build repositories listed with each
library publish the scripts and archives used. If any of that source stops
being available, open an issue at https://github.com/bonkedbythonk/anicat/issues
and it will be provided.

The license texts for section 1 were taken from each project's current
repository rather than from the pinned release. Libraries statically included
inside those prebuilt frameworks that MPVKit does not list separately are
covered by the license text reproduced for the library that includes them.
System libraries the app links from macOS or iOS itself (libc++, zlib,
libxml2, libbz2, libiconv, expat) are part of the operating system and not
distributed with Anicat.

Section 1 is the player in the Apple builds and section 5 the player in the
Windows zip; each ships only with its own platform, as do the fonts and
shaders in sections 2 and 3. Corresponding source for section 5: the mpv
release tag and shinchiro's mpv-winbuild-cmake build scripts linked there,
which fetch every library from the upstream listed with it. The same issue
tracker applies if any of that becomes unavailable.""")

    add("\n\n" + rule("1. Native libraries (MPVKit 1.0.0, MPVKit-GPL product)"))
    for lib in NATIVE:
        add_library(add, lib)

    add("\n\n" + rule("2. Fonts"))
    add("\nGeist and IBM Plex Mono, under the SIL Open Font License, version 1.1.\n")
    add(read(os.path.join(UI_RESOURCES, "Fonts", "OFL.txt")))

    add("\n\n" + rule("3. Anime4K shaders"))
    add("\nhttps://github.com/bloc97/Anime4K")
    add("The AutoDownscalePre shaders are dedicated to the public domain (Unlicense);")
    add("the rest carry this notice:\n")
    add(anime4k_notice())

    add("\n\n" + rule("4. Rust crates (core/Cargo.lock, and server/Cargo.lock for Windows)"))
    add("\nWhere a crate offers a choice of licenses, the one reproduced is the one taken.")
    groups = {}
    for pkg in crate_closure():
        chosen, texts = crate_texts(pkg)
        key = tuple(normalized(t) for t in texts)
        entry = groups.setdefault(key, {"texts": texts, "crates": []})
        entry["crates"].append((pkg["name"], pkg["version"], " AND ".join(sorted(chosen)) or "see text"))
    for group in sorted(groups.values(), key=lambda g: sorted(g["crates"])[0]):
        crates = sorted(set(group["crates"]))
        add("\n" + "-" * 78)
        for name, version, license_id in crates:
            add(f"{name} {version} ({license_id})")
        add("")
        add("\n\n".join(group["texts"]))

    add("\n\n" + rule("5. mpv.exe (Windows zip only, shinchiro build of mpv 0.41.0)"))
    for lib in WINDOWS_NATIVE:
        add_library(add, lib)

    add("\n\n" + rule("5b. mpv scripts and fonts (Windows zip only)"))
    for lib in WINDOWS_SCRIPTS:
        add_library(add, lib)

    add("\n\n" + rule("6. Full license texts"))
    for title, path in FULL_TEXTS:
        add("\n" + rule(title, "-") + "\n")
        add(read(path))

    return "\n".join(out) + "\n"


def main():
    args = sys.argv[1:]
    out = OUT
    if "--output" in args:
        # To try a change without overwriting the committed file, which is the
        # one --check compares against and the one both packagers ship.
        out = os.path.abspath(args[args.index("--output") + 1])
    text = build()
    if "--check" in args:
        current = read(OUT) + "\n" if os.path.exists(OUT) else ""
        if current != text:
            sys.exit(f"{os.path.relpath(OUT, ROOT)} is stale: run python3 scripts/generate-third-party-notices.py and commit it")
        print("third-party notices up to date")
        return
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8") as f:
        f.write(text)
    print(f"wrote {out} ({len(text) // 1024} KB, {len(text.splitlines())} lines)")


if __name__ == "__main__":
    main()
