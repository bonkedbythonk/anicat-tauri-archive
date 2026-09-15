<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/branding/logo-dark.png">
    <img src="assets/branding/logo.png" alt="Anicat" width="140">
  </picture>
  <h1>Anicat</h1>
  <p><strong>Watch, read, and track anime, manga, light novels and film — a native desktop app powered by AniList.</strong></p>

  <p>
    <img src="https://img.shields.io/github/v/release/bonkedbythonk/anicat?style=flat-square&label=latest" alt="Latest Release">
    <img src="https://img.shields.io/badge/platform-macOS-lightgrey?style=flat-square" alt="Platform">
    <img src="https://img.shields.io/badge/license-GPLv3-blue?style=flat-square" alt="License">
  </p>

  <img src="assets/branding/dashboard.png" alt="Anicat home screen" width="720">
</div>

---

Anicat is an app for your Mac that plays anime, reads manga and light novels,
and keeps your [AniList](https://anilist.co) profile up to date while you do
it. You search for something, press play, and it starts. No browser tabs, no
choosing between sites, nothing to download and wait for first.

Four kinds of thing, all in the one app:

| | Where it comes from | Works today |
|---|---|---|
| **Anime** | Fan-subtitled releases, which start playing while the rest is still arriving | Yes |
| **Manga** | MangaDex, and a second site for titles MangaDex cannot show | Yes |
| **Light novels** | Official volumes, and Japanese web novels | Yes |
| **Films and TV** | The same way anime works | Yes |

Video plays inside the app itself, the way a normal video player does. Nothing
finishes downloading before it starts: the file arrives while you watch it, and
skipping ahead pulls that part down next.

Everything you watch or read is reported back to AniList automatically, so your
lists, progress and scores stay right without you touching them.

> **Disclaimer:** Anicat hosts zero content — it scrapes publicly accessible third-party sites and streams from public torrent swarms. It is for educational and personal use only, and use is at your own risk under your local laws. The developer has no affiliation with any content provider and is not responsible for how the app is used. See [DISCLAIMER.md](DISCLAIMER.md) for the full text.

---

## Table of Contents

- [Install](#install)
- [Setting it up](#setting-it-up)
- [Features](#features)
- [Screenshots](#screenshots)
- [Building from Source](#building-from-source)
- [Dependencies](#dependencies)
- [Project history](#project-history)
- [Legal](#legal)
- [License](#license)

---

## Install

**You need:** a Mac with Apple silicon — any Mac sold since late 2020, or
anything whose chip is called M1, M2, M3 or newer — running macOS 15 (Sequoia)
or later. Click the Apple menu, then About This Mac, if you are not sure.

Open the **Terminal** app (press <kbd>Cmd</kbd>+<kbd>Space</kbd>, type
`terminal`, press Return), paste this line, and press Return:

```bash
curl -fsSL https://raw.githubusercontent.com/bonkedbythonk/anicat/master/scripts/install_macos.sh | bash
```

It downloads the latest version, puts Anicat in your Applications folder and
opens it. That is the whole install.

This is the recommended way, and not only because it is one step: macOS blocks
apps it cannot verify, and downloading by hand means clearing that block
yourself through System Settings. The installer takes care of it, so Anicat
just opens.

<details>
<summary>Or install it by hand</summary>

1. Download `Anicat-<version>-macos-arm64.dmg` from the
   [Releases page](https://github.com/bonkedbythonk/anicat/releases). The
   `.zip` beside it holds the same app for anyone who would rather unpack it
   themselves.
2. Open it and drag Anicat onto the Applications folder in the same window.
3. Open your Applications folder and double-click Anicat. macOS refuses and
   says it cannot check the app for malicious software. Click Done.
4. Open **System Settings**, go to **Privacy & Security**, scroll to the
   bottom, and click **Open Anyway** next to the line about Anicat. Confirm
   with your password or Touch ID, then click Open Anyway once more.

Steps 3 and 4 are only needed the first time.

Apple charges a yearly fee to have an app certified and this one has not paid
it, which is all that warning means. Right-clicking the app and choosing Open
used to skip it; Apple removed that shortcut in macOS Sequoia, and Open Anyway
in Settings replaced it. To skip the whole dance in one line instead:

```bash
xattr -dr com.apple.quarantine /Applications/Anicat.app
```

</details>

Anything on the Releases page numbered 5.x is the old version of Anicat, which
was a different program that happened to share the name. Start with the newest
release; installing it replaces an older one in place, with nothing to
uninstall first.

### Test builds

New versions go out as test builds before they become the regular release.
They have the newest features and fixes, and they have not been used for long,
so expect bugs. If you would like to help find them, install the test line
instead:

```bash
curl -fsSL https://raw.githubusercontent.com/bonkedbythonk/anicat/master/scripts/install_macos.sh | bash -s -- --beta
```

A test build's version ends in `-beta` followed by a number (Anicat > About
Anicat shows it). It tells you when a newer test build or a regular release is
out, and running the regular command above at any time moves you back to the
regular release. Please report what you find with the
[Bug report](https://github.com/bonkedbythonk/anicat/issues/new/choose)
template, including the version and the log it asks for. Test builds are the
ones marked Pre-release on the
[Releases page](https://github.com/bonkedbythonk/anicat/releases).

---

## Setting it up

The first time you open Anicat it asks you to connect your
[AniList](https://anilist.co) account. AniList is a free website that keeps
track of what you have watched and read; Anicat uses it as your library.

1. Anicat opens AniList in your browser and asks you to approve it.
2. AniList sends you to a page whose address contains a long code. Copy that
   whole address and paste it back into Anicat.
3. Your lists appear, and the home screen fills up.

That is the whole setup. From then on, anything you watch or read updates
AniList on its own.

You can skip this and still watch and read everything — you just will not have
a library, and nothing gets tracked. Your AniList login stays on this Mac and
is never sent anywhere except AniList itself.

---

## Features

**Watching**

- **Up Next** — everything you are part-way through, in one list, with a
  "Pick for me" button when you cannot decide.
- **Press play and it plays.** Anicat finds the episode itself and starts it
  within a few seconds. There is no list of mirrors to pick from and no file to
  download first.
- **It remembers where you stopped**, plays the next episode when one finishes,
  and can skip openings and endings for you.
- **Sharper picture** — an optional upscaler that makes older or lower-quality
  episodes look better on a big screen.
- **Mini player** — shrink the video into the corner and keep browsing.
- **Subtitles and dubs** — pick any audio or subtitle track the release
  includes, and tell Anicat you prefer dubs so it looks for one first.

**Reading**

- **Manga** — one page, two pages, or a continuous scroll; left-to-right or
  right-to-left; your place is saved and sent to AniList.
- **Light novels** — official volumes and Japanese web novels, with control
  over the typeface and size. A volume can be saved for offline reading or
  exported as an ebook file for a Kindle or Kobo.

**Keeping track**

- **Your AniList library** — every list, as covers or as a table, editable
  without leaving the app.
- **Automatic progress** — an episode counts as watched once you pass 85% of
  it. Scores and list changes sync both ways.
- **Schedule** — what airs this week, either everything or just your shows.
- **History and downloads** — what you have watched, and episodes kept for
  offline playback.

**Extras**

- **Films and TV** as well as anime, from the same app.
- **Carry on across Macs** — start an episode on one Mac and pick it up on
  another.
- **Discord** can show what you are watching. Off unless you turn it on.
- **Keyboard shortcuts** for everything, with a cheat sheet on `?`.

<details>
<summary>The same list, for people who want the technical version</summary>

- **Playback** — libmpv drawn inside the window through Metal, at the display's
  full refresh rate. Anime4K upscaling, AniSkip intro and outro skip keyed to
  the file's real length, resume position, auto-next with the next episode
  preloaded at 75%, a corner mini-player, sideways mode for a rotated screen,
  and the display kept awake while a stream plays.
- **Sources** — candidates are gathered from SubsPlease, AnimeTosho, Nyaa and
  SeaDex in one pass, the best two raced against each other, and the release
  that won an episode is remembered and tried first next time. Playback is a
  local HTTP range server over the torrent, so a seek moves the download.
- **Player info popover** — audio and subtitle tracks listed by language and
  title, a Sub/Dub switch that keeps full subtitles, a release switcher that
  resumes at the same position, speed, and an optional keyboard backlight
  dimmer for night watching.
- **Detail pages** — episodes with thumbnails and air dates, cast with in-app
  character, voice actor and staff pages, relations and recommendations,
  AniList forum threads, browser-style back and forward including a two-finger
  swipe, a poster that morphs out of the card you opened, and a hero banner
  that settles into a compact header as you scroll.
- **Films and TV** — a TMDB-backed catalog beside the AniList one. A film is
  matched on title and year and an episode on SxxEyy, neither of which the
  anime search has a notion of, so they take their own path into the same
  player.
- **Manga** — MangaDex first, MangaKatana for titles MangaDex has matched but
  cannot serve.
- **Light novels** — official volumes sliced out of one HTML page per volume,
  plus ncode.syosetu.com web novels; offline storage as JSON and EPUB export
  written without a zip dependency.
- **AniList sync** — progress reported continuously while you watch, watched at
  85%, inline list editing, Planning shelves on the manga and novel pages.
- **Continuity** — Handoff of playback and reading between Macs on the same
  Apple ID, Bonjour discovery of other instances.

</details>

---

## Screenshots

<div align="center">
  <img src="assets/branding/dashboard.png" alt="Home screen" width="720">
  <br><br>
  <img src="assets/branding/detail.png" alt="Anime detail page" width="720">
  <br><br>
  <img src="assets/branding/manga.png" alt="Manga shelves" width="720">
  <br><br>
  <img src="assets/branding/stats.png" alt="Watch statistics" width="720">
</div>

The screenshots come from a build run with `ANICAT_SCREENSHOT_MODE=1`, which
swaps the personal data (lists, history, profile, statistics) for fixtures built
from the trending catalog — they show the layout, not anyone's watch history.

---

## Building from Source

**Prerequisites:**

- macOS 15 or later on Apple silicon, with Xcode's command line tools (Swift 6)
- [Rust](https://rustup.rs/) stable, with the Apple targets: `rustup target add aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim`
- No mpv install needed: libmpv and FFmpeg come from [MPVKit](https://github.com/mpvkit/MPVKit) as SwiftPM binary dependencies (about 1.7 GB of xcframeworks on first resolve)

```bash
git clone https://github.com/bonkedbythonk/anicat.git
cd anicat

# Compile the Rust engine for every Apple target and generate the Swift
# bindings. Once after cloning, and again after any change to core/src/ffi.rs.
bash scripts/build-xcframework.sh

# Build and run
cd AnicatApple
swift build --product Anicat
bash dev-run.sh        # copies the binary into dist/Anicat.app and opens it
```

Useful while working on it:

```bash
cd AnicatApple && swift test
cd core && cargo test --lib && cargo clippy --lib --tests -- -D warnings
```

`scripts/package-anicat-macos-app.sh release` produces a standalone `.app`
in `AnicatApple/dist/`; add `install` to replace `/Applications/Anicat.app`
with it. `bash AnicatApple/dev-run.sh` rebuilds a debug copy in `dist/` and
relaunches it, and never touches the installed one.

---

## Dependencies

| Dependency | Purpose |
|---|---|
| [AniList](https://anilist.co) | Library, tracking, search, profile data for anime, manga and novels |
| [TMDB](https://themoviedb.org) | Catalog for film and TV |
| [mpv](https://mpv.io) via [MPVKit](https://github.com/mpvkit/MPVKit) | Media playback, libmpv linked statically, drawn through Metal |
| [librqbit](https://github.com/ikatson/rqbit) | Embedded torrent engine |
| [MangaDex](https://mangadex.org), [MangaKatana](https://mangakatana.com) | Manga chapters |
| [Syosetu](https://syosetu.com) | Web novels |
| [AniSkip](https://api.aniskip.com) | Intro and outro timestamps |
| [UniFFI](https://mozilla.github.io/uniffi-rs/) | Rust to Swift bindings |

---

## Project history

Anicat has been rewritten from the ground up three times. Each rewrite threw
away the UI layer and kept the idea: one place to find, play and track anime,
with AniList as the source of truth.

| When | What it was |
|---|---|
| **May 2026** | A Python command-line tool, playback handed to IINA. Built on the foundations of [Viu](https://github.com/viu-media/viu) and refined for macOS. |
| **May 2026** | A FastAPI dashboard with a Next.js front end over the same Python core. |
| **June 2026** | A packaged desktop app: CI for macOS and Windows, mpv bundled rather than assumed. The first build that could be handed to someone. |
| **June 2026** · v4.0.0 | The first full rewrite. Tauri v2, Vite and React over a Rust backend, replacing Next.js and the monolithic Python sidecar. |
| **September 2026** · v6.0.0 | The second, and the current one. A headless Rust engine that knows nothing about a UI, with SwiftUI and AppKit over it and libmpv in-process. |

That is also why the version number is as high as it is: it counts the
project, not the app in front of you. The native build reached v6.0.0 because
the Tauri one had already run from v4.0.0 to v5.8.0 in four months.

Dated commit by commit in [HISTORY.md](HISTORY.md).

---

## Legal

Anicat is for educational and personal use only. See [DISCLAIMER.md](DISCLAIMER.md) and [SECURITY.md](SECURITY.md).

## License

[GNU General Public License v3.0](LICENSE)
