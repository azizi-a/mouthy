use crate::audio_out;
use crate::clipboard::{self, ClipboardText};
use crate::overlay::{self, OverlayView};
use crate::settings::{self, AppSettings, TtsBackend};
use crate::tray::{self, TrayIconState};
use crate::tts;
use log::{debug, error, info, warn};
use once_cell::sync::Lazy;
use rodio::buffer::SamplesBuffer;
use rodio::Player;
use serde::Serialize;
use std::collections::HashMap;
use std::num::NonZero;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

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
/// Most clipboard characters read aloud. Normalising and splitting run on the
/// main thread, which a huge clipboard would stall.
const CLIPBOARD_CHAR_CAP: usize = 20_000;
/// What a voice preview reads. English in every app language, since every
/// voice reads English.
const PREVIEW_TEXT: &str = "Hi. This is how I sound when I read your text aloud.";

type Chunk = Result<Vec<i16>, String>;

/// One speak session. Its stop flag is also its identity.
struct Session {
    stop: Arc<AtomicBool>,
    /// Until the hotkey goes up, a hotkey press is the held key repeating.
    released_since_start: bool,
    /// When the capture's ⌘C went out with ⌥, ⌃ or ⇧ still down: the only
    /// time a hotkey listener can echo it as a release.
    copied_while_held_at: Option<Instant>,
    /// What it reads, for the notice when none of it was speakable.
    input: Input,
}

impl Session {
    fn new(stop: Arc<AtomicBool>, copied_while_held_at: Option<Instant>, input: Input) -> Self {
        Self {
            stop,
            released_since_start: false,
            copied_while_held_at,
            input,
        }
    }

    /// A voice preview. No hotkey started it, so the first press stops it.
    fn preview(stop: Arc<AtomicBool>) -> Self {
        Self {
            stop,
            released_since_start: true,
            copied_while_held_at: None,
            input: Input::Preview,
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

/// Where the text to speak comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Input {
    /// Copied from the frontmost app with ⌘C, leaving the clipboard as it was.
    Selection,
    /// The clipboard as it is; nothing is copied.
    Clipboard,
    /// `PREVIEW_TEXT`, in the voice being previewed.
    Preview,
}

impl Input {
    fn name(self) -> &'static str {
        match self {
            Input::Selection => "selection",
            Input::Clipboard => "clipboard",
            Input::Preview => "voice preview",
        }
    }

    /// The notice when there was nothing to speak.
    fn empty_notice(self) -> OverlayView {
        match self {
            Input::Selection => OverlayView::NothingSelected,
            Input::Clipboard => OverlayView::ClipboardEmpty,
            Input::Preview => OverlayView::Error {
                message: "The voice preview had nothing to speak".into(),
            },
        }
    }
}

/// `text` cut to its first `cap` characters.
fn truncate_chars(mut text: String, cap: usize) -> String {
    if let Some((end, _)) = text.char_indices().nth(cap) {
        text.truncate(end);
    }
    text
}

fn cap_clipboard_text(text: String) -> String {
    let full_len = text.len();
    let text = truncate_chars(text, CLIPBOARD_CHAR_CAP);
    if text.len() < full_len {
        info!("Reading the first {CLIPBOARD_CHAR_CAP} characters of the clipboard");
    }
    text
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
/// must never call the public entry points (`speak_selection`,
/// `speak_clipboard`, `preview_voice`, `stop_speaking`): on the main thread
/// those run inline and would lock it again.
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
    request_speech(app, source, Trigger::Other, Input::Selection);
}

/// Speaks the clipboard text, or stops if already speaking.
pub fn speak_clipboard(app: &AppHandle, source: &str) {
    request_speech(app, source, Trigger::Other, Input::Clipboard);
}

/// Reads `PREVIEW_TEXT` in `voice`, stopping any speech first. Fails, stopping
/// nothing, when the voice is not valid or the local engine lacks a file it
/// needs.
pub fn preview_voice(app: &AppHandle, voice: &str) -> Result<(), String> {
    let settings = settings::get_settings(app);
    // Checked as `change_tts_voice_setting` checks a voice before storing it.
    let voice = match settings.tts_backend {
        TtsBackend::Local => tts::voice::VoiceSpec::parse(voice)?.to_string(),
        TtsBackend::Http => voice.trim().to_string(),
    };
    if voice.is_empty() {
        return Err("No voice to preview".into());
    }
    let backend =
        speech_backend(app, &settings, &voice).map_err(|view| preview_error(view, &voice))?;
    info!("Previewing the voice {voice}");
    let speech = Speech {
        sentences: sentences_for(&tts::text::normalize(PREVIEW_TEXT), &backend),
        backend,
        captured_at: Instant::now(),
    };
    let (send_started, started) = mpsc::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = send_started.send(preview_on_main(&handle, settings, speech));
    })
    .map_err(|e| format!("Failed to schedule the voice preview: {e}"))?;
    // Called on the main thread, as the synchronous command is, the preview
    // has started inline by now. From another thread it starts later, and a
    // failure to start shows only on the overlay.
    started.try_recv().unwrap_or(Ok(()))
}

