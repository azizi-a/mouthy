use crate::audio_out;
use crate::clipboard;
use crate::overlay;
use crate::settings;
use crate::tray::{self, TrayIconState};
use crate::tts;
use log::{debug, error, info, warn};
use once_cell::sync::Lazy;
use rodio::buffer::SamplesBuffer;
use rodio::Player;
use std::collections::HashMap;
use std::num::NonZero;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

// Shortcut Action Trait
pub trait ShortcutAction: Send + Sync {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
}

/// Sentences queued in the player ahead of the one playing. The synth thread
/// can run at most this many plus the channel capacity ahead of playback.
const READ_AHEAD: usize = 2;
const PLAYBACK_POLL: Duration = Duration::from_millis(20);
const MONO: NonZero<u16> = NonZero::<u16>::MIN;
const SAMPLE_RATE: NonZero<u32> = NonZero::new(tts::http::SAMPLE_RATE).unwrap();

type Chunk = Result<Vec<i16>, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Capturing,
    Speaking,
}

/// One press of the speak hotkey: capturing the selection, then speaking it.
struct Session {
    generation: u64,
    stop: Arc<AtomicBool>,
    phase: Phase,
}

/// What the overlay and tray show after a session change.
enum Ui {
    Speaking,
    Idle,
    Notice {
        kind: &'static str,
        message: Option<String>,
    },
}

/// The current speak session, if any.
#[derive(Default)]
pub struct TtsState {
    current: Mutex<Option<Session>>,
    last_generation: AtomicU64,
    /// Numbers UI changes in the order the session changed.
    ui_seq: AtomicU64,
    /// Highest UI change applied; only the main thread touches it.
    ui_applied: AtomicU64,
}

type SessionLock<'a> = MutexGuard<'a, Option<Session>>;

impl TtsState {
    fn lock(&self) -> SessionLock<'_> {
        self.current.lock().unwrap_or_else(|poisoned| {
            warn!("TTS state mutex was poisoned, recovering");
            poisoned.into_inner()
        })
    }

    /// Schedules `ui` on the main thread. Taking the lock guard numbers the
    /// change in session order. Never touch the tray or overlay directly while
    /// holding the lock: their getters wait on the main thread, which may be
    /// waiting on this lock.
    fn post_ui(&self, _held: &SessionLock<'_>, app: &AppHandle, ui: Ui) {
        let seq = self.ui_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let handle = app.clone();
        if let Err(e) = app.run_on_main_thread(move || apply_ui(&handle, seq, ui)) {
            warn!("Failed to update the overlay and tray: {e}");
        }
    }
}

/// Main thread only.
fn apply_ui(app: &AppHandle, seq: u64, ui: Ui) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    // A change posted from another thread can arrive after a newer one that
    // the main thread applied inline.
    if seq < state.ui_applied.load(Ordering::SeqCst) {
        return;
    }
    state.ui_applied.store(seq, Ordering::SeqCst);
    match ui {
        Ui::Speaking => {
            overlay::show_speaking_overlay(app);
            tray::set_tray_state(app, TrayIconState::Speaking);
        }
        Ui::Idle => {
            overlay::hide_recording_overlay(app);
            tray::set_tray_state(app, TrayIconState::Idle);
        }
        Ui::Notice { kind, message } => {
            overlay::show_overlay_notice(app, kind, message.as_deref());
            tray::set_tray_state(app, TrayIconState::Idle);
        }
    }
}

fn is_current(current: &Option<Session>, generation: u64) -> bool {
    current.as_ref().map(|session| session.generation) == Some(generation)
}

/// Ends the session `current` holds, if any, with `ui`.
fn end_session(state: &TtsState, current: &mut SessionLock<'_>, app: &AppHandle, ui: Ui) {
    if let Some(session) = current.take() {
        session.stop.store(true, Ordering::SeqCst);
    }
    state.post_ui(current, app, ui);
}

