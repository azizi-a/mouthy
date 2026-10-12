# Build Instructions

Mouthy targets Apple-Silicon macOS first. Windows and Linux are kept compiling where cheap but are not tested.

## Prerequisites

- [Rust](https://rustup.rs/) (latest stable)
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

## Troubleshooting

### Accessibility stays "enabled" after a rebuild

Local builds are ad-hoc signed, so each rebuild can get a new code identity that the old Accessibility grant does not cover. Quit Mouthy, reset its grant, reopen it and grant Accessibility again when prompted:

```bash
osascript -e 'tell application id "com.azizi.mouthy" to quit' || true
tccutil reset Accessibility com.azizi.mouthy
open /Applications/Mouthy.app
```
