# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

Mouthy starts as a fork of [Handy](https://github.com/cjpais/Handy) (CJ Pais, MIT) at upstream commit 7b2a28a, with speech-to-text replaced by text-to-speech. The release app bundle is 9.6 MB on Apple Silicon (debug binary 62 MB, down from Handy's 152 MB).

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
