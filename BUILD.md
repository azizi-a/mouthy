# Build Instructions

Mouthy targets Apple-Silicon macOS first. Windows and Linux are kept compiling where cheap but are not tested.

Intel and universal macOS builds fail to link: ONNX Runtime's prebuilt archive exists only for Apple Silicon (macOS 13.4+).

## Prerequisites

- [Rust](https://rustup.rs/) 1.92 or newer (ONNX Runtime's `ort` crate needs it)
- [Bun](https://bun.sh/)
- Xcode Command Line Tools: `xcode-select --install`
- For other platforms, the [Tauri prerequisites](https://tauri.app/start/prerequisites/)

## Setup

```bash
git clone git@github.com:azizi-a/mouthy.git
cd mouthy
bun install
```

## Develop

```bash
bun run tauri dev
```

Under `tauri dev`, macOS charges the Accessibility permission to your terminal, not to Mouthy.

## Build

```bash
bun run tauri build --bundles app
```

`--bundles app` skips the DMG step, which needs Finder automation permission. The app lands in `src-tauri/target/release/bundle/macos/Mouthy.app`.

## Release

Releases are unsigned (ad-hoc, no Developer ID). First merge a PR that sets the version in `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml` (then `cd src-tauri && cargo update -p mouthy --offline`), names the dmg in the README and dates the changelog's top block. Then, from an up-to-date `main` (examples use 0.4.0):

```bash
CI=true bun run tauri build
cd src-tauri/target/release/bundle/dmg
shasum -a 256 Mouthy_0.4.0_aarch64.dmg > SHA256SUMS
```

`CI=true` skips the dmg script's Finder automation, so the dmg window keeps the default layout. The dmg and `SHA256SUMS` are in `src-tauri/target/release/bundle/dmg/`.

Tag with a signed annotated tag whose message is the version's changelog block, and push it by name, never with `--tags`. A clone that fetched Handy's tags may hold a local tag of the same name: delete it first, and fetch upstream with `--no-tags`.

```bash
git tag -d v0.4.0                          # only if a Handy tag of that name exists
git tag -s --cleanup=whitespace v0.4.0 -F changelog-block.md   # keeps the ### headings
git push origin v0.4.0
```

Create a draft release with the install notes (Open Anyway, or `xattr -dr com.apple.quarantine /Applications/Mouthy.app`):

```bash
gh release create v0.4.0 --draft --title "Mouthy 0.4.0" --notes-file release-notes.md \
  src-tauri/target/release/bundle/dmg/Mouthy_0.4.0_aarch64.dmg \
  src-tauri/target/release/bundle/dmg/SHA256SUMS
```

After publishing, check that GitHub calls it the latest release; the cask's livecheck relies on it:

```bash
gh api repos/azizi-a/mouthy/releases/latest --jq .tag_name   # v0.4.0
```

### Homebrew cask

Prepare the cask bump alongside the release: in a clone of [azizi-a/homebrew-mouthy](https://github.com/azizi-a/homebrew-mouthy), on a new branch, set `version` and `sha256` in `Casks/mouthy.rb` to the dmg's (the hash is in `SHA256SUMS`) and open a PR to the tap's `main`. Merge it only after the release is published, because draft assets cannot be downloaded. Before publishing, run `brew style` on the cask and `brew audit --cask --strict` (Homebrew audits casks by name only, so copy the cask into a throwaway local tap); after publishing, run `brew audit --cask --online`.

## Troubleshooting

### Accessibility stays "enabled" after a rebuild

Local builds are ad-hoc signed, so each rebuild can get a new code identity that the old Accessibility grant does not cover. Quit Mouthy, reset its grant, reopen it and grant Accessibility again when prompted:

```bash
osascript -e 'tell application id "com.azizi.mouthy" to quit' || true
tccutil reset Accessibility com.azizi.mouthy
open /Applications/Mouthy.app
```
