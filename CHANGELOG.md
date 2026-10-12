# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

- Forked from [Handy](https://github.com/cjpais/Handy) (cjpais, MIT) at upstream commit 7b2a28a; added the Phase 1 plan and agent docs.

### Changed

- Rebranded to Mouthy: new name, bundle identifier `com.azizi.mouthy`, placeholder icons, and attribution to Handy in README and LICENSE. Removed Handy's sponsor images, donate link, CI workflows and Nix packaging.

### Removed

- Auto-updater, "Check for Updates" tray item, What's New release notes, and the process plugin. Updates return in Phase 3 with a Mouthy signing key.
- Speech-to-text hotkeys (push-to-talk, hold and toggle modes), the hold threshold, history re-transcribe, and Handy's settings migrations. Hotkeys do nothing until the speak action lands.
- Microphone capture, voice activity detection, the Live overlay style, and the microphone permission prompt. Onboarding now asks only for Accessibility, and the app no longer declares microphone access.
- Start/stop sound feedback, sound themes, custom sounds, and the feedback volume. The output device setting stays for speech playback.
- Speech recognition engines (whisper/transcribe.cpp and ONNX), the model catalog, downloader and Models page, model selection and unloading (settings, footer and tray), acceleration and language settings, the headless `--transcribe-file`/`--list-devices`/`--list-models` CLI, and Windows portable mode. Onboarding now finishes after the Accessibility step.
- Text post-processing: LLM cleanup (OpenAI-compatible providers and Apple Intelligence), custom words, filler-word removal, Chinese script conversion, language detection, the Post Process page, and the `--toggle-post-process` flag and macOS `SIGUSR1` trigger.