/// The message for a notice `speech_backend` gave instead of a backend.
fn preview_error(view: OverlayView, voice: &str) -> String {
    match view {
        OverlayView::Error { message } => message,
        _ => format!("Offline voice files for '{voice}' are missing"),
    }
}

fn request_speech(app: &AppHandle, source: &str, trigger: Trigger, input: Input) {
    let handle = app.clone();
    let source = source.to_string();
    // Never wait for this: the handy-keys thread calling it is the one the
    // main thread's shortcut commands wait on.
    if let Err(e) =
        app.run_on_main_thread(move || speak_selection_on_main(&handle, &source, trigger, input))
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

fn speak_selection_on_main(app: &AppHandle, source: &str, trigger: Trigger, input: Input) {
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

    info!("Speaking the {} ({source})", input.name());
    let settings = settings::get_settings(app);
    // Checked before the copy: without its files the local voice cannot speak.
    let backend = match speech_backend(app, &settings, &settings.tts_voice) {
        Ok(backend) => backend,
        Err(view) => {
            overlay::show_overlay(app, view);
            return;
        }
    };
    let capture_started = Instant::now();
    let captured = match input {
        Input::Selection => clipboard::capture_selection(app)
            .map(|selection| (selection.text, selection.copied_while_held_at)),
        // No ⌘C goes out, so there is no copy echo to ignore.
        Input::Clipboard => match clipboard::read_text() {
            Ok(ClipboardText::Text(text)) => Ok((Some(cap_clipboard_text(text)), None)),
            Ok(ClipboardText::Empty) => Ok((None, None)),
            Ok(ClipboardText::Secret) => {
                overlay::show_overlay(app, OverlayView::ClipboardSecret);
                return;
            }
            Err(e) => Err(e),
        },
        Input::Preview => Ok((Some(PREVIEW_TEXT.to_string()), None)),
    };
    let (text, copied_while_held_at) = match captured {
        Ok(captured) => captured,
        Err(e) => {
            error!("Failed to read the {}: {e}", input.name());
            overlay::show_overlay(app, OverlayView::Error { message: e });
            return;
        }
    };
    let text = text
        .map(|text| tts::text::normalize(&text))
        .unwrap_or_default();
    let sentences = sentences_for(&text, &backend);
    let captured_at = Instant::now();
    info!(
        "Captured {} chars, {} sentences in {} ms",
        text.chars().count(),
        sentences.len(),
        (captured_at - capture_started).as_millis()
    );
    if sentences.is_empty() {
        overlay::show_overlay(app, input.empty_notice());
        return;
    }

    let speech = Speech {
        sentences,
        backend,
        captured_at,
    };
    let stop = Arc::new(AtomicBool::new(false));
    let session = Session::new(stop, copied_while_held_at, input);
    // A failure to start is on the overlay already.
    let _ = start_session_on_main(app, &state, settings, speech, session);
}

fn preview_on_main(app: &AppHandle, settings: AppSettings, speech: Speech) -> Result<(), String> {
    let Some(state) = app.try_state::<TtsState>() else {
        return Err("Speech is not ready yet".into());
    };
    // No preload for a replaced preview: this one loads its own voice next.
    if stop_session_on_main(app, &state).is_some() {
        info!("Stopping speech for a voice preview");
    }
    let session = Session::preview(Arc::new(AtomicBool::new(false)));
    start_session_on_main(app, &state, settings, speech, session)
}

/// Starts the threads for `speech` and makes `session` current, or shows why
/// they could not start and returns it.
fn start_session_on_main(
    app: &AppHandle,
    state: &TtsState,
    settings: AppSettings,
    speech: Speech,
    session: Session,
) -> Result<(), String> {
    if let Err(e) = start_speech(app, settings, speech, &session.stop) {
        error!("Failed to start speech threads: {e}");
        session.stop.store(true, Ordering::SeqCst);
        let message = e.to_string();
        overlay::show_overlay(
            app,
            OverlayView::Error {
                message: message.clone(),
            },
        );
        // A preview may have stopped the session that set it to speaking.
        tray::set_tray_state(app, TrayIconState::Idle);
        return Err(message);
    }
    let input = session.input;
    *state.lock() = Some(session);
    overlay::show_overlay(app, OverlayView::Speaking);
    tray::set_tray_state(app, TrayIconState::Speaking);
    emit_speech_state(app, SessionPhase::Started, input, None);
    Ok(())
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
    if stop_session_on_main(app, &state) == Some(Input::Preview) {
        // So the next hotkey finds the settings voice loaded, not the preview's.
        preload_local_voice(app);
    }
    overlay::hide_overlay(app);
    tray::set_tray_state(app, TrayIconState::Idle);
}

/// Stops and forgets the current session. Returns what it was reading.
fn stop_session_on_main(app: &AppHandle, state: &TtsState) -> Option<Input> {
    let session = state.lock().take()?;
    session.stop.store(true, Ordering::SeqCst);
    emit_speech_state(app, SessionPhase::Ended, session.input, None);
    Some(session.input)
}

/// Ends the session that owns `stop`, unless it already ended.
fn end_session_on_main(app: &AppHandle, stop: &Arc<AtomicBool>, end: PlaybackEnd) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    let input = {
        let mut current = state.lock();
        let Some(input) = owner_input(&current, stop) else {
            return;
        };
        *current = None;
        input
    };
    let error = match end {
        PlaybackEnd::Done => {
            overlay::hide_overlay(app);
            None
        }
        PlaybackEnd::NothingSpoken => {
            info!("No sentence had speakable text");
            overlay::show_overlay(app, input.empty_notice());
            None
        }
        PlaybackEnd::Failed(message) => {
            overlay::show_overlay(
                app,
                OverlayView::Error {
                    message: message.clone(),
                },
            );
            Some(message)
        }
    };
    tray::set_tray_state(app, TrayIconState::Idle);
    emit_speech_state(app, SessionPhase::Ended, input, error);
    if input == Input::Preview {
        // So the next hotkey finds the settings voice loaded, not the preview's.
        preload_local_voice(app);
    }
}

