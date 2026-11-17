use crate::settings::ReadingLanguage;
use clap::builder::PossibleValue;
use clap::{Parser, ValueEnum};
use log::warn;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug, Clone, Default)]
#[command(name = "mouthy", about = "Mouthy - Text to Speech")]
pub struct CliArgs {
    /// Start with the main window hidden
    #[arg(long)]
    pub start_hidden: bool,

    /// Disable the system tray icon
    #[arg(long)]
    pub no_tray: bool,

    /// Speak the current selection, or stop if speaking (sent to running instance)
    #[arg(long)]
    pub speak_selection: bool,

    /// Speak the clipboard text, or stop if speaking (sent to running instance)
    #[arg(long)]
    pub speak_clipboard: bool,

    /// Speak a UTF-8 text file, cutting off any speech (sent to running instance, or spoken once launched)
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with_all = ["speak_selection", "speak_clipboard", "stop"]
    )]
    pub speak_file: Option<PathBuf>,

    /// Voice for --speak-file: a voice id or blend, such as bm_lewis or
    /// 'bm_lewis(70)+bm_george(30)'; one not downloaded falls back to the Settings voice
    // clap waives `requires` when the required flag conflicts with one given,
    // so the other speak flags are refused here too.
    #[arg(
        long,
        value_name = "SPEC",
        requires = "speak_file",
        conflicts_with_all = ["speak_selection", "speak_clipboard", "stop"]
    )]
    pub voice: Option<String>,

    /// Language for --speak-file, overriding the Settings reading language
    #[arg(
        long,
        value_enum,
        value_name = "LANG",
        requires = "speak_file",
        conflicts_with_all = ["speak_selection", "speak_clipboard", "stop"]
    )]
    pub lang: Option<ReadingLanguage>,

    /// Stop speaking (sent to running instance)
    #[arg(long)]
    pub stop: bool,

    /// Enable debug mode with verbose logging
    #[arg(long)]
    pub debug: bool,
}

impl ValueEnum for ReadingLanguage {
    fn value_variants<'a>() -> &'a [Self] {
        &[ReadingLanguage::English, ReadingLanguage::Voice]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(match self {
            ReadingLanguage::English => {
                PossibleValue::new("en").help("English, in the voice's accent")
            }
            ReadingLanguage::Voice => PossibleValue::new("voice").help("The voice's own language"),
        })
    }
}

/// A `--speak-file` request: the file, and the voice and language asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakFile {
    pub path: PathBuf,
    pub voice: Option<String>,
    pub language: Option<ReadingLanguage>,
}

impl CliArgs {
    /// The `--speak-file` request, a relative path taken from `cwd`, the
    /// directory Mouthy was launched from.
    pub fn speak_file_in(&self, cwd: &Path) -> Option<SpeakFile> {
        self.speak_file.as_ref().map(|path| SpeakFile {
            path: cwd.join(path),
            voice: self.voice.clone(),
            language: self.lang,
        })
    }

