use crate::actions::ACTION_MAP;
#[cfg(unix)]
use log::debug;
use log::warn;
use tauri::AppHandle;

#[cfg(unix)]
use signal_hook::consts::SIGUSR2;
#[cfg(unix)]
use signal_hook::iterator::Signals;
#[cfg(unix)]
use std::thread;

/// Run a binding's action as a single press; external triggers have no release.
/// Used by signal handlers, CLI flags, and any other external trigger.
pub fn send_external_input(app: &AppHandle, binding_id: &str, source: &str) {
    match ACTION_MAP.get(binding_id) {
        Some(action) => action.start(app, binding_id, source),
        None => warn!("No action in ACTION_MAP for '{binding_id}' (from {source})"),
    }
}

/// Listen for SIGUSR2, which remotely toggles transcription.
///
/// SIGUSR1 must stay unhandled on Linux: WebKitGTK's JavaScriptCore garbage
/// collector sends it to its own threads to suspend them, so handling it
/// caused phantom recordings on every GC cycle (#1660).
#[cfg(unix)]
pub fn setup_signal_handler(app_handle: AppHandle) {
    let mut signals =
        Signals::new([SIGUSR2]).expect("failed to register transcription signal handler");
    debug!("Signal handler registered (SIGUSR2)");
    thread::spawn(move || {
        for _ in signals.forever() {
            debug!("Received SIGUSR2");
            send_external_input(&app_handle, "transcribe", "SIGUSR2");
        }
    });
}