/// Tells the UI the session that owns `stop` has started playing audio, unless
/// it has already ended.
fn speaking_on_main(app: &AppHandle, stop: &Arc<AtomicBool>) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    let input = owner_input(&state.lock(), stop);
    if let Some(input) = input {
        emit_speech_state(app, SessionPhase::Speaking, input, None);
    }
}

/// What the current session reads, if it is the one that owns `stop`.
fn owner_input(current: &Option<Session>, stop: &Arc<AtomicBool>) -> Option<Input> {
    current
        .as_ref()
        .filter(|session| Arc::ptr_eq(&session.stop, stop))
        .map(|session| session.input)
}

/// The `speech-state` event's payload.
#[derive(Clone, Debug, Serialize)]
struct SpeechStateChange {
    state: SessionPhase,
    /// Whether the session is a voice preview.
    preview: bool,
    /// Why synthesis or playback failed; only ever on `ended`.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum SessionPhase {
    Started,
    /// The first audio is playing.
    Speaking,
    Ended,
}

/// Sent from the main thread, so a session's end always reaches the UI before
/// the start of the session that replaced it.
fn emit_speech_state(app: &AppHandle, state: SessionPhase, input: Input, error: Option<String>) {
    let change = SpeechStateChange {
        state,
        preview: input == Input::Preview,
        error,
    };
    if let Err(e) = app.emit("speech-state", change) {
        warn!("Failed to report the speech state: {e}");
    }
}

/// Normalised `text` cut into the requests `backend` gets.
fn sentences_for(text: &str, backend: &tts::Backend) -> Vec<tts::text::Chunk> {
    let sentences = tts::text::split_sentences(text, backend.max_chunk_chars());
    if matches!(backend, tts::Backend::Local(_)) {
        tts::text::split_first_clause(sentences)
    } else {
        sentences
    }
}

