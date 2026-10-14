use crate::audio_out;
use crate::clipboard;
use crate::overlay::{self, OverlayView};
use crate::settings::{self, AppSettings, TtsBackend};
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
use std::time::{Duration, Instant};
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
const SAMPLE_RATE: NonZero<u32> = NonZero::new(tts::SAMPLE_RATE).unwrap();
/// A hotkey release this soon after a ⌘C sent with the hotkey still held is
/// the hotkey listener taking our synthetic ⌘-up for the user letting go.
const COPY_ECHO_WINDOW: Duration = Duration::from_millis(150);

type Chunk = Result<Vec<i16>, String>;

/// One speak session. Its stop flag is also its identity.
struct Session {
    stop: Arc<AtomicBool>,
    /// Until the hotkey goes up, a hotkey press is the held key repeating.
    released_since_start: bool,
    /// When the capture's ⌘C went out with ⌥, ⌃ or ⇧ still down: the only
    /// time a hotkey listener can echo it as a release.
    copied_while_held_at: Option<Instant>,
}

impl Session {
    fn new(stop: Arc<AtomicBool>, copied_while_held_at: Option<Instant>) -> Self {
        Self {
            stop,
            released_since_start: false,
            copied_while_held_at,
        }
    }

    /// Records a hotkey release unless it is the echo of our own ⌘C. Returns
    /// whether it counted.
    fn note_release(&mut self, released_at: Instant) -> bool {
        if is_copy_echo(released_at, self.copied_while_held_at) {
            return false;
        }
        self.released_since_start = true;
        true
    }
}

/// What asked to speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trigger {
    /// A held hotkey repeats its press.
    Hotkey,
    /// Tray, CLI or signal.
    Other,
}

/// Whether a press while speaking stops the speech. A hotkey press does only
/// once the key has been released, so holding it never stops what it started.
fn should_toggle_stop(released_since_start: bool, trigger: Trigger) -> bool {
    trigger == Trigger::Other || released_since_start
}

/// Whether a release at `released_at` is the echo of a ⌘C sent at
/// `copied_while_held_at`.
fn is_copy_echo(released_at: Instant, copied_while_held_at: Option<Instant>) -> bool {
    copied_while_held_at.is_some_and(|copied_at| {
        released_at
            .checked_duration_since(copied_at)
            .is_some_and(|after| after <= COPY_ECHO_WINDOW)
    })
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
    request_speech(app, source, Trigger::Other);
}

fn request_speech(app: &AppHandle, source: &str, trigger: Trigger) {
    let handle = app.clone();
    let source = source.to_string();
    // Never wait for this: the handy-keys thread calling it is the one the
    // main thread's shortcut commands wait on.
    if let Err(e) =
        app.run_on_main_thread(move || speak_selection_on_main(&handle, &source, trigger))
    {
        error!("Failed to schedule speech: {e}");
    }
}

/// Lets the next hotkey press stop the current speech.
fn hotkey_released(app: &AppHandle) {
    // Timed before posting: a capture can hold up the main thread for a while.
    let released_at = Instant::now();
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || hotkey_released_on_main(&handle, released_at)) {
        error!("Failed to schedule the hotkey release: {e}");
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

fn speak_selection_on_main(app: &AppHandle, source: &str, trigger: Trigger) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    let released = state
        .lock()
        .as_ref()
        .map(|session| session.released_since_start);
    if let Some(released_since_start) = released {
        if should_toggle_stop(released_since_start, trigger) {
            info!("Stopping speech ({source})");
            stop_speaking_on_main(app);
        } else {
            debug!("Ignoring a repeat while held ({source})");
        }
        return;
    }

    info!("Speaking selection ({source})");
    let settings = settings::get_settings(app);
    // Checked before the copy: without its files the local voice cannot speak.
    let backend = match speech_backend(app, &settings) {
        Ok(backend) => backend,
        Err(view) => {
            overlay::show_overlay(app, view);
            return;
        }
    };
    let capture_started = Instant::now();
    let selection = match clipboard::capture_selection(app) {
        Ok(selection) => selection,
        Err(e) => {
            error!("Failed to capture the selection: {e}");
            overlay::show_overlay(app, OverlayView::Error { message: e });
            return;
        }
    };
    let text = selection
        .text
        .map(|text| tts::text::normalize(&text))
        .unwrap_or_default();
    let mut sentences = tts::text::split_sentences(&text, backend.max_chunk_chars());
    if matches!(backend, tts::Backend::Local(_)) {
        sentences = tts::text::split_first_clause(sentences);
    }
    let captured_at = Instant::now();
    info!(
        "Captured {} chars, {} sentences in {} ms",
        text.chars().count(),
        sentences.len(),
        (captured_at - capture_started).as_millis()
    );
    if sentences.is_empty() {
        overlay::show_overlay(app, OverlayView::NothingSelected);
        return;
    }

    let stop = Arc::new(AtomicBool::new(false));
    let speech = Speech {
        sentences,
        backend,
        captured_at,
    };
    if let Err(e) = start_speech(app, settings, speech, &stop) {
        error!("Failed to start speech threads: {e}");
        stop.store(true, Ordering::SeqCst);
        let message = e.to_string();
        overlay::show_overlay(app, OverlayView::Error { message });
        return;
    }
    *state.lock() = Some(Session::new(stop, selection.copied_while_held_at));
    overlay::show_overlay(app, OverlayView::Speaking);
    tray::set_tray_state(app, TrayIconState::Speaking);
}

