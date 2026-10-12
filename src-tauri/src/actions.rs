use crate::audio_out;
use crate::clipboard;
use crate::overlay::{self, OverlayView};
use crate::settings;
use crate::tray::{self, TrayIconState};
use crate::tts;
use log::{debug, error, info, warn};
use once_cell::sync::Lazy;
use rodio::buffer::SamplesBuffer;
use rodio::Player;
use std::collections::HashMap;
use std::num::NonZero;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Most sentences the player holds, counting the one playing. Also the
/// channel capacity, so synthesis stays only a few sentences ahead.
const READ_AHEAD: usize = 2;
const PLAYBACK_POLL: Duration = Duration::from_millis(20);
const MONO: NonZero<u16> = NonZero::<u16>::MIN;
const SAMPLE_RATE: NonZero<u32> = NonZero::new(tts::http::SAMPLE_RATE).unwrap();

type Chunk = Result<Vec<i16>, String>;

/// One speak session. Its stop flag is also its identity.
struct Session {
    stop: Arc<AtomicBool>,
}

/// The current speak session, if any. Only the main thread locks it, so
/// nothing the main thread waits on can be holding it. The `*_on_main` fns
/// must never call the public entry points: on the main thread those run
/// inline and would lock it again.
#[derive(Default)]
pub struct TtsState {
    current: Mutex<Option<Session>>,
}

impl TtsState {
    fn lock(&self) -> MutexGuard<'_, Option<Session>> {
        self.current.lock().unwrap_or_else(|poisoned| {
            warn!("TTS state mutex was poisoned, recovering");
            poisoned.into_inner()
        })
    }
}

/// Speaks the current selection, or stops if already speaking. A press during
/// the capture runs after it, so it stops the speech that capture started.
pub fn speak_selection(app: &AppHandle, source: &str) {
    let handle = app.clone();
    let source = source.to_string();
    // Never wait for this: the handy-keys thread calling it is the one the
    // main thread's shortcut commands wait on.
    if let Err(e) = app.run_on_main_thread(move || speak_selection_on_main(&handle, &source)) {
        error!("Failed to schedule speech: {e}");
    }
}

/// Stops any speech in progress and hides the overlay.
pub fn stop_speaking(app: &AppHandle, source: &str) {
    info!("Stopping speech ({source})");
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || stop_speaking_on_main(&handle)) {
        error!("Failed to schedule stopping speech: {e}");
    }
}

fn speak_selection_on_main(app: &AppHandle, source: &str) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    if state.lock().is_some() {
        info!("Stopping speech ({source})");
        stop_speaking_on_main(app);
        return;
    }

    info!("Speaking selection ({source})");
    let text = match clipboard::capture_selection(app) {
        Ok(text) => text
            .map(|text| tts::text::normalize(&text))
            .unwrap_or_default(),
        Err(e) => {
            error!("Failed to capture the selection: {e}");
            overlay::show_overlay(app, OverlayView::Error { message: e });
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
        overlay::show_overlay(app, OverlayView::NothingSelected);
        return;
    }

    let stop = Arc::new(AtomicBool::new(false));
    if let Err(e) = start_speech(app, sentences, &stop) {
        error!("Failed to start speech threads: {e}");
        stop.store(true, Ordering::SeqCst);
        let message = e.to_string();
        overlay::show_overlay(app, OverlayView::Error { message });
        return;
    }
    *state.lock() = Some(Session { stop });
    overlay::show_overlay(app, OverlayView::Speaking);
    tray::set_tray_state(app, TrayIconState::Speaking);
}

fn stop_speaking_on_main(app: &AppHandle) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    if let Some(session) = state.lock().take() {
        session.stop.store(true, Ordering::SeqCst);
    }
    overlay::hide_recording_overlay(app);
    tray::set_tray_state(app, TrayIconState::Idle);
}

/// Ends the session that owns `stop`, unless it already ended.
fn end_session_on_main(app: &AppHandle, stop: &Arc<AtomicBool>, error: Option<String>) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    {
        let mut current = state.lock();
        let owns = current
            .as_ref()
            .is_some_and(|session| Arc::ptr_eq(&session.stop, stop));
        if !owns {
            return;
        }
        *current = None;
    }
    match error {
        Some(message) => overlay::show_overlay(app, OverlayView::Error { message }),
        None => overlay::hide_recording_overlay(app),
    }
    tray::set_tray_state(app, TrayIconState::Idle);
}