/// Speaks the current selection, or stops if already speaking. A press while
/// the selection is still being captured is ignored.
pub fn speak_selection(app: &AppHandle, source: &str) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    let mut current = state.lock();
    match current.as_ref().map(|session| session.phase) {
        Some(Phase::Speaking) => {
            info!("Stopping speech ({source})");
            end_session(&state, &mut current, app, Ui::Idle);
            return;
        }
        Some(Phase::Capturing) => {
            debug!("Ignoring speak from {source}: still capturing the selection");
            return;
        }
        None => {}
    }

    let generation = state.last_generation.fetch_add(1, Ordering::Relaxed) + 1;
    let stop = Arc::new(AtomicBool::new(false));
    *current = Some(Session {
        generation,
        stop: stop.clone(),
        phase: Phase::Capturing,
    });
    drop(current);

    info!("Speaking selection ({source})");
    // Capture needs the main thread. Never wait for it here: the handy-keys
    // thread calling this is the one the main thread's shortcut commands wait on.
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || capture_and_speak(&handle, generation, stop)) {
        error!("Failed to schedule selection capture: {e}");
        let mut current = state.lock();
        if is_current(&current, generation) {
            *current = None;
        }
    }
}

/// Stops any speech or capture in progress and hides the overlay.
pub fn stop_speaking(app: &AppHandle, source: &str) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    info!("Stopping speech ({source})");
    end_session(&state, &mut state.lock(), app, Ui::Idle);
}

/// Runs on the main thread.
fn capture_and_speak(app: &AppHandle, generation: u64, stop: Arc<AtomicBool>) {
    let captured = clipboard::capture_selection(app);
    let settings = settings::get_settings(app);

    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    let mut current = state.lock();
    if !is_current(&current, generation) {
        debug!("Selection captured after the session was stopped; not speaking it");
        return;
    }

    let text = match captured {
        Ok(text) => text
            .map(|text| tts::text::normalize(&text))
            .unwrap_or_default(),
        Err(e) => {
            error!("Failed to capture the selection: {e}");
            let ui = Ui::Notice {
                kind: "error",
                message: Some(e),
            };
            end_session(&state, &mut current, app, ui);
            return;
        }
    };
    let sentences = tts::text::split_sentences(&text);
    info!(
        "Captured {} chars, {} sentences",
        text.chars().count(),
        sentences.len()
    );
    if sentences.is_empty() {
        let ui = Ui::Notice {
            kind: "nothing-selected",
            message: None,
        };
        end_session(&state, &mut current, app, ui);
        return;
    }

    let (tx, rx) = mpsc::sync_channel::<Chunk>(2);
    let playback = {
        let app = app.clone();
        let stop = stop.clone();
        let device = settings.selected_output_device.clone();
        let volume = settings.tts_volume;
        thread::Builder::new()
            .name("tts-playback".into())
            .spawn(move || play(app, generation, stop, rx, device, volume))
    };
    let synth = playback.and_then(|_| {
        let stop = stop.clone();
        thread::Builder::new()
            .name("tts-synth".into())
            .spawn(move || {
                synthesize(
                    sentences,
                    settings.tts_base_url,
                    settings.tts_voice,
                    settings.tts_speed,
                    stop,
                    tx,
                )
            })
    });
    if let Err(e) = synth {
        error!("Failed to start speech threads: {e}");
        let ui = Ui::Notice {
            kind: "error",
            message: Some(e.to_string()),
        };
        end_session(&state, &mut current, app, ui);
        return;
    }

    if let Some(session) = current.as_mut() {
        session.phase = Phase::Speaking;
    }
    state.post_ui(&current, app, Ui::Speaking);
}

/// Synth thread: one request per sentence, handed to playback in order.
fn synthesize(
    sentences: Vec<String>,
    base_url: String,
    voice: String,
    speed: f32,
    stop: Arc<AtomicBool>,
    tx: SyncSender<Chunk>,
) {
    let client = tts::http::client();
    for sentence in &sentences {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        // The only block_on in the app: it panics on an async runtime worker,
        // and this is a plain thread.
        let chunk = tauri::async_runtime::block_on(tts::http::synthesize(
            &client, &base_url, &voice, speed, sentence,
        ));
        let failed = chunk.is_err();
        // A send error means playback has ended.
        if tx.send(chunk).is_err() || failed {
            break;
        }
    }
}