fn hotkey_released_on_main(app: &AppHandle, released_at: Instant) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    let echoed = state
        .lock()
        .as_mut()
        .is_some_and(|session| !session.note_release(released_at));
    if echoed {
        debug!("Ignoring the hotkey release echoed by our ⌘C");
    }
}

fn stop_speaking_on_main(app: &AppHandle) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    if let Some(session) = state.lock().take() {
        session.stop.store(true, Ordering::SeqCst);
    }
    overlay::hide_overlay(app);
    tray::set_tray_state(app, TrayIconState::Idle);
}

/// Ends the session that owns `stop`, unless it already ended.
fn end_session_on_main(app: &AppHandle, stop: &Arc<AtomicBool>, end: PlaybackEnd) {
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
    match end {
        PlaybackEnd::Done => overlay::hide_overlay(app),
        PlaybackEnd::NothingSpoken => {
            info!("No sentence had speakable text");
            overlay::show_overlay(app, OverlayView::NothingSelected);
        }
        PlaybackEnd::Failed(message) => overlay::show_overlay(app, OverlayView::Error { message }),
    }
    tray::set_tray_state(app, TrayIconState::Idle);
}

/// The backend a request will use, or the notice to show instead.
fn speech_backend(app: &AppHandle, settings: &AppSettings) -> Result<tts::Backend, OverlayView> {
    match settings.tts_backend {
        TtsBackend::Http => Ok(tts::Backend::Http {
            base_url: settings.tts_base_url.clone(),
            voice: settings.tts_voice.clone(),
        }),
        TtsBackend::Local => {
            let files = local_model_files(app, settings)
                .map_err(|message| OverlayView::Error { message })?;
            match files.missing() {
                Some(missing) => {
                    info!("The local voice is missing {missing}");
                    Err(OverlayView::ModelMissing)
                }
                None => Ok(tts::Backend::Local(files)),
            }
        }
    }
}

fn local_model_files(
    app: &AppHandle,
    settings: &AppSettings,
) -> Result<tts::local::ModelFiles, String> {
    let models_dir = crate::commands::models_dir(app)?;
    Ok(
        tts::local::ModelFiles::new(&models_dir, settings.tts_model_variant, &settings.tts_voice)
            .with_espeak(tts::espeak::find_if_enabled(settings.tts_espeak_fallback)),
    )
}

/// Loads the local voice in the background when the settings select it, so
/// the next request does not wait for it, and frees it when they do not or
/// its files are gone.
pub fn preload_local_voice(app: &AppHandle) {
    let app = app.clone();
    let spawned = thread::Builder::new()
        .name("tts-preload".into())
        .spawn(move || {
            let result = tts::local::preload(|| {
                let settings = settings::get_settings(&app);
                if settings.tts_backend != TtsBackend::Local {
                    return None;
                }
                let files = local_model_files(&app, &settings)
                    .inspect_err(|e| warn!("Cannot find the local voice: {e}"))
                    .ok()?;
                match files.missing() {
                    Some(missing) => {
                        info!("The local voice is missing {missing}");
                        None
                    }
                    None => Some(files),
                }
            });
            if let Err(message) = result {
                info!("The local voice is not loaded: {message}");
            }
            // The Models page shows what this preload found.
            crate::commands::models::files_changed(&app);
        });
    if let Err(e) = spawned {
        warn!("Failed to start loading the local voice: {e}");
    }
}

/// What a session speaks and how.
struct Speech {
    sentences: Vec<tts::text::Chunk>,
    backend: tts::Backend,
    /// When the selection was ready, for the first-audio timing.
    captured_at: Instant,
}

fn start_speech(
    app: &AppHandle,
    settings: AppSettings,
    speech: Speech,
    stop: &Arc<AtomicBool>,
) -> std::io::Result<()> {
    // The settings commands clamp too, but the store can be edited by hand.
    let speed = settings.tts_speed.clamp(0.5, 2.0);
    let volume = settings.tts_volume.clamp(0.0, 1.0);
    let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
    {
        let app = app.clone();
        let stop = stop.clone();
        let device = settings.selected_output_device;
        let captured_at = speech.captured_at;
        thread::Builder::new()
            .name("tts-playback".into())
            .spawn(move || play(app, stop, rx, device, volume, captured_at))?;
    }
    let stop = stop.clone();
    let Speech {
        sentences, backend, ..
    } = speech;
    thread::Builder::new()
        .name("tts-synth".into())
        .spawn(move || synthesize(sentences, backend, speed, stop, tx))?;
    Ok(())
}

