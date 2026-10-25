use clap::Parser;

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

    /// Stop speaking (sent to running instance)
    #[arg(long)]
    pub stop: bool,

    /// Enable debug mode with verbose logging
    #[arg(long)]
    pub debug: bool,
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
}
