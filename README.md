# Mouthy

Mouthy is a free, open-source desktop text-to-speech app for macOS. Highlight text in any app, press a global hotkey, and hear it read aloud by a neural voice; press the hotkey again to stop. Playback starts as soon as the first sentence is ready, and your clipboard is put back the way it was.

**Status:** Phase 1. Mouthy works against a local OpenAI-compatible speech server. An offline engine that runs inside the app is planned for Phase 2 (see [PLAN.md](PLAN.md)). Windows and Linux builds compile but are untested.

## Requirements

- An Apple Silicon Mac with macOS 13.4 or later
- A local OpenAI-compatible speech server with `POST /v1/audio/speech` and `pcm` output, such as [Kokoro-FastAPI](https://github.com/remsky/Kokoro-FastAPI). Mouthy expects it at `http://127.0.0.1:8880` with the voice `bm_lewis` by default; both can be changed in Settings.

Check the server is up:

```bash
curl http://127.0.0.1:8880/v1/audio/voices
```

## Setup

Install [Rust](https://rustup.rs/) (stable), [Bun](https://bun.sh/) and the Xcode Command Line Tools (`xcode-select --install`), then:

```bash
git clone https://github.com/azizi-a/mouthy.git
cd mouthy
bun install
bun run tauri dev                    # run in development
bun run tauri build --bundles app    # build the app
```

The built app is `src-tauri/target/release/bundle/macos/Mouthy.app`; copy it to `/Applications` if you like. `--bundles app` skips the DMG step, which needs Finder automation permission. See [BUILD.md](BUILD.md) for more.

## First run

Mouthy copies your selection by sending ⌘C, which needs the Accessibility permission. On first launch it asks for it; enable Mouthy in System Settings → Privacy & Security → Accessibility.

- Under `bun run tauri dev`, macOS charges the permission to your terminal, not to Mouthy. Test the hotkey with the built app.
- Local builds are ad-hoc signed, so a rebuild can invalidate the grant even though Mouthy still shows as enabled. Reset it and grant again:

  ```bash
  tccutil reset Accessibility com.azizi.mouthy
  ```

## Usage

Select text in any app and press **⌥⎋** (Option+Escape). Mouthy reads it aloud and shows a small overlay while speaking. Press ⌥⎋ again, or click × on the overlay, to stop. If nothing is selected the overlay says so; if the server cannot be reached it shows the error with an Open Settings button.

The tray menu has Speak Selection, Stop, Settings and Quit.

### Command line and signals

A running Mouthy can be controlled from scripts or other hotkey tools:

```bash
/Applications/Mouthy.app/Contents/MacOS/mouthy --speak-selection   # speak, or stop if speaking
/Applications/Mouthy.app/Contents/MacOS/mouthy --stop
pkill -USR2 -x mouthy   # speak selection
pkill -USR1 -x mouthy   # stop
```

Call the binary directly: `open -a Mouthy --args …` drops the arguments. With no running instance, `--speak-selection` and `--stop` just launch the app. Launch flags: `--start-hidden` (no main window; tray icon only) and `--no-tray` (no tray icon; closing the window quits).

## Settings

Open Settings from the tray. The General page has three groups:

- **General:** hotkey, launch at login, start hidden, overlay style and position
- **Speech:** server URL, voice (a list when the server offers `/v1/audio/voices`, otherwise free text), speed (0.5×–2×), volume
- **Sound:** output device
- **Advanced page:** tray icon, experimental features (shortcut backend)
- **About page:** language, theme, app data and log folders

Mouthy sends the selected text to the server URL you set. Plain `http://` is unencrypted, so use `https://` for a server on another machine.

## Known issues

- **⌥⎋ clashes with macOS "Speak selection",** which uses the same key by default. If it is on in System Settings → Accessibility → Spoken Content, turn it off or change either hotkey.
- **Kokoro-FastAPI can hang.** Some input, such as Chinese text with an English voice, leaves the server listening but never replying, so speech fails with a timeout. Restart the server.
- **VS Code copies the current line when nothing is selected,** so ⌥⎋ with no selection reads that line.
- **Clipboard:** rich content (images, files, formatted text) is restored after each copy. A clipboard larger than 16 MB is not restored; it keeps the copied selection instead.
- **Windows and Linux are untested,** and `Alt+Escape` may already be taken by the window manager there.

### The hotkey pauses (macOS Secure Input)

While a password field has focus, or an app such as Terminal has Secure Keyboard Entry on, macOS hides key presses from the default shortcut backend. ⌥⎋ stops responding for about three seconds, then Mouthy registers it another way and it works again. Hotkeys that use fn or one side's modifier cannot be covered that way, and shortcuts cannot be changed meanwhile; Mouthy shows a warning naming the app holding Secure Input when it can. Leave the password field, turn off Secure Keyboard Entry, or quit that app. The Tauri shortcut backend (Settings → Advanced → Experimental) has no pause, but does not support fn or modifier-only hotkeys.

## Attribution

Mouthy is derived from [Handy](https://github.com/cjpais/Handy) by CJ Pais (MIT). It is not affiliated with or endorsed by the Handy project.

## License

MIT. See [LICENSE](LICENSE).