/// The backend a request will use, or the notice to show instead.
fn speech_backend(
    app: &AppHandle,
    settings: &AppSettings,
    voice: &str,
) -> Result<tts::Backend, OverlayView> {
    match settings.tts_backend {
        TtsBackend::Http => Ok(http_backend(settings, voice)),
        TtsBackend::Local => {
            let files = local_model_files(app, settings, voice)
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

fn http_backend(settings: &AppSettings, voice: &str) -> tts::Backend {
    tts::Backend::Http {
        base_url: settings.tts_base_url.clone(),
        voice: voice.to_string(),
    }
}

fn local_model_files(
    app: &AppHandle,
    settings: &AppSettings,
    voice: &str,
) -> Result<tts::local::ModelFiles, String> {
    let models_dir = crate::commands::models_dir(app)?;
    Ok(
        tts::local::ModelFiles::new(&models_dir, settings.tts_model_variant, voice)
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
                let files = local_model_files(&app, &settings, &settings.tts_voice)
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
    /// When the text was ready, for the first-audio timing.
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
        app: app.clone(),
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
    // Posted like the session's end, so it is reported only while current.
    let first_audio = || {
        let stop = stop.clone();
        let handle = app.clone();
        if let Err(e) = app.run_on_main_thread(move || speaking_on_main(&handle, &stop)) {
            warn!("Failed to report the first audio: {e}");
        }
    };
    guard.end = play_chunks(&player, &rx, &stop, captured_at, first_audio);
}

/// Plays chunks from `rx` until they run out, synthesis fails or `stop` is
/// set, calling `first_audio` once the first chunk is queued. Returns a
/// synthesis failure once the audio queued before it has played.
fn play_chunks(
    player: &Player,
    rx: &Receiver<Chunk>,
    stop: &AtomicBool,
    captured_at: Instant,
    first_audio: impl FnOnce(),
) -> PlaybackEnd {
    let mut first_audio = Some(first_audio);
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
                if let Some(first_audio) = first_audio.take() {
                    info!(
                        "First audio {} ms after the capture",
                        captured_at.elapsed().as_millis()
                    );
                    first_audio();
                }
            }
            Ok(Err(message)) => {
                error!("Speech synthesis failed: {message}");
                play_out(player, stop);
                return PlaybackEnd::Failed(message);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                play_out(player, stop);
                return if first_audio.is_none() {
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
        request_speech(app, shortcut_str, Trigger::Hotkey, Input::Selection);
    }

    fn stop(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        hotkey_released(app);
    }
}

/// Toggle like `SpeakAction`, reading the clipboard instead of the selection.
struct SpeakClipboardAction;

impl ShortcutAction for SpeakClipboardAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, shortcut_str: &str) {
        request_speech(app, shortcut_str, Trigger::Hotkey, Input::Clipboard);
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
    map.insert(
        "speak_clipboard".to_string(),
        Arc::new(SpeakClipboardAction) as Arc<dyn ShortcutAction>,
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
    fn an_empty_clipboard_has_its_own_notice() {
        assert!(matches!(
            Input::Selection.empty_notice(),
            OverlayView::NothingSelected
        ));
        assert!(matches!(
            Input::Clipboard.empty_notice(),
            OverlayView::ClipboardEmpty
        ));
    }

    #[test]
    fn truncation_keeps_whole_characters() {
        assert_eq!(truncate_chars("héllo wörld".into(), 7), "héllo w");
        assert_eq!(truncate_chars("ab".into(), 5), "ab");
        assert_eq!(truncate_chars("日本語".into(), 2), "日本");
        let long = "é".repeat(CLIPBOARD_CHAR_CAP + 10);
        assert_eq!(cap_clipboard_text(long).chars().count(), CLIPBOARD_CHAR_CAP);
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
        let mut session = Session::new(
            Arc::new(AtomicBool::new(false)),
            Some(after(300)),
            Input::Selection,
        );

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
        let mut session = Session::new(Arc::new(AtomicBool::new(false)), None, Input::Selection);
        assert!(session.note_release(copied_at + Duration::from_millis(20)));
        assert!(should_toggle_stop(
            session.released_since_start,
            Trigger::Hotkey
        ));
    }

    #[test]
    fn the_first_hotkey_press_stops_a_voice_preview() {
        let session = Session::preview(Arc::new(AtomicBool::new(false)));
        assert_eq!(session.input, Input::Preview);
        assert!(should_toggle_stop(
            session.released_since_start,
            Trigger::Hotkey
        ));
    }

    #[test]
    fn both_engines_speak_the_preview_sentence() {
        let text = tts::text::normalize(PREVIEW_TEXT);
        let http = http_backend(&settings::get_default_settings(), "af_alloy");
        let local = tts::Backend::Local(tts::local::ModelFiles::new(
            std::path::Path::new("models"),
            settings::ModelVariant::Int8,
            "af_alloy",
        ));
        for backend in [http, local] {
            let sentences = sentences_for(&text, &backend);
            assert!(!sentences.is_empty());
            assert!(sentences
                .iter()
                .all(|s| s.text.chars().count() <= backend.max_chunk_chars()));
        }
    }

    #[test]
    fn the_server_gets_the_voice_asked_for_not_the_settings_voice() {
        let settings = AppSettings {
            tts_voice: "bm_lewis".into(),
            ..settings::get_default_settings()
        };
        let tts::Backend::Http { base_url, voice } = http_backend(&settings, "af_heart") else {
            panic!("expected the HTTP backend");
        };
        assert_eq!(voice, "af_heart");
        assert_eq!(base_url, settings.tts_base_url);
    }

    #[test]
    fn a_preview_error_names_the_missing_voice() {
        assert!(preview_error(OverlayView::ModelMissing, "af_alloy").contains("'af_alloy'"));
        let message = "Failed to get app data directory".to_string();
        assert_eq!(
            preview_error(
                OverlayView::Error {
                    message: message.clone()
                },
                "af_alloy"
            ),
            message
        );
    }

    #[test]
    fn speech_state_payload_names_the_phase_and_whether_it_is_a_preview() {
        let payload = |state, preview, error| {
            serde_json::to_value(SpeechStateChange {
                state,
                preview,
                error,
            })
            .unwrap()
        };
        assert_eq!(
            payload(SessionPhase::Started, true, None),
            serde_json::json!({ "state": "started", "preview": true })
        );
        assert_eq!(
            payload(SessionPhase::Speaking, true, None),
            serde_json::json!({ "state": "speaking", "preview": true })
        );
        assert_eq!(
            payload(SessionPhase::Ended, false, None),
            serde_json::json!({ "state": "ended", "preview": false })
        );
        assert_eq!(
            payload(SessionPhase::Ended, true, Some("server error".into())),
            serde_json::json!({ "state": "ended", "preview": true, "error": "server error" })
        );
    }

    #[test]
    fn only_the_current_session_reports_its_state() {
        let stop = Arc::new(AtomicBool::new(false));
        let current = Some(Session::preview(stop.clone()));
        assert_eq!(owner_input(&current, &stop), Some(Input::Preview));
        assert_eq!(
            owner_input(&current, &Arc::new(AtomicBool::new(false))),
            None,
            "a replaced session must not report its first audio"
        );
        assert_eq!(owner_input(&None, &stop), None);
    }

    #[test]
    fn first_audio_is_reported_once_when_the_first_chunk_is_queued() {
        let (player, _output) = Player::new();
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        tx.send(Ok(vec![0; 480])).unwrap();
        tx.send(Ok(vec![0; 480])).unwrap();
        let stop = AtomicBool::new(false);
        let reports = std::sync::atomic::AtomicUsize::new(0);

        let (player_ref, stop_ref, reports_ref) = (&player, &stop, &reports);
        thread::scope(|scope| {
            let _release = StopOnDrop(stop_ref);
            let playback = scope.spawn(move || {
                play_chunks(player_ref, &rx, stop_ref, Instant::now(), || {
                    reports_ref.fetch_add(1, Ordering::SeqCst);
                })
            });
            thread::sleep(PLAYBACK_POLL * 10);
            assert_eq!(player.len(), 2);
            assert_eq!(reports.load(Ordering::SeqCst), 1);
            stop.store(true, Ordering::SeqCst);
            assert_eq!(playback.join().unwrap(), PlaybackEnd::Done);
        });

        let (player, _output) = Player::new();
        let (tx, rx) = mpsc::sync_channel::<Chunk>(READ_AHEAD);
        drop(tx);
        let mut reported = false;
        play_chunks(
            &player,
            &rx,
            &AtomicBool::new(false),
            Instant::now(),
            || reported = true,
        );
        assert!(!reported, "nothing was queued, so nothing is reported");
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
                scope.spawn(move || play_chunks(player_ref, &rx, stop_ref, Instant::now(), || {}));
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
                scope.spawn(move || play_chunks(player_ref, &rx, stop_ref, Instant::now(), || {}));
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
                scope.spawn(move || play_chunks(player_ref, &rx, stop_ref, Instant::now(), || {}));
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
                scope.spawn(move || play_chunks(player_ref, &rx, stop_ref, Instant::now(), || {}));
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
            play_chunks(&player, &rx, &stop, Instant::now(), || {}),
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
                scope.spawn(move || play_chunks(player_ref, &rx, stop_ref, Instant::now(), || {}));
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
            play_chunks(&player, &rx, &stop, Instant::now(), || {}),
            PlaybackEnd::NothingSpoken
        );
        // A stopped session ends quietly even when nothing played.
        stop.store(true, Ordering::SeqCst);
        assert_eq!(
            play_chunks(&player, &rx, &stop, Instant::now(), || {}),
            PlaybackEnd::Done
        );
    }
}