/// Ends the session when the playback thread exits for any reason, including
/// a panic or a missing output device, unless it was stopped or replaced first.
struct SessionGuard {
    app: AppHandle,
    generation: u64,
    stop: Arc<AtomicBool>,
    error: Option<String>,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        // Also ends the synth thread when playback ended first.
        self.stop.store(true, Ordering::SeqCst);
        if thread::panicking() {
            error!("Speech playback thread panicked");
        }
        let Some(state) = self.app.try_state::<TtsState>() else {
            return;
        };
        let mut current = state.lock();
        if !is_current(&current, self.generation) {
            return;
        }
        let ui = match self.error.take() {
            Some(message) => Ui::Notice {
                kind: "error",
                message: Some(message),
            },
            None => Ui::Idle,
        };
        end_session(&state, &mut current, &self.app, ui);
    }
}

/// Playback thread: sole owner of the output device and the player.
fn play(
    app: AppHandle,
    generation: u64,
    stop: Arc<AtomicBool>,
    rx: Receiver<Chunk>,
    device: Option<String>,
    volume: f32,
) {
    let mut guard = SessionGuard {
        app,
        generation,
        stop: stop.clone(),
        error: None,
    };
    let mut sink = match audio_out::open_output(device.as_deref()) {
        Ok(sink) => sink,
        Err(e) => {
            error!("Cannot play speech: {e}");
            guard.error = Some(e);
            return;
        }
    };
    sink.log_on_drop(false);
    let player = Player::connect_new(sink.mixer());
    player.set_volume(volume);

    let mut queued = 0usize;
    loop {
        if stop.load(Ordering::SeqCst) {
            player.stop();
            break;
        }
        // `append` never blocks, so this is what bounds the read-ahead.
        if player.len() >= READ_AHEAD {
            thread::sleep(PLAYBACK_POLL);
            continue;
        }
        match rx.recv_timeout(PLAYBACK_POLL) {
            Ok(Ok(samples)) => {
                // Appending after `stop` restarts a stopped player.
                if stop.load(Ordering::SeqCst) {
                    player.stop();
                    break;
                }
                player.append(SamplesBuffer::new(MONO, SAMPLE_RATE, pcm_to_f32(&samples)));
                queued += 1;
                if queued == 1 {
                    info!("First speech chunk queued");
                }
            }
            Ok(Err(message)) => {
                error!("Speech synthesis failed: {message}");
                guard.error = Some(message);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                while !player.empty() && !stop.load(Ordering::SeqCst) {
                    thread::sleep(PLAYBACK_POLL);
                }
                player.stop();
                break;
            }
        }
    }
    debug!("Speech playback ended after {queued} chunks");
}

fn pcm_to_f32(samples: &[i16]) -> Vec<f32> {
    samples
        .iter()
        .map(|&sample| f32::from(sample) / 32768.0)
        .collect()
}

/// Toggle: press speaks the selection or stops speech; release does nothing.
struct SpeakAction;

impl ShortcutAction for SpeakAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, shortcut_str: &str) {
        speak_selection(app, shortcut_str);
    }

    fn stop(&self, _app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {}
}

// Static Action Map
pub static ACTION_MAP: Lazy<HashMap<String, Arc<dyn ShortcutAction>>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "speak".to_string(),
        Arc::new(SpeakAction) as Arc<dyn ShortcutAction>,
    );
    map
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_binding_has_an_action() {
        for id in settings::get_default_settings().bindings.keys() {
            assert!(ACTION_MAP.contains_key(id), "no action for binding '{id}'");
        }
    }

    #[test]
    fn pcm_converts_to_unit_range_floats() {
        assert_eq!(
            pcm_to_f32(&[0, i16::MIN, 16384, i16::MAX]),
            vec![0.0, -1.0, 0.5, 32767.0 / 32768.0]
        );
    }
}
