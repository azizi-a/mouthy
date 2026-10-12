# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

- Forked from [Handy](https://github.com/cjpais/Handy) (cjpais, MIT) at upstream commit 7b2a28a; added the Phase 1 plan and agent docs.

### Changed

- Rebranded to Mouthy: new name, bundle identifier `com.azizi.mouthy`, placeholder icons, and attribution to Handy in README and LICENSE. Removed Handy's sponsor images, donate link, CI workflows and Nix packaging.

### Removed

- Auto-updater, "Check for Updates" tray item, What's New release notes, and the process plugin. Updates return in Phase 3 with a Mouthy signing key.
- Speech-to-text hotkeys (push-to-talk, hold and toggle modes), the hold threshold, history re-transcribe, and Handy's settings migrations. Hotkeys do nothing until the speak action lands.
