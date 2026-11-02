use clap::Parser;
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

    /// Stop speaking (sent to running instance)
    #[arg(long)]
    pub stop: bool,

    /// Enable debug mode with verbose logging
    #[arg(long)]
    pub debug: bool,
}

/// A `--speak-file` request: the file and the voice asked for, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakFile {
    pub path: PathBuf,
    pub voice: Option<String>,
}

impl CliArgs {
    /// The `--speak-file` request, a relative path taken from `cwd`, the
    /// directory Mouthy was launched from.
    pub fn speak_file_in(&self, cwd: &Path) -> Option<SpeakFile> {
        self.speak_file.as_ref().map(|path| SpeakFile {
            path: cwd.join(path),
            voice: self.voice.clone(),
        })
    }

    /// The `--speak-file` request in a command line another launch handed
    /// over, its path resolved against that launch's working directory.
    pub fn handed_over_speak_file(args: &[String], cwd: &str) -> Option<SpeakFile> {
        Self::try_parse_from(args)
            .ok()?
            .speak_file_in(Path::new(cwd))
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
    fn a_handed_over_speak_file_resolves_against_the_launch_directory() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        let request = |path: &str, voice: Option<&str>| SpeakFile {
            path: PathBuf::from(path),
            voice: voice.map(str::to_string),
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
            CliArgs::handed_over_speak_file(&args(&["mouthy", "--speak-file", "m.txt"]), ""),
            Some(request("m.txt", None)),
            "with no working directory the path stays relative, for the reader to refuse"
        );
        assert_eq!(
            CliArgs::handed_over_speak_file(&args(&["mouthy", "--stop"]), "/work"),
            None
        );
    }
}
