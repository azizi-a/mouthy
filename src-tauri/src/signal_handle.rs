use crate::actions;
use log::debug;
use signal_hook::consts::SIGUSR2;
use signal_hook::iterator::Signals;
use std::thread;
use tauri::AppHandle;

#[cfg(target_os = "macos")]
use signal_hook::consts::SIGUSR1;

/// Listen for SIGUSR2, which speaks the selection (or stops speech), and on
/// macOS SIGUSR1, which stops speech.
///
/// SIGUSR1 must stay unhandled on Linux: WebKitGTK's JavaScriptCore garbage
/// collector sends it to its own threads to suspend them, so handling it
/// fired the action on every GC cycle (cjpais/Handy#1660).
pub fn setup_signal_handler(app_handle: AppHandle) {
    #[cfg(target_os = "macos")]
    let (handled, names) = ([SIGUSR2, SIGUSR1], "SIGUSR2, SIGUSR1");
    #[cfg(not(target_os = "macos"))]
    let (handled, names) = ([SIGUSR2], "SIGUSR2");

    let mut signals = Signals::new(handled).expect("failed to register signal handler");
    debug!("Signal handler registered ({names})");
    thread::spawn(move || {
        for signal in signals.forever() {
            match signal {
                SIGUSR2 => actions::speak_selection(&app_handle, "SIGUSR2"),
                #[cfg(target_os = "macos")]
                SIGUSR1 => actions::stop_speaking(&app_handle, "SIGUSR1"),
                _ => {}
            }
        }
    });
}
