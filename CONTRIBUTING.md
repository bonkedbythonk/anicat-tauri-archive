# Contributing to Anicat

Thanks for wanting to help. Anicat is maintained by one person, so the most
useful contributions are clear bug reports and small, focused pull requests.
Everyone taking part is expected to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Reporting a bug

Open an issue with the **Bug report** template. The things that make a report
fixable:

- **The app version** (Anicat > About Anicat) and your macOS version.
- **Which mode** you were in: Anime, Manga, Light novels, or Films and TV.
- **The log.** Settings > Advanced > Reveal Log File opens
  `~/Library/Logs/Anicat/anicat.log`. Attach the part around the problem;
  the previous three launches are kept as `anicat.log.1` to `.3`.
- **Nothing loads at all?** AniList has periodic outages during which it
  refuses requests from signed-out users. Check
  [AniList's status](https://anilist.co) before reporting.

Reports from [test builds](README.md#test-builds) are especially welcome:
they are how bugs get caught before a version reaches everyone. A test build's
version ends in `-beta.N`; put the whole thing in the report.

Security problems do not go in public issues; see [SECURITY.md](SECURITY.md).

## Suggesting a feature

Open an issue with the **Feature request** template and describe the problem
you want solved, not only the solution. Requests to add a specific piracy site
or to share where content can be found will be closed.

## Development setup

The app is a Swift package (`AnicatApple/`, SwiftUI with libmpv in-process)
over a Rust engine (`core/`), joined by UniFFI. `ARCHITECTURE.md` has the full
picture. Build prerequisites and commands are in the README under
[Building from Source](README.md#building-from-source). In short:

```bash
bash scripts/build-xcframework.sh   # after cloning, and after any change to core/src/ffi.rs
cd AnicatApple && swift build --product Anicat
```

Stale bindings link fine and then call the wrong symbols at runtime, so rerun
`build-xcframework.sh` whenever `core/src/ffi.rs` or a type it exports changes.

## Before opening a pull request

**Open pull requests against `native-swift`, not `master`.** `native-swift` is
the development branch. `master` only moves when a release ships, and the
install script is downloaded from it, so anything merged there reaches users
straight away. GitHub selects `master` by default; change the base branch when
you open the pull request.

Run the checks CI runs:

Run the checks CI runs:

```bash
cd core && cargo test --lib && cargo clippy --lib --tests -- -D warnings
cd AnicatApple && swift build --product Anicat && swift test
```

Three Swift test suites query the live AniList API and fail during its
outages; that is not your change. If you touched a shared view, also confirm
it still compiles for the iOS Simulator:

```bash
cd AnicatApple && swift build --product AnicatUI --triple arm64-apple-ios18.0-simulator --sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)" --scratch-path .build-ios
```

If your change touches `Cargo.lock`, regenerate the third-party notices with
`python3 scripts/generate-third-party-notices.py`; never edit the output by
hand.

## Style

- **English only**, in code, comments, UI strings and commit messages.
- **No emojis** anywhere.
- **Comments explain the failure, not the code.** A non-obvious constant,
  guard, or ordering carries a comment saying what broke without it, ideally
  with the measurement. A comment that restates the line is worse than none.
- **Commits follow [Conventional Commits](https://www.conventionalcommits.org)**
  with a scope, as in `fix(player): ...` or `feat(cinema): ...`. Squash
  "fix typo" and "try CI again" commits into the commit they belong to.
- Match the code around you: naming, structure, and how much is commented.

## License

Anicat is licensed under the [GNU General Public License v3.0](LICENSE). By
submitting a contribution you agree that it is licensed under the same terms.
