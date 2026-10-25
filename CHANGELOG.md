# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## 0.4.1 — 2026-10-25

- The Settings voice picker lists only installed voices by default; a 'Show uninstalled voices' switch reveals the rest.

### Notes

- 0.4.0 was tagged but never published; this is the first release with the Phase 3 features below.

## 0.4.0 — 2026-10-25

Choosing a voice by ear: Settings lists all 54 Kokoro voices with Preview, and blends the voice in use with a second one. Also new: ⌥⇧⎋ reads the clipboard, skipping text a password manager marked secret; Settings refuses a shortcut another one already uses; a new icon; and the licences of the software Mouthy includes. Mouthy is still not signed or notarised and does not update itself.

### Added

- Voice picker in Settings → General → Speech: the voice in use at the top, then every voice by language, each with Download, Preview and Use, and an Installed only filter. With the HTTP server it lists the server's voices and says when the server has not answered a preview in 10 seconds. The Models page previews voices too, and a hotkey press stops a preview.
- Voice blends: the picker's Blend row mixes the voice in use with a second one on a slider, and Other… takes any blend of up to four voices written as Kokoro-FastAPI writes it, `bm_lewis(70)+bm_george(30)`, where each weight is a share of their sum and a voice without one counts as 1. Both engines speak blends; the offline voice reads with the first voice's pronunciation, refuses a name or blend it cannot read with a message, and the Models page shows each voice's share and downloads any that are missing.
- All 54 Kokoro v1.0 voices, labelled by language: Japanese, Mandarin, Spanish, French, Hindi, Italian and Brazilian Portuguese join British and American English.
- Speak clipboard: press ⌥⇧⎋ (Alt+Shift+Esc on Windows/Linux) to read the clipboard text aloud without copying the selection, for terminal programs such as Claude Code and nvim that leave no selection to copy. Clipboard text a password manager marked as secret is not read. Also available as `--speak-clipboard`.
- Settings refuses a shortcut that another one already uses. After an update, a new shortcut whose keys you already use starts unset.
- The app ships the licences of Mouthy and the software it includes, ONNX Runtime's third-party notices among them; Settings → About → Third-party licences opens them.

### Changed

- New app icon and menu-bar icons: a pair of lips, open while Mouthy is speaking.

### Notes

- Mouthy reads every text as English, so the non-English voices read English with their language's accent, not their own language.
- Going back to 0.3.0 with a blend selected: 0.3.0 reports the blend as a missing voice until you pick a single voice.

## 0.3.0 — 2026-10-24

Reading copied text as prose: lines wrapped by terminals, email and PDFs are joined again, and the offline voice says initialisms, dotted names, number ranges and abbreviations as a person would. New installs default to the American English voice `af_alloy`.

### Added

- Text wrapped onto several lines, as terminals, man pages, email and PDFs deliver it, is joined again before reading, so a sentence is no longer cut at every line end. A line is joined to the next only when the next line's first word would not have fitted on it, whether it starts with a capital or not, so lines of a similar width with no punctuation, such as an unmarked list, join too. Headings, list items, quotes, tables, chat lines, fenced code, code that opens a block with `{` or `:` and lines ending in punctuation keep their breaks, as do lines starting with a number, a code such as a commit hash, or a word in capitals such as a log level, outside lower-case prose.
- The offline voice spells initialisms by their letter names ("ID", "the API URL", "IDs", "U.S.", "e.g.") and reads camel-case names as their words ("GitHub", "macOS", "HTMLParser"). Capitals its dictionary knows as words stay words ("NASA", "RAM", "NOTE"), and JSON, YAML, TOML, README, TODO and FIXME are said as words.

### Changed

- New installs default to the American English voice `af_alloy` instead of `bm_lewis`; existing installs keep the voice they use.
- Settings with no speech engine saved, or an unreadable one, now use the offline voice instead of the HTTP server; installs that chose the HTTP server keep it.

### Fixed

- With the offline voice, a dash between two plain numbers reads "to", so "pages 10-12" is "pages ten to twelve" and "a 3-1 win" is "three to one".
- With the offline voice, a full stop inside a name no longer ends a sentence: "Node.js", "example.com" and ".env" are read with "dot", and "e.g." or "U.S." without pauses.
- With the offline voice, titles and common abbreviations are said in full without a pause after them: "Dr. Smith" is "Doctor Smith", and "Mr.", "Mrs.", "Ms.", "St.", "etc.", "vs.", "approx." and "No. 5" likewise.

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
