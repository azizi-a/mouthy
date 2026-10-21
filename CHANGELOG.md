# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

### Changed

- New installs default to the American English voice `af_alloy` instead of `bm_lewis`; existing installs keep the voice they use.

## 0.2.0 — 2026-10-14

Offline speech: the Kokoro-82M voice runs inside Mouthy through ONNX Runtime, with the speech server kept as an alternative. The release app bundle is 62 MB on Apple Silicon (19.8 MB zipped), most of it ONNX Runtime and the offline voice's English lexicons.

### Added

- Offline speech: with Speech Engine set to Local (offline), the Kokoro-82M voice runs inside Mouthy from model files in its models folder, with no server. It loads in the background at launch, British voices (`b…`) get British pronunciations, and stopping ends synthesis at once. It reads each sentence in one go, so the intonation carries through to its end (only a sentence over 450 characters is cut, at a clause break), and trims the voice's padding so sentences join with natural pauses and speech starts sooner. Its memory reaches about 0.6–1.1 GB after long sentences and stays there while Mouthy runs. Missing files show a notice with Open Settings.
- The offline voice reads money, decimals, version numbers, years and decades, ordinals, times (including 7.30pm) and percentages as words, and leaves dates such as 2026-10-13 and phone or card numbers as written. Curly apostrophes and double quotes, accented letters (including an accent typed as a separate mark), ligatures and the minus sign are read as plain characters, so "don’t" and "café" are no longer spelled out, and a spaced dash keeps its pause.
- Settings → General → Pronounce unknown words with espeak-ng (off by default, Local only): with a separately installed espeak-ng (`brew install espeak-ng`), the offline voice asks it how to say words its dictionary lacks, such as names, instead of spelling them out. Mouthy does not bundle espeak-ng (GPL-3); it runs it as a separate program.
- Settings → Models downloads the offline voice from Hugging Face: the int8 or fp32 model and any of 28 English voices, with progress, cancel and resume, each checked against its published SHA-256. The page also picks the model and voice and deletes files; deleting the files in use frees their memory. Its status names a selected voice that has no file, such as a typed name, and shows why the voice failed to load. The missing-files notice's Open Settings opens it.

### Changed

- New installs use the offline voice by default; installs that used the HTTP server keep it, as do settings whose speech engine value cannot be read.
- With the offline voice, a long first sentence starts playing after its first clause (never inside a range such as 1914–1918), so speech begins sooner.
- Requires macOS 13.4 or later.
- Building downloads a prebuilt ONNX Runtime (about 10 MB, cached after the first build) for the offline voice and needs Rust 1.92 or newer. Intel and universal macOS builds no longer link.

### Fixed

- × on the overlay closes it without bringing the Settings window to the front.

## 0.1.0 (not released)

Mouthy starts as a fork of [Handy](https://github.com/cjpais/Handy) (CJ Pais, MIT) at upstream commit 7b2a28a, with speech-to-text replaced by text-to-speech. The release app bundle is 9.6 MB on Apple Silicon (4.4 MB zipped).

### Added

- Speak selection: press ⌥⎋ (Alt+Esc on Windows/Linux) to read the selected text aloud, and press again to stop. Playback starts once the first sentence is ready while the rest is still being synthesised.
- Speech through an OpenAI-compatible server that supports `pcm` output, such as Kokoro-FastAPI (default `http://127.0.0.1:8880`, voice `bm_lewis`). Errors show the server's own message, and a system proxy does not get in the way.
- Text clean-up before reading: links become "link", markdown symbols and extra whitespace go, lines without punctuation (headings, bullets) read as separate sentences, and common abbreviations such as "Dr." do not end a sentence. A sentence the server cannot speak is skipped.
- Selection capture that restores the clipboard afterwards, including rich content and password-manager privacy markers. It waits briefly for the hotkey's modifiers to be released before copying. Clipboards over 16 MB are not restored.
- Overlay states for speaking, "nothing selected" and errors, with an Open Settings button and a stop button.
- Tray items Speak Selection and Stop, the `--speak-selection` and `--stop` flags, and the signals `SIGUSR2` (speak) and `SIGUSR1` (stop, macOS only).
- Speech settings: server URL, voice (typed, or picked from the server's voice list), speed (0.5×–2×) and volume.

### Changed

- Rebranded to Mouthy: name, bundle identifier `com.azizi.mouthy`, placeholder icons, an indigo accent, and attribution to Handy in README, LICENSE and About.
- Launch at login, start hidden and the overlay settings live on the General page with the hotkey and speech settings.
- Wording no longer mentions transcription, recording or feedback sounds. Text whose meaning changed shows in English in other languages until translated.
- The Windows installer uses Tauri's standard template.

### Removed

- Speech-to-text: microphone capture, voice activity detection, recognition engines, the model catalog, downloader and Models page, and the microphone permission.
- Post-processing (LLM clean-up, Apple Intelligence, custom words, filler-word removal, Chinese script conversion, language detection) and transcript history.
- Pasting or typing text into the focused app and every paste-related setting.
- Start/stop sounds and sound themes; the output device setting stays.
- The Cancel shortcut, push-to-talk and hold modes, and the `--toggle-transcription`, `--toggle-post-process`, `--cancel`, `--transcribe-file`, `--list-devices` and `--list-models` flags.
- The auto-updater, What's New notes, Windows portable mode and settings migrations from Handy.
- Handy's sponsor images, donate link, CI workflows and Nix packaging, plus dependencies left unused by the above.

### Fixed

- Speech plays on output devices that reject the default 50 ms buffer, such as Bluetooth headsets.
- Holding the hotkey no longer stops the speech it started: a press stops speech only after the hotkey has been released.