    /// The `--speak-file` request in a command line another launch handed
    /// over, its path resolved against that launch's working directory.
    pub fn handed_over_speak_file(args: &[String], cwd: &str) -> Option<SpeakFile> {
        match Self::try_parse_from(args) {
            Ok(parsed) => parsed.speak_file_in(Path::new(cwd)),
            Err(e) => {
                // A newer binary can hand over a flag this version lacks.
                if args
                    .iter()
                    .any(|arg| arg == "--speak-file" || arg.starts_with("--speak-file="))
                {
                    warn!("Ignoring a handed-over --speak-file: {:?}", e.kind());
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speak_clipboard_flag_parses_on_its_own() {
        let args = CliArgs::try_parse_from(["mouthy", "--speak-clipboard"]).unwrap();
        assert!(args.speak_clipboard);
        assert!(!args.speak_selection);
    }

    #[test]
    fn speak_file_takes_the_next_argument_or_an_equals_value() {
        let args = CliArgs::try_parse_from(["mouthy", "--speak-file", "/tmp/m.txt"]).unwrap();
        assert_eq!(args.speak_file, Some(PathBuf::from("/tmp/m.txt")));
        let args = CliArgs::try_parse_from(["mouthy", "--speak-file=/tmp/m.txt"]).unwrap();
        assert_eq!(args.speak_file, Some(PathBuf::from("/tmp/m.txt")));
        assert!(CliArgs::try_parse_from(["mouthy", "--speak-file"]).is_err());
        assert!(
            CliArgs::try_parse_from(["mouthy", "--speak-file", "--stop"]).is_err(),
            "a flag is not taken as the path"
        );
        assert_eq!(
            CliArgs::try_parse_from(["mouthy"]).unwrap().speak_file,
            None
        );
    }

    #[test]
    fn speak_file_refuses_the_other_speak_flags() {
        for other in ["--speak-selection", "--speak-clipboard", "--stop"] {
            assert!(
                CliArgs::try_parse_from(["mouthy", "--speak-file", "/tmp/m.txt", other]).is_err(),
                "--speak-file with {other}"
            );
        }
        let args =
            CliArgs::try_parse_from(["mouthy", "--start-hidden", "--speak-file", "/tmp/m.txt"])
                .unwrap();
        assert!(args.start_hidden);
    }

    #[test]
    fn voice_comes_with_speak_file_in_either_spelling() {
        let args = CliArgs::try_parse_from([
            "mouthy",
            "--speak-file",
            "/tmp/m.txt",
            "--voice",
            "bm_lewis(70)+bm_george(30)",
        ])
        .unwrap();
        assert_eq!(args.voice.as_deref(), Some("bm_lewis(70)+bm_george(30)"));
        let args =
            CliArgs::try_parse_from(["mouthy", "--voice=bm_lewis", "--speak-file=/tmp/m.txt"])
                .unwrap();
        assert_eq!(args.voice.as_deref(), Some("bm_lewis"));
        assert_eq!(
            CliArgs::try_parse_from(["mouthy", "--speak-file", "/tmp/m.txt"])
                .unwrap()
                .voice,
            None
        );
    }

    #[test]
    fn voice_without_speak_file_is_refused() {
        for list in [
            &["mouthy", "--voice", "bm_lewis"][..],
            &["mouthy", "--voice=bm_lewis", "--speak-clipboard"],
            &["mouthy", "--voice", "bm_lewis", "--stop"],
        ] {
            assert!(CliArgs::try_parse_from(list).is_err(), "{list:?}");
        }
        assert!(
            CliArgs::try_parse_from(["mouthy", "--speak-file", "/tmp/m.txt", "--voice"]).is_err(),
            "--voice needs a value"
        );
    }

    #[test]
    fn lang_comes_with_speak_file_as_en_or_voice() {
        let lang = |list: &[&str]| CliArgs::try_parse_from(list).map(|args| args.lang);
        assert_eq!(
            lang(&["mouthy", "--speak-file", "/tmp/m.txt", "--lang", "en"]).unwrap(),
            Some(ReadingLanguage::English)
        );
        assert_eq!(
            lang(&["mouthy", "--lang=voice", "--speak-file=/tmp/m.txt"]).unwrap(),
            Some(ReadingLanguage::Voice)
        );
        assert_eq!(
            lang(&["mouthy", "--speak-file", "/tmp/m.txt"]).unwrap(),
            None
        );
        for list in [
            &["mouthy", "--speak-file", "/tmp/m.txt", "--lang", "es"][..],
            &["mouthy", "--speak-file", "/tmp/m.txt", "--lang", "english"],
            &["mouthy", "--speak-file", "/tmp/m.txt", "--lang"],
            &["mouthy", "--speak-file", "/tmp/m.txt", "--lang", "EN"],
            &["mouthy", "--lang", "en"],
            &["mouthy", "--lang", "en", "--stop"],
            &["mouthy", "--lang", "en", "--voice", "bm_lewis"],
            &["mouthy", "--lang=en", "--speak-clipboard"],
            &["mouthy", "--lang", "en", "--speak-selection"],
            &[
                "mouthy",
                "--speak-file",
                "/tmp/m.txt",
                "--lang",
                "en",
                "--stop",
            ],
        ] {
            assert!(lang(list).is_err(), "{list:?}");
        }
    }

    #[test]
    fn lang_codes_are_the_stored_setting_values() {
        for language in ReadingLanguage::value_variants() {
            let code = language.to_possible_value().unwrap();
            assert_eq!(
                serde_json::to_value(language).unwrap(),
                code.get_name(),
                "{language:?}"
            );
        }
    }

    #[test]
    fn a_handed_over_speak_file_resolves_against_the_launch_directory() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        let request = |path: &str, voice: Option<&str>| SpeakFile {
            path: PathBuf::from(path),
            voice: voice.map(str::to_string),
            language: None,
        };
        assert_eq!(
            CliArgs::handed_over_speak_file(&args(&["mouthy", "--speak-file", "m.txt"]), "/work"),
            Some(request("/work/m.txt", None))
        );
        assert_eq!(
            CliArgs::handed_over_speak_file(&args(&["mouthy", "--speak-file=/tmp/m.txt"]), "/work"),
            Some(request("/tmp/m.txt", None))
        );
        assert_eq!(
            CliArgs::handed_over_speak_file(
                &args(&["mouthy", "--speak-file", "m.txt", "--voice", "bf_emma"]),
                "/work"
            ),
            Some(request("/work/m.txt", Some("bf_emma")))
        );
        assert_eq!(
            CliArgs::handed_over_speak_file(
                &args(&[
                    "mouthy",
                    "--speak-file",
                    "m.txt",
                    "--voice",
                    "ef_dora",
                    "--lang",
                    "en"
                ]),
                "/work"
            ),
            Some(SpeakFile {
                language: Some(ReadingLanguage::English),
                ..request("/work/m.txt", Some("ef_dora"))
            })
        );
        assert_eq!(
            CliArgs::handed_over_speak_file(&args(&["mouthy", "--speak-file", "m.txt"]), ""),
            Some(request("m.txt", None)),
            "with no working directory the path stays relative, for the reader to refuse"
        );
        assert_eq!(
            CliArgs::handed_over_speak_file(&args(&["mouthy", "--stop"]), "/work"),
            None
        );
        assert_eq!(
            CliArgs::handed_over_speak_file(
                &args(&["mouthy", "--speak-file", "m.txt", "--from-a-newer-build"]),
                "/work"
            ),
            None
        );
    }
}