fn start_speech(
    app: &AppHandle,
    sentences: Vec<String>,
    stop: &Arc<AtomicBool>,
) -> std::io::Result<()> {
    let settings = settings::get_settings(app);
    // The settings commands clamp too, but the store can be edited by hand.
    let speed = settings.tts_speed.clamp(0.5, 2.0);
    let volume = settings.tts_volume.clamp(0.0, 1.0);
    let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
    {
        let app = app.clone();
        let stop = stop.clone();
        let device = settings.selected_output_device;
        thread::Builder::new()
            .name("tts-playback".into())
            .spawn(move || play(app, stop, rx, device, volume))?;
    }
    let stop = stop.clone();
    let (base_url, voice) = (settings.tts_base_url, settings.tts_voice);
    thread::Builder::new()
        .name("tts-synth".into())
        .spawn(move || synthesize(sentences, base_url, voice, speed, stop, tx))?;
    Ok(())
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
    let _report_panic = ReportPanic(&tx);
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
        if let Err(e) = &chunk {
            if tts::http::is_unspeakable(e) {
                debug!("Skipping a sentence with no speakable text: {sentence:?}");
                continue;
            }
        }
        let failed = chunk.is_err();
        // A send error means playback has ended.
        if tx.send(chunk).is_err() || failed {
            break;
        }
    }
}

/// Reports a synth thread panic to playback, so speech that stops short shows
/// an error instead of ending as if it had finished.
struct ReportPanic<'a>(&'a SyncSender<Chunk>);

impl Drop for ReportPanic<'_> {
    fn drop(&mut self) {
        if thread::panicking() {
            let _ = self.0.send(Err("synthesis failed".into()));
        }
    }
}

/// Ends the session when the playback thread exits for any reason, including
/// a panic or a missing output device.
struct SessionGuard {
    app: AppHandle,
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
        let app = self.app.clone();
        let stop = self.stop.clone();
        let error = self.error.take();
        if let Err(e) = self
            .app
            .run_on_main_thread(move || end_session_on_main(&app, &stop, error))
        {
            warn!("Failed to end the speech session: {e}");
        }
    }
}

/// Playback thread: sole owner of the output device and the player.
fn play(
    app: AppHandle,
    stop: Arc<AtomicBool>,
    rx: Receiver<Chunk>,
    device: Option<String>,
    volume: f32,
) {
    let mut guard = SessionGuard {
        app,
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
    guard.error = play_chunks(&player, &rx, &stop);
}

/// Plays chunks from `rx` until they run out, synthesis fails or `stop` is
/// set. Returns the synthesis error once the audio queued before it has played.
fn play_chunks(player: &Player, rx: &Receiver<Chunk>, stop: &AtomicBool) -> Option<String> {
    loop {
        if stop.load(Ordering::SeqCst) {
            player.stop();
            return None;
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
                    return None;
                }
                player.append(SamplesBuffer::new(MONO, SAMPLE_RATE, pcm_to_f32(&samples)));
            }
            Ok(Err(message)) => {
                error!("Speech synthesis failed: {message}");
                play_out(player, stop);
                return Some(message);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                play_out(player, stop);
                return None;
            }
        }
    }
}

