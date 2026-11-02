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
    #[arg(long, value_name = "PATH")]
    pub speak_file: Option<PathBuf>,

    /// Stop speaking (sent to running instance)
    #[arg(long)]
    pub stop: bool,

    /// Enable debug mode with verbose logging
    #[arg(long)]
    pub debug: bool,
}

impl CliArgs {
    /// The `--speak-file` path, a relative one taken from `cwd`, the directory
    /// Mouthy was launched from.
    pub fn speak_file_in(&self, cwd: &Path) -> Option<PathBuf> {
        self.speak_file.as_ref().map(|path| cwd.join(path))
    }

    /// The `--speak-file` path in a command line another launch handed over,
    /// resolved against that launch's working directory.
    pub fn handed_over_speak_file(args: &[String], cwd: &str) -> Option<PathBuf> {
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
    fn a_handed_over_speak_file_resolves_against_the_launch_directory() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(
            CliArgs::handed_over_speak_file(&args(&["mouthy", "--speak-file", "m.txt"]), "/work"),
            Some(PathBuf::from("/work/m.txt"))
        );
        assert_eq!(
            CliArgs::handed_over_speak_file(&args(&["mouthy", "--speak-file=/tmp/m.txt"]), "/work"),
            Some(PathBuf::from("/tmp/m.txt"))
        );
        assert_eq!(
            CliArgs::handed_over_speak_file(&args(&["mouthy", "--speak-file", "m.txt"]), ""),
            Some(PathBuf::from("m.txt")),
            "with no working directory the path stays relative, for the reader to refuse"
        );
        assert_eq!(
            CliArgs::handed_over_speak_file(&args(&["mouthy", "--stop"]), "/work"),
            None
        );
    }
}