/// Synth thread: one sentence at a time, handed to playback in order.
fn synthesize(
    sentences: Vec<tts::text::Chunk>,
    backend: tts::Backend,
    speed: f32,
    stop: Arc<AtomicBool>,
    tx: SyncSender<Chunk>,
) {
    let _report_panic = ReportPanic(&tx);
    let mut synth = match backend.open(&stop) {
        Ok(synth) => synth,
        Err(_) if stop.load(Ordering::SeqCst) => return,
        Err(e) => {
            let _ = tx.send(Err(e));
            return;
        }
    };
    for sentence in &sentences {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let started = Instant::now();
        let chunk = match &mut synth {
            // The only block_on in the app: it panics on an async runtime
            // worker, and this is a plain thread.
            tts::Synth::Http {
                client,
                base_url,
                voice,
            } => tauri::async_runtime::block_on(tts::http::synthesize(
                client,
                base_url,
                voice,
                speed,
                &sentence.text,
            )),
            tts::Synth::Local(local) => {
                local.synthesize(&sentence.text, speed, sentence.ends_sentence)
            }
        };
        // Stopping cuts a local inference short; that is not a failure.
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let chars = sentence.text.chars().count();
        let unspeakable = match &chunk {
            Ok(samples) => samples.is_empty(),
            Err(e) => tts::http::is_unspeakable(e),
        };
        if unspeakable {
            debug!("Skipping a sentence with no speakable text ({chars} chars)");
            continue;
        }
        if chunk.is_ok() {
            debug!(
                "Synthesised {chars} chars in {} ms",
                started.elapsed().as_millis()
            );
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

/// How playback ended, which decides what the overlay shows next.
#[derive(Debug, PartialEq, Eq)]
enum PlaybackEnd {
    /// The audio ran out or speech was stopped.
    Done,
    /// Synthesis finished without sending any audio: every sentence was
    /// skipped as unspeakable.
    NothingSpoken,
    /// Synthesis failed; the audio queued before the failure has played.
    Failed(String),
}

/// Ends the session when the playback thread exits for any reason, including
/// a panic or a missing output device.
struct SessionGuard {
    app: AppHandle,
    stop: Arc<AtomicBool>,
    end: PlaybackEnd,
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
        let end = std::mem::replace(&mut self.end, PlaybackEnd::Done);
        if let Err(e) = self
            .app
            .run_on_main_thread(move || end_session_on_main(&app, &stop, end))
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
    captured_at: Instant,
) {
    let mut guard = SessionGuard {
        app,
        stop: stop.clone(),
        end: PlaybackEnd::Done,
    };
    let mut sink = match audio_out::open_output(device.as_deref()) {
        Ok(sink) => sink,
        Err(e) => {
            error!("Cannot play speech: {e}");
            guard.end = PlaybackEnd::Failed(e);
            return;
        }
    };
    sink.log_on_drop(false);
    let player = Player::connect_new(sink.mixer());
    player.set_volume(volume);
    guard.end = play_chunks(&player, &rx, &stop, captured_at);
}

/// Plays chunks from `rx` until they run out, synthesis fails or `stop` is
/// set. Returns a synthesis failure once the audio queued before it has played.
fn play_chunks(
    player: &Player,
    rx: &Receiver<Chunk>,
    stop: &AtomicBool,
    captured_at: Instant,
) -> PlaybackEnd {
    let mut queued_any = false;
    loop {
        if stop.load(Ordering::SeqCst) {
            player.stop();
            return PlaybackEnd::Done;
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
                    return PlaybackEnd::Done;
                }
                player.append(SamplesBuffer::new(MONO, SAMPLE_RATE, pcm_to_f32(&samples)));
                if !queued_any {
                    info!(
                        "First audio {} ms after the capture",
                        captured_at.elapsed().as_millis()
                    );
                }
                queued_any = true;
            }
            Ok(Err(message)) => {
                error!("Speech synthesis failed: {message}");
                play_out(player, stop);
                return PlaybackEnd::Failed(message);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                play_out(player, stop);
                return if queued_any {
                    PlaybackEnd::Done
                } else {
                    PlaybackEnd::NothingSpoken
                };
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

/// Toggle: a press speaks the selection, or stops speech once the key has
/// been released since that speech started.
struct SpeakAction;

impl ShortcutAction for SpeakAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, shortcut_str: &str) {
        request_speech(app, shortcut_str, Trigger::Hotkey);
    }

    fn stop(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        hotkey_released(app);
    }
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
    fn only_tray_cli_and_signals_stop_speech_before_a_hotkey_release() {
        assert!(!should_toggle_stop(false, Trigger::Hotkey));
        assert!(should_toggle_stop(true, Trigger::Hotkey));
        assert!(should_toggle_stop(false, Trigger::Other));
        assert!(should_toggle_stop(true, Trigger::Other));
    }

    #[test]
    fn copy_echo_window_is_the_150_ms_after_a_copy_sent_while_held() {
        let copied_at = Instant::now();
        let after = |ms| copied_at + Duration::from_millis(ms);
        assert!(is_copy_echo(copied_at, Some(copied_at)));
        assert!(is_copy_echo(after(50), Some(copied_at)));
        assert!(is_copy_echo(after(150), Some(copied_at)));
        assert!(!is_copy_echo(after(151), Some(copied_at)));
        assert!(
            !is_copy_echo(copied_at, Some(after(10))),
            "a release before the copy is the user letting go"
        );
        assert!(
            !is_copy_echo(after(50), None),
            "a copy sent with the modifiers up has no echo"
        );
    }

    #[test]
    fn holding_the_hotkey_never_stops_the_speech_it_started() {
        let pressed_at = Instant::now();
        let after = |ms| pressed_at + Duration::from_millis(ms);
        // Modifiers still down when the wait gave up and ⌘C went out.
        let mut session = Session::new(Arc::new(AtomicBool::new(false)), Some(after(300)));

        assert!(
            !session.note_release(after(340)),
            "the ⌘-up echo must not count"
        );
        assert!(
            !should_toggle_stop(session.released_since_start, Trigger::Hotkey),
            "the key repeat the echo lets through must not stop"
        );
        assert!(session.note_release(after(2000)));
        assert!(should_toggle_stop(
            session.released_since_start,
            Trigger::Hotkey
        ));
    }

    #[test]
    fn a_quick_tap_counts_its_release_however_close_to_the_copy() {
        let copied_at = Instant::now();
        // Modifiers were up by the copy, so there is no echo window.
        let mut session = Session::new(Arc::new(AtomicBool::new(false)), None);
        assert!(session.note_release(copied_at + Duration::from_millis(20)));
        assert!(should_toggle_stop(
            session.released_since_start,
            Trigger::Hotkey
        ));
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
            let playback =
                scope.spawn(move || play_chunks(player_ref, &rx, stop_ref, Instant::now()));
            // Nothing is consuming the audio yet, so the error must wait.
            thread::sleep(PLAYBACK_POLL * 10);
            assert!(!playback.is_finished());
            assert_eq!(player.len(), 1);

            // Stand in for the device and play it out.
            output.by_ref().take(48_000).for_each(drop);
            assert_eq!(
                playback.join().unwrap(),
                PlaybackEnd::Failed("server error".into())
            );
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
            let playback =
                scope.spawn(move || play_chunks(player_ref, &rx, stop_ref, Instant::now()));
            thread::sleep(PLAYBACK_POLL * 10);
            assert!(!playback.is_finished());

            stop.store(true, Ordering::SeqCst);
            assert_eq!(
                playback.join().unwrap(),
                PlaybackEnd::Failed("server error".into())
            );
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
            let playback =
                scope.spawn(move || play_chunks(player_ref, &rx, stop_ref, Instant::now()));
            thread::sleep(PLAYBACK_POLL * 10);
            assert!(
                !playback.is_finished(),
                "playback must wait for queued audio even after the sender is gone"
            );

            output.by_ref().take(48_000).for_each(drop);
            assert_eq!(playback.join().unwrap(), PlaybackEnd::Done);
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
            let playback =
                scope.spawn(move || play_chunks(player_ref, &rx, stop_ref, Instant::now()));
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

        assert_eq!(
            play_chunks(&player, &rx, &stop, Instant::now()),
            PlaybackEnd::Done
        );
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
            let playback =
                scope.spawn(move || play_chunks(player_ref, &rx, stop_ref, Instant::now()));
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
            assert_eq!(playback.join().unwrap(), PlaybackEnd::Done);
        });
    }

    #[test]
    fn synthesis_that_sent_no_audio_ends_as_nothing_spoken() {
        let (player, _output) = Player::new();
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        drop(tx);

        let stop = AtomicBool::new(false);
        assert_eq!(
            play_chunks(&player, &rx, &stop, Instant::now()),
            PlaybackEnd::NothingSpoken
        );
        // A stopped session ends quietly even when nothing played.
        stop.store(true, Ordering::SeqCst);
        assert_eq!(
            play_chunks(&player, &rx, &stop, Instant::now()),
            PlaybackEnd::Done
        );
    }
}