/// Waits for the queued audio to finish, or for `stop`.
fn play_out(player: &Player, stop: &AtomicBool) {
    while !player.empty() && !stop.load(Ordering::SeqCst) {
        thread::sleep(PLAYBACK_POLL);
    }
    player.stop();
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

    /// Lets a failed assertion end the playback thread instead of hanging the scope.
    struct StopOnDrop<'a>(&'a AtomicBool);

    impl Drop for StopOnDrop<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

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

    #[test]
    fn synth_panic_reaches_playback_as_an_error() {
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        let synth = thread::spawn(move || {
            let _report_panic = ReportPanic(&tx);
            panic!("synth thread panic (expected by this test)");
        });
        assert!(synth.join().is_err());
        assert_eq!(rx.recv(), Ok(Err("synthesis failed".to_string())));
    }

    #[test]
    fn synthesis_error_waits_for_queued_audio_to_play() {
        let (player, mut output) = Player::new();
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        tx.send(Ok(vec![0; 480])).unwrap();
        tx.send(Err("server error".into())).unwrap();
        let stop = AtomicBool::new(false);

        let (player_ref, stop_ref) = (&player, &stop);
        thread::scope(|scope| {
            let playback = scope.spawn(move || play_chunks(player_ref, &rx, stop_ref));
            // Nothing is consuming the audio yet, so the error must wait.
            thread::sleep(PLAYBACK_POLL * 10);
            assert!(!playback.is_finished());
            assert_eq!(player.len(), 1);

            // Stand in for the device and play it out.
            output.by_ref().take(48_000).for_each(drop);
            assert_eq!(playback.join().unwrap(), Some("server error".into()));
            assert!(player.empty());
        });
    }

    #[test]
    fn stop_ends_the_wait_for_queued_audio() {
        let (player, _output) = Player::new();
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        tx.send(Ok(vec![0; 480])).unwrap();
        tx.send(Err("server error".into())).unwrap();
        let stop = AtomicBool::new(false);

        let (player_ref, stop_ref) = (&player, &stop);
        thread::scope(|scope| {
            let playback = scope.spawn(move || play_chunks(player_ref, &rx, stop_ref));
            thread::sleep(PLAYBACK_POLL * 10);
            assert!(!playback.is_finished());

            stop.store(true, Ordering::SeqCst);
            assert_eq!(playback.join().unwrap(), Some("server error".into()));
        });
    }

    #[test]

    fn report_panic_sends_exactly_one_error_on_unwind() {
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        let synth = thread::spawn({
            let tx = tx.clone();
            move || {
                let _report_panic = ReportPanic(&tx);
                panic!("synth thread panic (expected by this test)");
            }
        });
        assert!(synth.join().is_err());
        drop(tx);
        assert_eq!(rx.recv(), Ok(Err("synthesis failed".to_string())));
        assert!(
            rx.recv().is_err(),
            "a panic must be reported once, then the channel closes"
        );
    }

    #[test]
    fn report_panic_sends_nothing_on_normal_completion() {
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        {
            let _report_panic = ReportPanic(&tx);
        }
        drop(tx);
        assert!(
            rx.recv().is_err(),
            "a finished synth thread must not report an error"
        );
    }

    #[test]
    fn finished_synthesis_is_not_an_error_and_plays_out_first() {
        let (player, mut output) = Player::new();
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        tx.send(Ok(vec![0; 480])).unwrap();
        drop(tx);
        let stop = AtomicBool::new(false);

        let (player_ref, stop_ref) = (&player, &stop);
        thread::scope(|scope| {
            let _release = StopOnDrop(stop_ref);
            let playback = scope.spawn(move || play_chunks(player_ref, &rx, stop_ref));
            thread::sleep(PLAYBACK_POLL * 10);
            assert!(
                !playback.is_finished(),
                "playback must wait for queued audio even after the sender is gone"
            );

            output.by_ref().take(48_000).for_each(drop);
            assert_eq!(playback.join().unwrap(), None);
        });
    }

    #[test]
    fn stop_mid_drain_returns_within_a_few_poll_ticks() {
        let (player, mut output) = Player::new();
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        tx.send(Ok(vec![0; 480])).unwrap();
        tx.send(Err("server error".into())).unwrap();
        let stop = AtomicBool::new(false);

        let (player_ref, stop_ref) = (&player, &stop);
        thread::scope(|scope| {
            let _release = StopOnDrop(stop_ref);
            let playback = scope.spawn(move || play_chunks(player_ref, &rx, stop_ref));
            thread::sleep(PLAYBACK_POLL * 10);
            assert!(!playback.is_finished());

            let stopped_at = std::time::Instant::now();
            stop.store(true, Ordering::SeqCst);
            playback.join().unwrap();
            assert!(
                stopped_at.elapsed() < PLAYBACK_POLL * 10,
                "stop took {:?}; it must not wait for the audio to finish",
                stopped_at.elapsed()
            );
            // The device drops stopped audio on its next read.
            output.by_ref().take(1_000).for_each(drop);
            assert!(player.empty(), "stop must clear the queued audio");
        });
    }

    #[test]
    fn stop_before_a_chunk_arrives_never_queues_it() {
        let (player, _output) = Player::new();
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        tx.send(Ok(vec![0; 480])).unwrap();
        let stop = AtomicBool::new(true);

        assert_eq!(play_chunks(&player, &rx, &stop), None);
        assert!(
            player.empty(),
            "appending after stop would restart a stopped player"
        );
    }

    #[test]
    fn playback_holds_at_most_read_ahead_chunks() {
        let (player, _output) = Player::new();
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        let stop = AtomicBool::new(false);

        let (player_ref, stop_ref) = (&player, &stop);
        thread::scope(|scope| {
            let _release = StopOnDrop(stop_ref);
            let playback = scope.spawn(move || play_chunks(player_ref, &rx, stop_ref));
            // Player takes READ_AHEAD, the channel holds READ_AHEAD more.
            for _ in 0..READ_AHEAD * 2 {
                tx.send(Ok(vec![0; 480])).unwrap();
            }
            thread::sleep(PLAYBACK_POLL * 10);
            assert_eq!(
                player.len(),
                READ_AHEAD,
                "nothing is playing out, so the player must not take more"
            );
            assert!(
                matches!(
                    tx.try_send(Ok(vec![0; 480])),
                    Err(mpsc::TrySendError::Full(_))
                ),
                "the unread chunks must stay in the channel so synthesis blocks"
            );

            stop.store(true, Ordering::SeqCst);
            assert_eq!(playback.join().unwrap(), None);
        });
    }
}
