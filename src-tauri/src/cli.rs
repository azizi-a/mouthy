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

    /// Stop speaking (sent to running instance)
    #[arg(long)]
    pub stop: bool,

    /// Enable debug mode with verbose logging
    #[arg(long)]
    pub debug: bool,
}
