# Mouthy

Mouthy is a free, open-source text-to-speech app for macOS and Linux that works offline. Select text in any app, press a hotkey, and the Kokoro neural voice reads it aloud on your computer; press the hotkey again to stop. Playback starts as soon as the first sentence is ready, and your clipboard is put back the way it was. A local OpenAI-compatible speech server can be used instead.

**Status:** 0.6.0, for Apple Silicon Macs with macOS 13.4 or later, and for x86_64 Linux on Wayland (see [Linux](#linux)). Mouthy is not signed with an Apple Developer ID or notarised, and does not update itself. Intel Macs are not supported, because ONNX Runtime, which runs the voice, has no prebuilt library for them. Windows builds compile but are untested.

## Install

With [Homebrew](https://brew.sh/):

```bash
brew install --cask azizi-a/mouthy/mouthy
```

The first time, Homebrew asks you to trust the third-party tap. It may ask for an administrator password to copy Mouthy into /Applications. `brew upgrade --cask mouthy` installs new releases, and `brew uninstall --zap --cask mouthy` also deletes the voices Mouthy downloaded and its settings.

Or download `Mouthy_0.6.0_aarch64.dmg` from [GitHub Releases](https://github.com/azizi-a/mouthy/releases), open it and drag Mouthy to Applications.

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
/Applications/Mouthy.app/Contents/MacOS/mouthy --speak-file reply.txt   # speak a text file, cutting off any speech
/Applications/Mouthy.app/Contents/MacOS/mouthy --speak-file reply.txt --voice 'bm_lewis(70)+bm_george(30)'   # in another voice
/Applications/Mouthy.app/Contents/MacOS/mouthy --speak-file reply.txt --voice ef_dora --lang en   # a Spanish voice reading English
/Applications/Mouthy.app/Contents/MacOS/mouthy --stop
pkill -USR2 -x mouthy   # speak selection
pkill -USR1 -x mouthy   # stop
```

`--speak-file` is for scripts that hand Mouthy each new message: the text goes in a UTF-8 file rather than an argument, so it needs no quoting, and each call cuts off what Mouthy is saying instead of toggling. It reads the first 20,000 characters, takes a relative path from the caller's directory, and stops at a hotkey press or `--stop`.

`--voice` reads that file in another voice than the Settings one, so each script or session can have its own: a voice name or a blend as typed in the voice picker's Other… (quote a blend for the shell). A voice that is not downloaded, or is not a voice name, falls back to the Settings voice and the log says so. With the server engine the voice is trimmed of spaces and otherwise passed to the server unchanged.

`--lang en` reads that file as English whatever the voice's language, and `--lang voice` in the voice's own; without it the Settings reading language applies (see Languages). A script that speaks English text in a rotation of voices should pass `--lang en`, so a Spanish or Japanese voice keeps reading English.

When no Mouthy is running, the command is Mouthy itself and does not return until Mouthy quits. A script should start Mouthy first (`open -a Mouthy`, or the binary with `--start-hidden` in the background), give it a second to start, and then call the binary for each message. That call returns before the running Mouthy reads the file, so the file must outlive it: write each message to a new file and delete old ones later, rather than overwriting or deleting the file straight away.

Call the binary directly: `open -a Mouthy --args …` drops the arguments. With no running instance, `--speak-selection`, `--speak-clipboard` and `--stop` just launch the app, and `--speak-file` launches it and speaks the file. Launch flags: `--start-hidden` (no main window; tray icon only) and `--no-tray` (no tray icon; closing the window quits).

## Voices

Mouthy runs the [Kokoro-82M](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX) voice itself, with no server. Settings → General → Speech lists the voices you have downloaded, each with Preview and Use. Turn on **Show uninstalled voices** to see all 54 and download any of them (0.5 MB each). The Blend row mixes the voice in use with a second voice on a slider, and Other… takes any voice name or a blend of up to four voices, such as `bm_lewis(70)+bm_george(30)`.

A voice reads text in its own language (see Languages), and a blend reads in its first voice's. English voices starting with `b` get British pronunciations, the other English voices American, as does a voice name Mouthy does not recognise.

Settings → Models manages the files: the int8 model, the fp32 model (326 MB; about twice as slow), the voices, each of which you can preview, use or delete, and the Japanese dictionary. Downloads show progress, can be cancelled and pick up where they stopped, and each file is checked against a SHA-256 that Mouthy pins. The files live in `~/Library/Application Support/com.azizi.mouthy/models/`: the Kokoro files in `kokoro-v1.0/`, the Japanese dictionary, unpacked, in `naist-jdic/`.

Mouthy uses about 350 MB of memory once the voice has loaded, and up to about 1.1 GB after a sentence of 400 characters or more; it keeps that memory while it runs. Kokoro-82M and its voices are released under the Apache License 2.0.

### Better pronunciation of unknown words

Mouthy reads many words an English voice's dictionary lacks from the words they are made of, such as changelog, getUserName or tsconfig, and says a few developer words such as sudo and kubectl the way developers do. What it cannot read that way, including most unfamiliar names, is spelled out letter by letter. If you install [espeak-ng](https://github.com/espeak-ng/espeak-ng) (`brew install espeak-ng`) and turn on Settings → General → **Pronounce unknown words with espeak-ng**, Mouthy asks it how to say those words, and any unfamiliar name, instead. Each new word it asks about delays speech a little (20–50 ms on an M4 Pro, never more than half a second). espeak-ng is GPL-3 software: Mouthy does not bundle or download it, and only runs your installed copy as a separate program.

## Languages

English voices read English. The others read their own language:

- **Spanish, French, Italian, Brazilian Portuguese and Hindi** (`ef_*`, `ff_*`, `if_*`, `pf_*`, `hf_*` and their male voices): read as Kokoro learned these languages. Mouthy asks [espeak-ng](https://github.com/espeak-ng/espeak-ng) how to say each sentence, numbers and times included, so these voices need it installed (`brew install espeak-ng`; on Linux see below), whatever the unknown-words setting says. Without it they show a notice instead of speaking, and `--voice` with one of them falls back to the Settings voice unless the file is read as English. English text in one of these voices is read with its language's rules unless Reading Language is English (below).
- **Japanese** (`jf_*`, `jm_*`): reads Japanese, with its numbers in Japanese. English inside Japanese text is read as English, and text with no Japanese in it is read as an American English voice reads it. Kanji need the Japanese dictionary, a 29 MB download on Settings → Models that takes 80 MB once unpacked: the [NAIST Japanese Dictionary](https://github.com/jpreprocess/naist-jdic) (BSD licence) as [jpreprocess](https://github.com/jpreprocess/jpreprocess) publishes it, from `https://github.com/jpreprocess/jpreprocess/releases/download/v0.15.0/naist-jdic-jpreprocess.tar.gz`. With it, Mouthy finds each word and how it is said, so particles such as は sound "wa" and long vowels are long, and dates and counts read as they are said, in digits or kanji (4月 しがつ, 1日 ついたち after a month, 2人 and 二人 ふたり, 三日間 みっかかん, 34日 さんじゅうよっか; in an approximate count such as 二、三日 or 8、9人 the second number reads plainly, に、さんにち). Its limits: a word it does not know written in kanji is skipped; it can choose the wrong reading where context decides (高い山 after 富士山 read with ざん, 空いた read あいた, 机の上 with nothing after it read with じょう); 1日 reads ついたち only right after a month, so on its own or in 4月の1日 it reads いちにち, as a length of time; in an approximate count the first number keeps its plain reading (四、五日 is said しごにち, 七、八日 しちはちにち, 十四、五日 じゅうしごにち), 一、二日 reads いち、ふつか while 1、2日 reads いち、ににち, a space after 、 (二、 三日) makes two numbers, two days with no month before them read as a count (5、6日の両日), a numbered list can read as one (一、二人以上で…), and a count followed by any word starting with とも keeps its own reading, as a date or "both" would (1、2人ともに ふたり); a count that runs into the next word reads as a count (2人三脚 ふたり さん きゃく, where 二人三脚 reads ににんさんきゃく), and the word after a count keeps its own reading (2人暮らし ふたり くらし); a count in kanji with 百 or more keeps the dictionary's reading (二百四人 に ひゃく よん にん); other counters keep the plain number (一冊 いち さつ, 30分 さんじゅう ふん, 二十歳 に じゅう さい rather than はたち), and 四十九日 reads as a count of days, not the memorial's しじゅうくにち; a う written in katakana is not taken as a long vowel (行こウ いこ う); a minus sign is dropped (-5度) and a fraction such as 3/4 reads as two numbers. Without it, Japanese voices read kana only: words run together, は and へ are read "ha" and "he", long vowels follow the spelling, kanji are skipped, and text where more than a fifth of the letters are kanji is not read at all.
- **Mandarin** (`zf_*`, `zm_*`): reads Chinese characters with their tones, as Kokoro learned Mandarin. Words are found with jieba's dictionary, and a word such as 银行 or 重新 takes its reading from a phrase dictionary; other characters take their most common reading. Numbers are read in Chinese: "2024年" as 二零二四年, "3.14" as 三点一四, "50%" as 百分之五十. English inside Chinese text is read as English, and text with no Chinese in it is read as an American English voice reads it. The tone changes of 一 and 不 and of third tones in a row are read only where the phrase dictionary spells them. The dictionaries load once, in about 0.15 s, when a Chinese voice is first used, and add about 40 MB to Mouthy's memory.

Japanese and Mandarin voices read each sentence on its own, so every full stop gets its pause; a sentence under six characters, such as 你好！, joins the next.

The non-English readings were compared with the readers Kokoro was trained with (misaki and espeak-ng), not checked by native speakers. If a language sounds wrong, please open an issue with the text.

Settings → General → **Reading Language** chooses between the voice's own language (the default) and English, read with the voice's accent; British voices keep British pronunciations and all others use American ones. `--lang` overrides it for one `--speak-file` call. It does nothing with the server engine, which decides the language itself.

## Using a speech server instead

Set Settings → General → Speech Engine to **HTTP server** to send the text to a local OpenAI-compatible speech server with `POST /v1/audio/speech` and `pcm` output, such as [Kokoro-FastAPI](https://github.com/remsky/Kokoro-FastAPI). Mouthy expects it at `http://127.0.0.1:8880` with the voice `af_alloy` by default; the voice list then shows the server's voices. Check the server is up:

```bash
curl http://127.0.0.1:8880/v1/audio/voices
```

Plain `http://` is unencrypted, so use `https://` for a server on another machine. The Local engine sends nothing anywhere; only the Models page's downloads use the network.

## Settings

Open Settings from the tray or by launching Mouthy again. Besides the hotkeys and voices above, General sets launch at login, start hidden, the overlay, speed (0.5×–2×), volume and the output device. Advanced hides the tray icon and, under experimental features, changes the shortcut backend. About sets the language and theme, opens the app data and log folders, and opens the third-party licences.

## Linux

x86_64 builds (AppImage, rpm and deb) are on [GitHub Releases](https://github.com/azizi-a/mouthy/releases). They need glibc 2.39 or newer: Ubuntu 24.04, Fedora 40 or later (Bazzite included), Debian 13 or later; not Ubuntu 22.04, Debian 12 or Fedora 39. To build one yourself, see [BUILD.md](BUILD.md#linux).

On Linux, install [espeak-ng](#better-pronunciation-of-unknown-words) with your distribution's package manager, or with `brew install espeak-ng` on an immutable distribution such as Bazzite.

Mouthy targets Wayland. It has been run on one laptop: Bazzite with KDE Plasma on Wayland, from the AppImage. Nobody has tested it on GNOME or any other desktop, or run the rpm or deb on a desktop, and X11 sessions are not supported.

### Install on Linux

- **AppImage**, best on immutable distributions such as Bazzite: keep it at a path that will not change, such as `~/Applications/Mouthy.AppImage`, because desktop shortcuts and Launch on Startup store that path. Make it executable with `chmod +x ~/Applications/Mouthy.AppImage`.
- **rpm** on Fedora: `sudo dnf install ./Mouthy-*.x86_64.rpm`.
- **deb** on Ubuntu or Debian: `sudo apt install ./Mouthy_*_amd64.deb`.

Mouthy reads the highlighted text and the clipboard with `wl-paste` from **wl-clipboard**, which it does not bundle. Install it with your distribution's package manager, or with `brew install wl-clipboard` on an immutable distribution. Settings → General says whether Mouthy found it.

### Shortcuts on Wayland

Wayland does not let an app listen for a hotkey of its own, so you bind Mouthy's commands to keys in the desktop's settings. Settings → General lists them with the right path, as text you can select and copy:

```bash
mouthy --speak-selection   # read the highlighted text, or stop if speaking
mouthy --speak-clipboard   # read the clipboard, or stop if speaking
mouthy --stop
```

For the AppImage, use its full path instead of `mouthy`, such as `/home/you/Applications/Mouthy.AppImage --speak-selection`. In KDE Plasma, open System Settings → Keyboard → Shortcuts → Add New → Command or Script, enter the command and choose a key. A shortcut pressed while Mouthy is not running only starts it, so turn on Launch on Startup. A speak shortcut pressed within about 0.7 s of the last one Mouthy acted on is ignored, so a held key cannot stop the speech it started; bind `--stop` too for a stop that is never ignored.

If a shortcut runs but nothing happens, the desktop may be dropping the command's arguments (Plasma 6.0 did). Bind a script instead: each is two lines, `#!/bin/sh` and `exec "$HOME/Applications/Mouthy.AppImage" --speak-selection` (or `--speak-clipboard`, `--stop`; use `mouthy` for the rpm or deb). This makes all three in `~/.local/bin`:

```bash
mkdir -p ~/.local/bin
for action in speak-selection speak-clipboard stop; do
  printf '#!/bin/sh\nexec "$HOME/Applications/Mouthy.AppImage" --%s\n' "$action" > ~/.local/bin/mouthy-$action
done
chmod +x ~/.local/bin/mouthy-*
```

The highlighted text is the PRIMARY selection: whatever you last highlighted in any app. Some apps, such as terminals and browsers, keep it after you click elsewhere, so a press can read text you highlighted earlier.

On Linux most password managers do not mark a password they copy. Mouthy refuses to read the clipboard only when it carries KDE's password hint, so Speak Clipboard may read a copied password aloud.

**In-app hotkeys (optional).** The Event Tap shortcut backend (Settings → Advanced → Experimental Features, then Keyboard Implementation) reads the keyboard itself, which needs your user in the `input` group. Nobody has tested it on Linux yet. Membership lets any program you run read every key you type, passwords included. Add yourself with `sudo usermod -aG input $USER`, then log out and back in. On Fedora Atomic desktops such as Bazzite, copy the group into `/etc/group` first: `grep -E '^input:' /usr/lib/group | sudo tee -a /etc/group`.

### Overlay, tray and sound on Linux

- In a Wayland session the AppImage runs as a native Wayland app on KDE Plasma and other Wayland desktops, and under XWayland on GNOME: GNOME has no layer shell, and under XWayland the overlay can be placed and kept from taking the keyboard. If the AppImage misbehaves on your desktop, start it with `MOUTHY_GDK_BACKEND=x11` to run it under XWayland; `MOUTHY_GDK_BACKEND=wayland` runs it natively on GNOME. The rpm and deb run natively in every Wayland session.
- On a desktop with the GTK layer shell, such as KDE Plasma, the overlay sits at the screen's edge and starts on. Elsewhere it is an ordinary window and starts off; Settings → General turns it on. It does not take the keyboard from the app you are typing in, except perhaps when Mouthy runs natively on a desktop without the layer shell, such as the rpm or deb on GNOME, where the desktop decides. With the overlay off, notices such as "Nothing selected" go only to the log.
- The tray icon needs an appindicator library; the AppImage carries one, and the rpm and deb install it. Without one Mouthy runs without a tray: launch it again to open Settings. Whenever no tray icon shows (none installed, Show Tray Icon off, or `--no-tray`), closing Settings quits Mouthy.
- Sound goes through ALSA, which PipeWire serves on current desktops.

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

The built app is `src-tauri/target/release/bundle/macos/Mouthy.app`. Under `bun run tauri dev`, macOS charges the Accessibility permission to your terminal, so test the hotkeys with the built app. See [BUILD.md](BUILD.md) for more. To test a build by hand, work through [docs/phase3-checklist.md](docs/phase3-checklist.md), and [docs/phase4-checklist.md](docs/phase4-checklist.md) for Linux and the languages; [docs/listening-test.md](docs/listening-test.md) checks how text is read, by ear.

## Attribution

Mouthy is derived from [Handy](https://github.com/cjpais/Handy) by CJ Pais (MIT). It is not affiliated with or endorsed by the Handy project. Handy's copyright notice is in [NOTICE](NOTICE).

## License

MIT. See [LICENSE](LICENSE). The licences of the software Mouthy includes are in [src-tauri/resources/THIRD_PARTY_NOTICES.txt](src-tauri/resources/THIRD_PARTY_NOTICES.txt), which ships inside the app and opens from Settings → About.
