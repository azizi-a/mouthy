# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

- Forked from [Handy](https://github.com/cjpais/Handy) (cjpais, MIT) at upstream commit 7b2a28a; added the Phase 1 plan and agent docs.

### Added

- Speech building blocks: clean-up of selected text for reading aloud (links, markdown, whitespace), sentence chunking, and a client for OpenAI-compatible speech servers such as Kokoro-FastAPI.
- Speak selection: press ⌥⎋ (Alt+Esc on Windows/Linux) to read the selected text aloud, and press again to stop. Playback starts after the first sentence is ready while the rest is still being synthesised. Also available from the tray (Speak Selection / Stop), the `--speak-selection` and `--stop` flags, and `SIGUSR2` (speak) / `SIGUSR1` (stop, macOS only). The clipboard is restored exactly after the copy, including rich text and password-manager privacy markers. The overlay shows speaking, "nothing selected" and error states. Server URL, voice, speed and volume are stored settings (no UI yet).

### Changed

- Rebranded to Mouthy: new name, bundle identifier `com.azizi.mouthy`, placeholder icons, and attribution to Handy in README and LICENSE. Removed Handy's sponsor images, donate link, CI workflows and Nix packaging.

### Removed

- Auto-updater, "Check for Updates" tray item, What's New release notes, and the process plugin. Updates return in Phase 3 with a Mouthy signing key.
- Speech-to-text hotkeys (push-to-talk, hold and toggle modes), the hold threshold, history re-transcribe, and Handy's settings migrations. Hotkeys do nothing until the speak action lands.
- Microphone capture, voice activity detection, the Live overlay style, and the microphone permission prompt. Onboarding now asks only for Accessibility, and the app no longer declares microphone access.
- Start/stop sound feedback, sound themes, custom sounds, and the feedback volume. The output device setting stays for speech playback.
- Speech recognition engines (whisper/transcribe.cpp and ONNX), the model catalog, downloader and Models page, model selection and unloading (settings, footer and tray), acceleration and language settings, the headless `--transcribe-file`/`--list-devices`/`--list-models` CLI, and Windows portable mode. Onboarding now finishes after the Accessibility step.
- Text post-processing: LLM cleanup (OpenAI-compatible providers and Apple Intelligence), custom words, filler-word removal, Chinese script conversion, language detection, the Post Process page, and the `--toggle-post-process` flag and macOS `SIGUSR1` trigger.
- Transcript history: the History page, saved recordings and their SQLite database, history limit and auto-delete settings, and the tray "Copy Last Transcript" item. Mouthy no longer creates `history.db` or `recordings/` in its data folder.
- The Cancel shortcut, the `--toggle-transcription` and `--cancel` flags, and the tray Cancel item; Speak Selection and Stop replace them.

### Fixed

- Speech plays on output devices that reject rodio's default 50 ms buffer, such as Bluetooth headsets.
