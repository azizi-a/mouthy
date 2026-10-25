# Mouthy

Mouthy is a free, open-source text-to-speech app for macOS that works offline. Select text in any app, press a hotkey, and the Kokoro neural voice reads it aloud on your Mac; press the hotkey again to stop. Playback starts as soon as the first sentence is ready, and your clipboard is put back the way it was. A local OpenAI-compatible speech server can be used instead.

**Status:** 0.4.1, for Apple Silicon Macs with macOS 13.4 or later. Mouthy is not signed with an Apple Developer ID or notarised, and does not update itself. Intel Macs are not supported, because ONNX Runtime, which runs the voice, has no prebuilt library for them. Windows and Linux builds compile but are untested.

## Install

With [Homebrew](https://brew.sh/):

```bash
brew install --cask azizi-a/mouthy/mouthy
```

The first time, Homebrew asks you to trust the third-party tap. It may ask for an administrator password to copy Mouthy into /Applications. `brew upgrade --cask mouthy` installs new releases, and `brew uninstall --zap --cask mouthy` also deletes the voices Mouthy downloaded and its settings.

Or download `Mouthy_0.4.1_aarch64.dmg` from [GitHub Releases](https://github.com/azizi-a/mouthy/releases), open it and drag Mouthy to Applications.

Either way, macOS blocks the first launch because Mouthy is not notarised. Click Done, then open System Settings → Privacy & Security and click **Open Anyway** (on macOS 14 and earlier you can instead right-click Mouthy and choose Open). Or clear the download flag before launching:

```bash
xattr -dr com.apple.quarantine /Applications/Mouthy.app
```

## First run

1. **Grant Accessibility.** Mouthy copies your selection by sending ⌘C, which needs the Accessibility permission. It asks on first launch; enable Mouthy in System Settings → Privacy & Security → Accessibility.
2. **Download the voice.** Open Settings → Models and click **Download** next to the status line. It fetches the int8 model (92 MB) and the `af_alloy` voice (0.5 MB) from Hugging Face. Until then the hotkey shows "Offline voice files are missing", and its Open Settings button goes to the same page.

After that Mouthy needs no network. Mouthy is only ad-hoc signed, so an update can invalidate the Accessibility grant even though Mouthy still shows as enabled. If the hotkey stops working, reset the grant and allow it again when asked:

```bash
tccutil reset Accessibility com.azizi.mouthy
```

## Usage

Select text in any app and press **⌥⎋** (Option+Escape). Mouthy reads it aloud and shows a small overlay while speaking. Press ⌥⎋ again, or click × on the overlay, to stop. If nothing is selected, or the voice cannot speak, the overlay says so.

Press **⌥⇧⎋** (Option+Shift+Escape) to read the clipboard instead: the text you last copied, with nothing copied from the selection. Either hotkey stops speech. Clipboard text that a password manager marked as secret is not read.

Both hotkeys can be changed in Settings → General. The tray menu has Speak Selection, Stop, Settings and Quit.

### Command line and signals

A running Mouthy can be controlled from scripts or other hotkey tools:

```bash
/Applications/Mouthy.app/Contents/MacOS/mouthy --speak-selection   # speak, or stop if speaking
/Applications/Mouthy.app/Contents/MacOS/mouthy --speak-clipboard   # speak the clipboard, or stop if speaking
/Applications/Mouthy.app/Contents/MacOS/mouthy --stop
pkill -USR2 -x mouthy   # speak selection
pkill -USR1 -x mouthy   # stop
```

Call the binary directly: `open -a Mouthy --args …` drops the arguments. With no running instance, `--speak-selection`, `--speak-clipboard` and `--stop` just launch the app. Launch flags: `--start-hidden` (no main window; tray icon only) and `--no-tray` (no tray icon; closing the window quits).

## Voices

Mouthy runs the [Kokoro-82M](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX) voice itself, with no server. Settings → General → Speech lists the voices you have downloaded, each with Preview and Use. Turn on **Show uninstalled voices** to see all 54 and download any of them (0.5 MB each). The Blend row mixes the voice in use with a second voice on a slider, and Other… takes any voice name or a blend of up to four voices, such as `bm_lewis(70)+bm_george(30)`.

Mouthy reads all text as English. Voices starting with `b` get British pronunciations, all others American, and a blend reads with its first voice's. 28 voices are English; the rest read English with their language's accent and cannot read their own language.

Settings → Models manages the files: the int8 model, the fp32 model (326 MB; about twice as slow) and the voices, each of which you can preview, use or delete. Downloads show progress, can be cancelled and pick up where they stopped, and each file is checked against its published SHA-256. The files live in `~/Library/Application Support/com.azizi.mouthy/models/kokoro-v1.0/`.

Mouthy uses about 350 MB of memory once the voice has loaded, and up to about 1.1 GB after a sentence of 400 characters or more; it keeps that memory while it runs. Kokoro-82M and its voices are released under the Apache License 2.0.

### Better pronunciation of unknown words

Words the voice's dictionary lacks, such as many names and product terms, are spelled out letter by letter. If you install [espeak-ng](https://github.com/espeak-ng/espeak-ng) (`brew install espeak-ng`) and turn on Settings → General → **Pronounce unknown words with espeak-ng**, Mouthy asks it how to say them instead. Each new unknown word delays speech a little (20–50 ms on an M4 Pro, never more than half a second). espeak-ng is GPL-3 software: Mouthy does not bundle or download it, and only runs your installed copy as a separate program.

## Using a speech server instead

Set Settings → General → Speech Engine to **HTTP server** to send the text to a local OpenAI-compatible speech server with `POST /v1/audio/speech` and `pcm` output, such as [Kokoro-FastAPI](https://github.com/remsky/Kokoro-FastAPI). Mouthy expects it at `http://127.0.0.1:8880` with the voice `af_alloy` by default; the voice list then shows the server's voices. Check the server is up:

```bash
curl http://127.0.0.1:8880/v1/audio/voices
```

Plain `http://` is unencrypted, so use `https://` for a server on another machine. The Local engine sends nothing anywhere; only the Models page's downloads use the network.

## Settings

Open Settings from the tray. Besides the hotkeys and voices above, General sets launch at login, start hidden, the overlay, speed (0.5×–2×), volume and the output device. Advanced hides the tray icon and, under experimental features, changes the shortcut backend. About sets the language and theme, opens the app data and log folders, and opens the third-party licences.

## Known issues

- **⌥⎋ clashes with macOS "Speak selection",** which uses the same key by default. If it is on in System Settings → Accessibility → Spoken Content, turn it off or change either hotkey.
- **Nothing is selected after a drag in Claude Code, nvim and other terminal programs** that turn on mouse reporting, which takes the drag from the terminal. Hold Option while dragging in iTerm2, or Shift in Zed and most other terminals, or copy inside the program (in nvim, yank to `+`) and press ⌥⇧⎋. Lines the program wrapped are joined again before reading.
- **VS Code copies the current line when nothing is selected,** so ⌥⎋ with no selection reads that line.
- **The first words take longer** when a long first sentence has no early comma.
- **A clipboard larger than 16 MB is not restored;** it keeps the copied selection instead.
- **Kokoro-FastAPI can hang.** Some input, such as Chinese text with an English voice, leaves the server listening but never replying, so speech fails with a timeout. Restart the server.

### The hotkey pauses (macOS Secure Input)

While a password field has focus, or an app such as Terminal has Secure Keyboard Entry on, macOS hides key presses from the default shortcut backend. The hotkeys stop responding for about three seconds, then Mouthy registers them another way and they work again. Meanwhile a hotkey that uses fn does not work, one that uses one side's modifier answers to either side, and shortcuts cannot be changed; Mouthy shows a warning, naming the app holding Secure Input when it can. The Tauri shortcut backend (Settings → Advanced → Experimental) has no pause, but does not support fn or modifier-only hotkeys.

## Build from source

You need [Rust](https://rustup.rs/) 1.92 or newer, [Bun](https://bun.sh/) and the Xcode Command Line Tools (`xcode-select --install`).

```bash
git clone https://github.com/azizi-a/mouthy.git
cd mouthy
bun install
bun run tauri dev                    # run in development
bun run tauri build --bundles app    # build the app
```

The built app is `src-tauri/target/release/bundle/macos/Mouthy.app`. Under `bun run tauri dev`, macOS charges the Accessibility permission to your terminal, so test the hotkeys with the built app. See [BUILD.md](BUILD.md) for more. To test a build by hand, work through [docs/phase3-checklist.md](docs/phase3-checklist.md); [docs/listening-test.md](docs/listening-test.md) checks how text is read, by ear.

## Attribution

Mouthy is derived from [Handy](https://github.com/cjpais/Handy) by CJ Pais (MIT). It is not affiliated with or endorsed by the Handy project. Handy's copyright notice is in [NOTICE](NOTICE).

## License

MIT. See [LICENSE](LICENSE). The licences of the software Mouthy includes are in [src-tauri/resources/THIRD_PARTY_NOTICES.txt](src-tauri/resources/THIRD_PARTY_NOTICES.txt), which ships inside the app and opens from Settings → About.
