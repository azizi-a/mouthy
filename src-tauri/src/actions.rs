use crate::audio_out;
use crate::clipboard::{self, ClipboardText};
use crate::overlay::{self, OverlayView};
use crate::settings::{self, AppSettings, ReadingLanguage, TtsBackend};
use crate::tray::{self, TrayIconState};
use crate::tts;
use crate::tts::catalog::VoiceLanguage;
use log::{debug, error, info, warn};
use once_cell::sync::Lazy;
use rodio::buffer::SamplesBuffer;
use rodio::Player;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::num::NonZero;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
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
/// Most characters read aloud from the clipboard, a file or a Linux selection.
/// Normalising and splitting them run on the main thread, which a huge one
/// would stall.
pub(crate) const TEXT_CHAR_CAP: usize = 20_000;
/// Most bytes read from a file: a UTF-8 character is at most four bytes, so
/// these always hold `TEXT_CHAR_CAP` whole characters when the file has them.
const FILE_BYTE_CAP: u64 = TEXT_CHAR_CAP as u64 * 4;
/// On Linux the selection and clipboard come from wl-paste, which waits for
/// the app that owns them, and that can be Mouthy's own main thread. So they
/// are read on the request thread (`send_in_order`) instead.
const READ_OFF_MAIN_THREAD: bool = cfg!(target_os = "linux");
/// On Linux, the least time between two requests that go ahead, when they
/// are not hotkey presses: longer than a keyboard waits before repeating a
/// held key (500–660 ms), so a desktop that repeats a held shortcut never
/// stops what its first press started.
const PRESS_GAP: Duration = Duration::from_millis(700);
/// And the least quiet before one goes ahead: repeats come every 30 ms or so,
/// so a burst never gets through part way.
const REPEAT_GAP: Duration = Duration::from_millis(100);

/// What a voice preview reads in `language`, whatever the app's language.
fn preview_text(language: VoiceLanguage) -> &'static str {
    match language {
        VoiceLanguage::AmericanEnglish | VoiceLanguage::BritishEnglish => {
            "Hi. This is how I sound when I read your text aloud."
        }
        VoiceLanguage::Spanish => "Hola. Así sueno cuando leo tu texto en voz alta.",
        VoiceLanguage::French => {
            "Bonjour. Voici comment je parle quand je lis votre texte à haute voix."
        }
        VoiceLanguage::Italian => "Ciao. Ecco come suono quando leggo il tuo testo ad alta voce.",
        VoiceLanguage::BrazilianPortuguese => {
            "Oi. É assim que a minha voz soa quando leio o seu texto em voz alta."
        }
        VoiceLanguage::Hindi => "नमस्ते। आपका टेक्स्ट ज़ोर से पढ़ते समय मेरी आवाज़ ऐसी सुनाई देती है।",
        // Kana only, and no は or へ particle: kanji and those particles need a
        // dictionary to read right.
        VoiceLanguage::Japanese => {
            "どうも。これが、あなたのテキストをよみあげるときの、わたしのこえです。"
        }
        VoiceLanguage::Mandarin => "你好。这就是我朗读你的文字时的声音。",
    }
}

/// The preview sentence in the language `backend` reads. The server gets
/// English, as it always has.
fn preview_sentence(backend: &tts::Backend) -> &'static str {
    preview_text(match backend {
        tts::Backend::Local(files) => files.language(),
        tts::Backend::Http { .. } => VoiceLanguage::AmericanEnglish,
    })
}

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
    /// Reads in another voice or language than the settings give, so theirs is
    /// loaded again when it ends and the next hotkey does not wait for it.
    other_reading: bool,
}

impl Session {
    fn new(stop: Arc<AtomicBool>, copied_while_held_at: Option<Instant>, input: Input) -> Self {
        Self {
            stop,
            released_since_start: false,
            copied_while_held_at,
            input,
            other_reading: false,
        }
    }

    /// A voice preview or a file. No hotkey started it, so the first press
    /// stops it.
    fn without_hotkey(stop: Arc<AtomicBool>, input: Input, other_reading: bool) -> Self {
        Self {
            stop,
            released_since_start: true,
            copied_while_held_at: None,
            input,
            other_reading,
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
    /// A text file named on the command line.
    File,
    /// The preview sentence, in the voice being previewed.
    Preview,
}

impl Input {
    fn name(self) -> &'static str {
        match self {
            Input::Selection => "selection",
            Input::Clipboard => "clipboard",
            Input::File => "file",
            Input::Preview => "voice preview",
        }
    }

    /// The notice when there was nothing to speak.
    fn empty_notice(self) -> OverlayView {
        match self {
            Input::Selection => OverlayView::NothingSelected,
            Input::Clipboard => OverlayView::ClipboardEmpty,
            Input::File => OverlayView::Error {
                message: "The file had nothing to speak".into(),
            },
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

fn cap_text(text: String, input: Input) -> String {
    let full_len = text.len();
    let text = truncate_chars(text, TEXT_CHAR_CAP);
    if text.len() < full_len {
        info!(
            "Reading the first {TEXT_CHAR_CAP} characters of the {}",
            input.name()
        );
    }
    text
}

/// The text of the file at `path`, cut to `TEXT_CHAR_CAP` characters, with
/// invalid UTF-8 replaced.
fn read_text_file(path: &Path) -> Result<String, String> {
    // Still relative only when the caller's working directory was unknown, and
    // Mouthy's own is the wrong one to read from.
    if !path.is_absolute() {
        return Err(
            "the caller's working directory could not be determined; an absolute path is needed"
                .into(),
        );
    }
    // Opening a pipe waits for a writer, holding up every later CLI request.
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("not a regular file".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(FILE_BYTE_CAP).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(cap_text(text, Input::File))
}

/// Whether a press while speaking stops the speech. A hotkey press does only
/// once the key has been released, so holding it never stops what it started.
fn should_toggle_stop(released_since_start: bool, trigger: Trigger) -> bool {
    trigger == Trigger::Other || released_since_start
}

/// Holds back requests that come in a burst, as a held key's repeats do: one
/// goes ahead `PRESS_GAP` after the last that did, once `REPEAT_GAP` has
/// passed with none at all.
#[derive(Debug, Default)]
struct RepeatGuard {
    last_allowed: Option<Instant>,
    last_seen: Option<Instant>,
}

impl RepeatGuard {
    fn allow(&mut self, now: Instant) -> bool {
        let since = |then: Option<Instant>| then.map(|then| now.saturating_duration_since(then));
        let allowed = since(self.last_allowed).is_none_or(|gap| gap >= PRESS_GAP)
            && since(self.last_seen).is_none_or(|gap| gap >= REPEAT_GAP);
        self.last_seen = Some(now);
        if allowed {
            self.last_allowed = Some(now);
        }
        allowed
    }
}

static REPEATS: Mutex<RepeatGuard> = Mutex::new(RepeatGuard {
    last_allowed: None,
    last_seen: None,
});

fn repeats() -> MutexGuard<'static, RepeatGuard> {
    REPEATS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Stops so far (Linux). Stops do not wait for reads, so a request that was
/// made before a stop, and is still being read when it runs, is dropped.
static STOPS: AtomicU64 = AtomicU64::new(0);

fn stops_so_far() -> u64 {
    STOPS.load(Ordering::SeqCst)
}

/// Whether a stop came after a request made when `stops` had been counted.
fn stopped_since(stops: u64) -> bool {
    stops_so_far() != stops
}

/// Work for the request thread, ending with a post to the main thread.
type Job = Box<dyn FnOnce() + Send>;

/// Runs `job` on the request thread once the jobs sent before it are done,
/// so what they post reaches the main thread in the order they were sent.
fn send_in_order(job: Job) {
    static REQUESTS: OnceLock<Option<Sender<Job>>> = OnceLock::new();
    let requests = REQUESTS.get_or_init(|| {
        let (requests, jobs) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("speech-requests".into())
            .spawn(move || {
                for job in jobs {
                    // One failed request must not stop the ones after it.
                    if panic::catch_unwind(AssertUnwindSafe(job)).is_err() {
                        error!("A speech request panicked");
                    }
                }
            })
            .inspect_err(|e| error!("Failed to start the speech request thread: {e}"))
            .ok()
            .map(|_| requests)
    });
    if !requests.as_ref().is_some_and(|r| r.send(job).is_ok()) {
        error!("Failed to queue a speech request");
    }
}

fn post(app: &AppHandle, what: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = app.run_on_main_thread(f) {
        error!("Failed to schedule {what}: {e}");
    }
}

/// Posts `f` to the main thread, after any request still being read.
fn post_in_order(app: &AppHandle, what: &'static str, f: impl FnOnce() + Send + 'static) {
    if READ_OFF_MAIN_THREAD {
        let app = app.clone();
        send_in_order(Box::new(move || post(&app, what, f)));
    } else {
        post(app, what, f);
    }
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
/// `speak_clipboard`, `speak_file`, `preview_voice`, `stop_speaking`): on the
/// main thread those run inline and would lock it again.
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

/// Speaks the text file at absolute `path` in `voice` and `language`, or the
/// settings ones, cutting off any speech. A file that cannot be spoken still
/// stops the speech, then says why.
pub fn speak_file(
    app: &AppHandle,
    path: &Path,
    voice: Option<String>,
    language: Option<ReadingLanguage>,
    source: &str,
) {
    let speak = file_request(app, path, voice, language, source);
    post_in_order(app, "speech", speak);
}

/// `speak_file` for the file Mouthy was launched with, on the main thread.
/// Nothing can be ahead of it, and starting it now lets a stop handed over
/// straight after find it speaking.
pub fn speak_launch_file(
    app: &AppHandle,
    path: &Path,
    voice: Option<String>,
    language: Option<ReadingLanguage>,
) {
    let speak = file_request(app, path, voice, language, "CLI");
    post(app, "speech", speak);
}

/// Reads the file, then gives what speaks it on the main thread.
fn file_request(
    app: &AppHandle,
    path: &Path,
    voice: Option<String>,
    language: Option<ReadingLanguage>,
    source: &str,
) -> impl FnOnce() + Send + 'static {
    let stops = stops_so_far();
    // Read before posting, so a handed-over file is read off the main thread. A
    // first launch reads on it, bounded by the regular-file check and byte cap.
    let text = read_text_file(path).map_err(|e| {
        warn!("Cannot read {}: {e}", path.display());
        let name = path.file_name().unwrap_or(path.as_os_str());
        OverlayView::Error {
            message: format!("Cannot read {}: {e}", name.to_string_lossy()),
        }
    });
    let handle = app.clone();
    let source = source.to_string();
    move || {
        if stopped_since(stops) {
            info!("Not speaking a file a later stop came before ({source})");
            return;
        }
        speak_file_on_main(&handle, &source, text, voice, language)
    }
}

/// What speaking a file's text needs and the voice and language it is read
/// in, or the notice to show instead.
fn file_speech(
    app: &AppHandle,
    text: Result<String, OverlayView>,
    voice: Option<&str>,
    language: Option<ReadingLanguage>,
) -> Result<(AppSettings, Speech, String, ReadingLanguage), OverlayView> {
    let text = text?;
    let settings = settings::get_settings(app);
    let voice = match voice {
        Some(requested) => file_voice(app, &settings, requested),
        None => settings.tts_voice.clone(),
    };
    let language = file_language(&settings, language);
    let backend = speech_backend(app, &settings, &voice, language)?;
    let split_started = Instant::now();
    let text = tts::text::normalize(&text);
    if let Some(message) = backend.refuses(&text) {
        return Err(OverlayView::Error { message });
    }
    let sentences = sentences_for(&text, &backend);
    let captured_at = Instant::now();
    info!(
        "Split the file's {} chars into {} sentences in {} ms",
        text.chars().count(),
        sentences.len(),
        (captured_at - split_started).as_millis()
    );
    if sentences.is_empty() {
        return Err(Input::File.empty_notice());
    }
    let speech = Speech {
        sentences,
        backend,
        captured_at,
    };
    Ok((settings, speech, voice, language))
}

/// The voice `--voice` asked for, written as the voice setting would be, or
/// the settings voice when that one cannot read the file.
fn file_voice(app: &AppHandle, settings: &AppSettings, requested: &str) -> String {
    usable_voice(settings, || crate::commands::models_dir(app), requested).unwrap_or_else(
        |reason| {
            warn!(
                "Cannot read the file in the voice '{requested}' ({reason}); using the settings voice {}",
                settings.tts_voice
            );
            settings.tts_voice.clone()
        },
    )
}

/// `requested` as the voice setting would store it, or why it cannot read: for
/// Local, it must be a voice or blend whose every voice is downloaded.
fn usable_voice(
    settings: &AppSettings,
    models_dir: impl FnOnce() -> Result<PathBuf, String>,
    requested: &str,
) -> Result<String, String> {
    match settings.tts_backend {
        // As the voice setting: only the server knows its voices.
        TtsBackend::Http => Some(requested.trim())
            .filter(|voice| !voice.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "no voice given".to_string()),
        TtsBackend::Local => {
            let voice = tts::voice::VoiceSpec::parse(requested)?.to_string();
            let missing =
                tts::local::ModelFiles::new(&models_dir()?, settings.tts_model_variant, &voice)
                    .missing_voices();
            if missing.is_empty() {
                Ok(voice)
            } else {
                Err(format!("not downloaded: {}", missing.join(", ")))
            }
        }
    }
}

/// Reads the preview sentence in `voice`, stopping any speech first. Fails,
/// stopping nothing, when the voice is not valid or the local engine lacks a
/// file it needs.
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
    let backend = speech_backend(app, &settings, &voice, settings.tts_language)
        .map_err(|view| preview_error(view, &voice))?;
    info!("Previewing the voice {voice}");
    let speech = Speech {
        sentences: sentences_for(&tts::text::normalize(preview_sentence(&backend)), &backend),
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
    if READ_OFF_MAIN_THREAD {
        // Hotkey presses have their own guard against a held key.
        if trigger == Trigger::Other && !repeats().allow(Instant::now()) {
            debug!("Ignoring a request too soon after the last one ({source})");
            return;
        }
        let stops = stops_so_far();
        send_in_order(Box::new(move || {
            let read = read_off_main(&handle, input);
            let app = handle.clone();
            post(&app, "speech", move || {
                if stopped_since(stops) {
                    debug!("Dropping a request a later stop came before ({source})");
                    return;
                }
                speak_selection_on_main(&handle, &source, trigger, input, Some(read))
            });
        }));
        return;
    }
    post(app, "speech", move || {
        speak_selection_on_main(&handle, &source, trigger, input, None)
    });
}

/// Lets the next hotkey press stop the current speech.
fn hotkey_released(app: &AppHandle) {
    // Timed before posting: a capture can hold up the main thread for a while.
    let released_at = Instant::now();
    let handle = app.clone();
    // In order, so it never overtakes the press before it.
    post_in_order(app, "the hotkey release", move || {
        hotkey_released_on_main(&handle, released_at)
    });
}

/// Stops any speech in progress and hides the overlay.
pub fn stop_speaking(app: &AppHandle, source: &str) {
    info!("Stopping speech ({source})");
    if READ_OFF_MAIN_THREAD {
        // Ahead of any request still being read, which then finds it stopped;
        // and the next request need not wait out the repeat guard.
        STOPS.fetch_add(1, Ordering::SeqCst);
        *repeats() = RepeatGuard::default();
    }
    let handle = app.clone();
    post(app, "stopping speech", move || {
        stop_speaking_on_main(&handle)
    });
}

/// What a press read: its text and when its ⌘C went out with the hotkey
/// held; or the notice to show instead.
type Capture = Result<(Option<String>, Option<Instant>), OverlayView>;

/// Captures on the main thread, which ⌘C needs (macOS).
fn capture_on_main(app: &AppHandle, input: Input, backend: &tts::Backend) -> Capture {
    match input {
        Input::Selection => clipboard::capture_selection(app)
            .map(|selection| (selection.text, selection.copied_while_held_at))
            .map_err(read_failed),
        // No ⌘C goes out, so there is no copy echo to ignore.
        Input::Clipboard => read_clipboard(),
        // `speak_file` reads a file before posting and never comes here.
        Input::File => Ok((None, None)),
        Input::Preview => Ok((Some(preview_sentence(backend).to_string()), None)),
    }
}

/// Reads with wl-paste, which must not run on the main thread (Linux).
fn read_off_main(app: &AppHandle, input: Input) -> Capture {
    let started = Instant::now();
    let read = match input {
        // A selection can be a whole page, so it is capped like the clipboard.
        Input::Selection => clipboard::capture_selection(app)
            .map(|selection| (selection.text.map(|text| cap_text(text, input)), None))
            .map_err(read_failed),
        Input::Clipboard => read_clipboard(),
        // Only selection and clipboard requests are read here.
        Input::File | Input::Preview => Ok((None, None)),
    };
    debug!(
        "Read the {} in {} ms",
        input.name(),
        started.elapsed().as_millis()
    );
    read
}

fn read_clipboard() -> Capture {
    match clipboard::read_text() {
        Ok(ClipboardText::Text(text)) => Ok((Some(cap_text(text, Input::Clipboard)), None)),
        Ok(ClipboardText::Empty) => Ok((None, None)),
        Ok(ClipboardText::Secret) => Err(OverlayView::ClipboardSecret),
        Err(e) => Err(read_failed(e)),
    }
}

/// Logged where it is shown, so a read dropped for a stop is no error.
fn read_failed(e: String) -> OverlayView {
    OverlayView::Error { message: e }
}

/// Speaks what `input` holds, or stops the speech. `read` is the text when it
/// was read before posting; otherwise it is captured here.
fn speak_selection_on_main(
    app: &AppHandle,
    source: &str,
    trigger: Trigger,
    input: Input,
    read: Option<Capture>,
) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    let released = state
        .lock()
        .as_ref()
        .map(|session| session.released_since_start);
    if let Some(released_since_start) = released {
        if let Some(Err(OverlayView::Error { message })) = &read {
            debug!("Dropping a failed read of the {}: {message}", input.name());
        }
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
    let backend = match speech_backend(app, &settings, &settings.tts_voice, settings.tts_language) {
        Ok(backend) => backend,
        Err(view) => {
            overlay::show_overlay(app, view);
            return;
        }
    };
    let capture_started = Instant::now();
    let captured = read.unwrap_or_else(|| capture_on_main(app, input, &backend));
    let (text, copied_while_held_at) = match captured {
        Ok(captured) => captured,
        Err(view) => {
            if let OverlayView::Error { message } = &view {
                error!("Failed to read the {}: {message}", input.name());
            }
            overlay::show_overlay(app, view);
            return;
        }
    };
    let text = text
        .map(|text| tts::text::normalize(&text))
        .unwrap_or_default();
    if let Some(message) = backend.refuses(&text) {
        info!("Not speaking the {}: {message}", input.name());
        overlay::show_overlay(app, OverlayView::Error { message });
        return;
    }
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
    let replaced = stop_session_on_main(app, &state);
    if replaced.is_some() {
        info!("Stopping speech for a voice preview");
    }
    let session = Session::without_hotkey(Arc::new(AtomicBool::new(false)), Input::Preview, true);
    // No preload for a replaced session unless this one fails to load its own voice.
    let started = start_session_on_main(app, &state, settings, speech, session);
    if started.is_err() {
        preload_after(app, replaced.as_ref());
    }
    started
}

/// Loads the settings voice again when `replaced` read in another voice or
/// language and no session took over to load its own.
fn preload_after(app: &AppHandle, replaced: Option<&Session>) {
    if replaced.is_some_and(|session| session.other_reading) {
        preload_local_voice(app);
    }
}

fn speak_file_on_main(
    app: &AppHandle,
    source: &str,
    text: Result<String, OverlayView>,
    voice: Option<String>,
    language: Option<ReadingLanguage>,
) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    let replaced = stop_session_on_main(app, &state);
    if replaced.is_some() {
        info!("Stopping speech for a file ({source})");
    }
    match file_speech(app, text, voice.as_deref(), language) {
        // No preload for a replaced session unless this one fails to load its own voice.
        Ok((settings, speech, voice, language)) => {
            let read_as = match (&speech.backend, language) {
                (tts::Backend::Local(_), ReadingLanguage::English) => " as English",
                _ => "",
            };
            info!("Speaking the file in {voice}{read_as} ({source})");
            let other_reading = reads_unlike_settings(&settings, &voice, language);
            let session = Session::without_hotkey(
                Arc::new(AtomicBool::new(false)),
                Input::File,
                other_reading,
            );
            // A failure to start is on the overlay already.
            if start_session_on_main(app, &state, settings, speech, session).is_err() {
                preload_after(app, replaced.as_ref());
            }
        }
        Err(view) => {
            preload_after(app, replaced.as_ref());
            tray::set_tray_state(app, TrayIconState::Idle);
            overlay::show_overlay(app, view);
        }
    }
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
    if stop_session_on_main(app, &state).is_some_and(|session| session.other_reading) {
        preload_local_voice(app);
    }
    overlay::hide_overlay(app);
    tray::set_tray_state(app, TrayIconState::Idle);
}

/// Stops and forgets the current session, returning it.
fn stop_session_on_main(app: &AppHandle, state: &TtsState) -> Option<Session> {
    let session = state.lock().take()?;
    session.stop.store(true, Ordering::SeqCst);
    emit_speech_state(app, SessionPhase::Ended, session.input, None);
    Some(session)
}

/// Ends the session that owns `stop`, unless it already ended.
fn end_session_on_main(app: &AppHandle, stop: &Arc<AtomicBool>, end: PlaybackEnd) {
    let Some(state) = app.try_state::<TtsState>() else {
        return;
    };
    let Some(Session {
        input,
        other_reading,
        ..
    }) = state
        .lock()
        .take_if(|session| Arc::ptr_eq(&session.stop, stop))
    else {
        return;
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
    if other_reading {
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
    let max_chars = backend.max_chunk_chars(text);
    match backend {
        tts::Backend::Http { .. } => tts::text::split_sentences(text, max_chars),
        tts::Backend::Local(files) => {
            let marks = files.marks(text);
            let sentences = tts::text::split_sentences_with(text, max_chars, marks);
            tts::text::split_first_clause(sentences, marks)
        }
    }
}

/// The backend a request will use, or the notice to show instead.
fn speech_backend(
    app: &AppHandle,
    settings: &AppSettings,
    voice: &str,
    language: ReadingLanguage,
) -> Result<tts::Backend, OverlayView> {
    match settings.tts_backend {
        // The server reads in whatever language it decides.
        TtsBackend::Http => Ok(http_backend(settings, voice)),
        TtsBackend::Local => {
            let files = local_model_files(app, settings, voice, language)
                .map_err(|message| OverlayView::Error { message })?;
            if let Some(missing) = files.missing() {
                info!("The local voice is missing {missing}");
                return Err(OverlayView::ModelMissing);
            }
            match files.unmet_requirement() {
                Some(message) => Err(OverlayView::Error { message }),
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
    language: ReadingLanguage,
) -> Result<tts::local::ModelFiles, String> {
    let models_dir = crate::commands::models_dir(app)?;
    let files = tts::local::ModelFiles::new(&models_dir, settings.tts_model_variant, voice);
    Ok(read_in(files, language)
        .with_english_fallback(settings.tts_espeak_fallback)
        .with_espeak(tts::espeak::find_if_enabled(settings.tts_espeak_fallback)))
}

/// `files` read in the language `choice` gives.
fn read_in(files: tts::local::ModelFiles, choice: ReadingLanguage) -> tts::local::ModelFiles {
    let language = reading_language(files.language(), choice);
    files.reading_as(language)
}

/// The language a voice that reads `voice_language` reads in under `choice`.
/// English keeps a British voice British and reads American otherwise.
fn reading_language(voice_language: VoiceLanguage, choice: ReadingLanguage) -> VoiceLanguage {
    match (choice, voice_language) {
        (ReadingLanguage::Voice, language) => language,
        (ReadingLanguage::English, VoiceLanguage::BritishEnglish) => VoiceLanguage::BritishEnglish,
        (ReadingLanguage::English, _) => VoiceLanguage::AmericanEnglish,
    }
}

/// The language `--lang` asked for, or the settings one.
fn file_language(settings: &AppSettings, requested: Option<ReadingLanguage>) -> ReadingLanguage {
    requested.unwrap_or(settings.tts_language)
}

/// Whether reading in `voice` and `language` loads something other than the
/// settings do. The server ignores the language.
fn reads_unlike_settings(settings: &AppSettings, voice: &str, language: ReadingLanguage) -> bool {
    voice != settings.tts_voice
        || (settings.tts_backend == TtsBackend::Local && language != settings.tts_language)
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
                let files =
                    local_model_files(&app, &settings, &settings.tts_voice, settings.tts_language)
                        .inspect_err(|e| warn!("Cannot find the local voice: {e}"))
                        .ok()?;
                if let Some(missing) = files.missing() {
                    info!("The local voice is missing {missing}");
                    return None;
                }
                if let Some(requirement) = files.unmet_requirement() {
                    info!("The local voice cannot load: {requirement}");
                    return None;
                }
                Some(files)
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
    fn a_held_key_starts_speech_once_and_never_stops_it() {
        let start = Instant::now();
        let ms = |n: u64| start + Duration::from_millis(n);
        let mut guard = RepeatGuard::default();
        assert!(guard.allow(ms(0)));
        // The keyboard waits 500 ms, then repeats every 33 ms for two seconds.
        let mut last = 0;
        for n in (500..2500).step_by(33) {
            assert!(!guard.allow(ms(n)), "repeat at {n} ms");
            last = n;
        }
        assert!(!guard.allow(ms(last + 99)), "99 ms after the last repeat");
        assert!(guard.allow(ms(last + 99 + 100)), "the next press stops it");
    }

    #[test]
    fn taps_far_enough_apart_all_go_ahead_and_quick_ones_do_not() {
        let start = Instant::now();
        let ms = |n: u64| start + Duration::from_millis(n);
        let mut guard = RepeatGuard::default();
        for n in [0, 800, 1600, 2400] {
            assert!(guard.allow(ms(n)), "tap at {n} ms");
        }
        assert!(!guard.allow(ms(2400 + 400)), "400 ms after the last tap");
        assert!(guard.allow(ms(2400 + 700)), "700 ms after it");
    }

    #[test]
    fn a_request_made_before_a_stop_is_dropped_and_one_after_is_not() {
        let before = stops_so_far();
        // What `stop_speaking` does on Linux before posting.
        STOPS.fetch_add(1, Ordering::SeqCst);
        assert!(stopped_since(before));
        assert!(!stopped_since(stops_so_far()));
    }

    #[test]
    fn after_a_stop_the_next_request_goes_ahead_at_once() {
        let start = Instant::now();
        let mut guard = RepeatGuard::default();
        assert!(guard.allow(start));
        // `stop_speaking` starts the guard afresh.
        guard = RepeatGuard::default();
        assert!(guard.allow(start + Duration::from_millis(250)));
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
        let long = "é".repeat(TEXT_CHAR_CAP + 10);
        assert_eq!(
            cap_text(long, Input::Clipboard).chars().count(),
            TEXT_CHAR_CAP
        );
    }

    #[test]
    fn a_text_file_is_read_whole_up_to_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reply.txt");

        std::fs::write(&path, "Hello there.\nSecond line.").unwrap();
        assert_eq!(read_text_file(&path).unwrap(), "Hello there.\nSecond line.");

        std::fs::write(&path, "a".repeat(FILE_BYTE_CAP as usize + 10)).unwrap();
        assert_eq!(read_text_file(&path).unwrap(), "a".repeat(TEXT_CHAR_CAP));

        // The byte cap cuts the last character in half; the cap must drop it whole.
        let four_byte = "😀".repeat(TEXT_CHAR_CAP + 10);
        std::fs::write(&path, format!("a{four_byte}")).unwrap();
        let text = read_text_file(&path).unwrap();
        assert_eq!(text.chars().count(), TEXT_CHAR_CAP);
        assert!(!text.contains('\u{FFFD}'));

        std::fs::write(&path, b"caf\xe9 au lait").unwrap();
        assert_eq!(read_text_file(&path).unwrap(), "caf\u{FFFD} au lait");

        std::fs::write(&path, "").unwrap();
        assert_eq!(read_text_file(&path).unwrap(), "");
    }

    #[test]
    fn a_text_file_that_cannot_be_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_text_file(&dir.path().join("missing.txt")).is_err());
        assert!(read_text_file(dir.path()).is_err(), "a directory");
        assert!(
            read_text_file(Path::new("reply.txt")).is_err(),
            "a relative path"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_is_refused_without_waiting_for_a_writer() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("reply.fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(read_text_file(&fifo));
        });
        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("reading a pipe blocked");
        assert!(result.is_err());
    }

    #[test]
    fn a_file_with_nothing_to_speak_says_so() {
        let OverlayView::Error { message } = Input::File.empty_notice() else {
            panic!("expected an error notice");
        };
        assert!(message.contains("file"));
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
    fn the_first_hotkey_press_stops_a_voice_preview_or_a_file() {
        for input in [Input::Preview, Input::File] {
            let session = Session::without_hotkey(Arc::new(AtomicBool::new(false)), input, true);
            assert_eq!(session.input, input);
            assert!(should_toggle_stop(
                session.released_since_start,
                Trigger::Hotkey
            ));
        }
    }

    fn local_settings(voice: &str) -> AppSettings {
        AppSettings {
            tts_backend: TtsBackend::Local,
            tts_voice: voice.into(),
            ..settings::get_default_settings()
        }
    }

    fn install_voices(models_dir: &Path, voices: &[&str]) {
        for voice in voices {
            let path = tts::local::voice_path(models_dir, voice).unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"voice").unwrap();
        }
    }

    #[test]
    fn a_downloaded_voice_or_blend_reads_the_file_in_canonical_form() {
        let dir = tempfile::tempdir().unwrap();
        install_voices(dir.path(), &["bm_lewis", "bm_george"]);
        let settings = local_settings("af_alloy");
        let usable =
            |requested| usable_voice(&settings, || Ok(dir.path().to_path_buf()), requested);
        assert_eq!(usable(" bm_lewis ").unwrap(), "bm_lewis");
        assert_eq!(
            usable("bm_lewis ( 70 ) + bm_george(30)").unwrap(),
            "bm_lewis(70)+bm_george(30)"
        );
    }

    #[test]
    fn a_voice_that_cannot_read_locally_says_why() {
        let dir = tempfile::tempdir().unwrap();
        install_voices(dir.path(), &["bm_lewis"]);
        let settings = local_settings("bm_lewis");
        let usable =
            |requested| usable_voice(&settings, || Ok(dir.path().to_path_buf()), requested);
        assert_eq!(usable("bf_emma").unwrap_err(), "not downloaded: bf_emma");
        assert_eq!(
            usable("bm_lewis(70)+bf_emma(30)").unwrap_err(),
            "not downloaded: bf_emma",
            "one missing voice is enough to fall back"
        );
        for requested in ["", "bm_lewis-bm_george", "../secret", "bm_lewis(0)"] {
            assert!(usable(requested).is_err(), "{requested:?}");
        }
        assert_eq!(
            usable_voice(&settings, || Err("no app data".into()), "bm_lewis").unwrap_err(),
            "no app data"
        );
    }

    #[test]
    fn the_server_gets_any_voice_asked_for_trimmed() {
        let settings = AppSettings {
            tts_backend: TtsBackend::Http,
            ..settings::get_default_settings()
        };
        let no_models = || -> Result<PathBuf, String> { panic!("HTTP never looks for files") };
        assert_eq!(
            usable_voice(&settings, no_models, " my_server_voice ").unwrap(),
            "my_server_voice"
        );
        assert!(usable_voice(&settings, no_models, "  ").is_err());
    }

    const ALL_LANGUAGES: [VoiceLanguage; 9] = [
        VoiceLanguage::AmericanEnglish,
        VoiceLanguage::BritishEnglish,
        VoiceLanguage::Spanish,
        VoiceLanguage::French,
        VoiceLanguage::Hindi,
        VoiceLanguage::Italian,
        VoiceLanguage::Japanese,
        VoiceLanguage::BrazilianPortuguese,
        VoiceLanguage::Mandarin,
    ];

    #[test]
    fn a_voice_reads_its_own_language_unless_english_is_chosen() {
        for language in ALL_LANGUAGES {
            assert_eq!(reading_language(language, ReadingLanguage::Voice), language);
            let english = if language == VoiceLanguage::BritishEnglish {
                VoiceLanguage::BritishEnglish
            } else {
                VoiceLanguage::AmericanEnglish
            };
            assert_eq!(
                reading_language(language, ReadingLanguage::English),
                english,
                "{language:?}"
            );
        }
    }

    #[test]
    fn lang_beats_the_reading_language_setting() {
        let mut settings = local_settings("ef_dora");
        assert_eq!(file_language(&settings, None), ReadingLanguage::Voice);
        assert_eq!(
            file_language(&settings, Some(ReadingLanguage::English)),
            ReadingLanguage::English
        );
        settings.tts_language = ReadingLanguage::English;
        assert_eq!(file_language(&settings, None), ReadingLanguage::English);
        assert_eq!(
            file_language(&settings, Some(ReadingLanguage::Voice)),
            ReadingLanguage::Voice
        );
    }

    fn files(voice: &str) -> tts::local::ModelFiles {
        tts::local::ModelFiles::new(Path::new("models"), settings::ModelVariant::Int8, voice)
    }

    #[test]
    fn english_keeps_a_british_voice_british_and_reads_any_other_as_american() {
        let english = |voice| read_in(files(voice), ReadingLanguage::English).language();
        for voice in ["bf_emma", "bm_lewis", "bm_lewis+ef_dora"] {
            assert_eq!(english(voice), VoiceLanguage::BritishEnglish, "{voice}");
        }
        for voice in [
            "af_alloy",
            "ef_dora",
            "ff_siwis",
            "hf_alpha",
            "if_sara",
            "jf_alpha",
            "pf_dora",
            "zf_xiaobei",
            "ef_dora+bf_emma",
            "xx_custom",
        ] {
            assert_eq!(english(voice), VoiceLanguage::AmericanEnglish, "{voice}");
        }
    }

    #[test]
    fn the_voice_choice_leaves_the_voices_language_alone() {
        for voice in ["af_alloy", "bf_emma", "ef_dora", "jf_alpha", "zf_xiaobei"] {
            assert_eq!(
                read_in(files(voice), ReadingLanguage::Voice).language(),
                files(voice).language(),
                "{voice}"
            );
        }
    }

    #[test]
    fn a_non_english_voice_read_as_english_previews_in_english() {
        let backend = tts::Backend::Local(read_in(files("jf_alpha"), ReadingLanguage::English));
        assert_eq!(
            preview_sentence(&backend),
            preview_text(VoiceLanguage::AmericanEnglish)
        );
    }

    #[test]
    fn a_file_in_another_voice_or_language_than_the_settings_reloads_them_after() {
        let settings = local_settings("ef_dora");
        assert!(!reads_unlike_settings(
            &settings,
            "ef_dora",
            ReadingLanguage::Voice
        ));
        assert!(reads_unlike_settings(
            &settings,
            "bf_emma",
            ReadingLanguage::Voice
        ));
        assert!(reads_unlike_settings(
            &settings,
            "ef_dora",
            ReadingLanguage::English
        ));
        let server = AppSettings {
            tts_backend: TtsBackend::Http,
            ..settings
        };
        assert!(!reads_unlike_settings(
            &server,
            "ef_dora",
            ReadingLanguage::English
        ));
        assert!(reads_unlike_settings(
            &server,
            "bf_emma",
            ReadingLanguage::Voice
        ));
    }

    fn local_backend(voice: &str) -> tts::Backend {
        tts::Backend::Local(tts::local::ModelFiles::new(
            std::path::Path::new("models"),
            settings::ModelVariant::Int8,
            voice,
        ))
    }

    #[test]
    fn every_language_has_a_preview_sentence() {
        let languages = [
            VoiceLanguage::AmericanEnglish,
            VoiceLanguage::BritishEnglish,
            VoiceLanguage::Japanese,
            VoiceLanguage::Mandarin,
            VoiceLanguage::Spanish,
            VoiceLanguage::French,
            VoiceLanguage::Hindi,
            VoiceLanguage::Italian,
            VoiceLanguage::BrazilianPortuguese,
        ];
        let sentences: std::collections::HashSet<_> =
            languages.into_iter().map(preview_text).collect();
        assert_eq!(
            sentences.len(),
            languages.len() - 1,
            "both Englishes share one"
        );
        for language in languages {
            let text = tts::text::normalize(preview_text(language));
            assert!(!text.is_empty(), "{language:?}");
        }
        let is_kanji = |c: char| ('\u{4e00}'..='\u{9fff}').contains(&c);
        assert!(!preview_text(VoiceLanguage::Japanese).contains(is_kanji));
    }

    /// A voice of each language this build cannot read yet.
    const READ_AS_ENGLISH: &[&str] = &["zf_xiaobei", "ef_dora", "hf_alpha"];

    #[test]
    fn a_voice_this_build_cannot_read_previews_in_english() {
        let english = preview_text(VoiceLanguage::AmericanEnglish);
        for voice in READ_AS_ENGLISH.iter().chain(&["xx_custom"]) {
            assert_eq!(preview_sentence(&local_backend(voice)), english, "{voice}");
        }
        let server = http_backend(&settings::get_default_settings(), "ef_dora");
        assert_eq!(preview_sentence(&server), english);
        let server = http_backend(&settings::get_default_settings(), "jm_kumo");
        assert_eq!(preview_sentence(&server), english);
    }

    #[test]
    fn a_japanese_voice_previews_in_japanese() {
        let backend = local_backend("jm_kumo");
        assert_eq!(
            preview_sentence(&backend),
            preview_text(VoiceLanguage::Japanese)
        );
        assert_eq!(backend.refuses(preview_sentence(&backend)), None);
        // "どうも。" is too short to read alone and joins the next sentence.
        let sentences = sentences_for(preview_sentence(&backend), &backend);
        assert_eq!(sentences.len(), 1, "{sentences:?}");
    }

    #[test]
    fn the_server_gets_chinese_and_hindi_cut_as_before() {
        let server = http_backend(&settings::get_default_settings(), "zf_xiaobei");
        for text in [
            "今天的天气非常好，我们一起去公园散步吧。明天可能会下大雨，所以要带伞。",
            "आज मौसम बहुत अच्छा है। कल शायद बारिश होगी, तो छाता ज़रूर ले जाना॥ क्या आप आना चाहेंगे?",
        ] {
            assert_eq!(
                sentences_for(text, &server),
                tts::text::split_sentences(text, 300),
                "{text}"
            );
        }
    }

    #[test]
    fn both_engines_speak_the_preview_sentence() {
        let http = http_backend(&settings::get_default_settings(), "af_alloy");
        for backend in [http, local_backend("af_alloy"), local_backend("jf_alpha")] {
            let text = tts::text::normalize(preview_sentence(&backend));
            let sentences = sentences_for(&text, &backend);
            assert!(!sentences.is_empty());
            assert!(sentences
                .iter()
                .all(|s| s.text.chars().count() <= backend.max_chunk_chars(&text)));
        }
    }

    /// What the English pin reads: the listening test as it stood when the pin
    /// was made, so later edits to that guide leave it alone, and then each of
    /// `ENGLISH_LINES` on its own, as a selection would be.
    const ENGLISH_TEXT: &str = include_str!("tts/fixtures/english-text.txt");
    const ENGLISH_PHONEMES: &str = include_str!("tts/fixtures/english-phonemes.txt");
    const ENGLISH_LINES: &[&str] = &[
        "The naïve café served crème brûlée and a jalapeño.",
        "Œuvre, Straße, æsthetics and the ﬁnal ﬂoor plan.",
        "Señor Muñoz flew from Zürich to Kraków and São Paulo.",
        "Cafe\u{301} and re\u{301}sume\u{301} are typed with separate accents.",
        "She said “no” twice, and it’s the user’s fault, isn’t it?",
        "„Quoted“ and ‘single’ quotes don’t split a word.",
        "Dr. Smith and Mrs. Jones met Prof. Brown on Baker St. at 9 a.m.",
        "Cats vs. dogs, approx. forty of each, etc.",
        "See No. 5, Capt. Hook and Smith Jr. at St. Paul's.",
        "Ms. Smith met Dr Who, Gen Z and Gen. Patton.",
        "Tools, e.g. hammers, i.e. the heavy ones, at 7 a.m. or 5 p.m.",
        "The API, URL and HTTP specs are on GitHub.",
        "IDs, URLs and PDFs: the U.S. and the U.K. agree.",
        "macOS, iOS, HTMLParser and camelCase names read as words.",
        "Node.js, README.md and ort.pyke.io are names, not sentences.",
        "It costs $3.2 billion, £45 or €10.50 each.",
        "Version 2.0.1 fixed 37 bugs on 1 April 2027.",
        "The 1990s, the '80s and the 21st century.",
        "At 10:30 or 7.30pm, about 45% of us left.",
        "Read pages 10-12, then 1914–1918, then 9:00–17:00.",
        "Prices fell by \u{2212}5 percent, from 1,500 to 1,425.",
        "He came 3rd, then 2nd, then 1st.",
        "Call 020-7946-0958 before 2026-10-13.",
        "Tea — or coffee – is fine, well—maybe.",
        "Use [the API docs][1] and (see [the FAQ]).",
        "Soft\u{ad}ware, zero\u{200b}width and no\u{a0}break spaces vanish.",
        "Wait... what? Really!",
        "日本語 and emoji 🎉 pass through untouched.",
        "When the meeting finally ended, after a long day of talks about the budget, everyone went home.",
        "The report, which the committee had asked for in the spring, ran to more than two hundred pages, \
         covered every branch of the library from the smallest reading room to the main hall, listed the \
         books that had been lost, damaged or never returned, set out what each repair would cost and how \
         long it would take, and ended with a long list of recommendations that the members argued about \
         for most of the summer, until the chair finally called a vote in September and the plan was \
         approved by a narrow margin.",
    ];

    /// One `ends_sentence<TAB>phonemes` line per chunk `voice` reads the
    /// English pin's text as.
    fn english_reading(voice: &str) -> Vec<String> {
        english_chunks(voice)
            .into_iter()
            .map(|(_, line)| line)
            .collect()
    }

    /// Each chunk of the English pin's text, with its `english_reading` line.
    fn english_chunks(voice: &str) -> Vec<(String, String)> {
        let backend = local_backend(voice);
        let chunks: Vec<tts::text::Chunk> = std::iter::once(ENGLISH_TEXT)
            .chain(ENGLISH_LINES.iter().copied())
            .flat_map(|text| sentences_for(&tts::text::normalize(text), &backend))
            .collect();
        let tts::Backend::Local(files) = &backend else {
            unreachable!()
        };
        let texts: Vec<String> = chunks.iter().map(|chunk| chunk.text.clone()).collect();
        let phonemes = tts::local::phonemes_without_espeak(files, &texts);
        chunks
            .into_iter()
            .zip(phonemes)
            .map(|(chunk, phonemes)| {
                let line = format!("{}\t{phonemes}", chunk.ends_sentence);
                (chunk.text, line)
            })
            .collect()
    }

    /// One `voice<TAB>ends_sentence<TAB>phonemes` line per chunk.
    fn english_phonemes() -> String {
        ["bm_lewis", "af_alloy"]
            .into_iter()
            .flat_map(|voice| {
                english_reading(voice)
                    .into_iter()
                    .map(move |line| format!("{voice}\t{line}\n"))
            })
            .collect()
    }

    #[test]
    fn a_voice_in_a_language_this_build_cannot_read_reads_as_american_english() {
        let american = english_reading("af_alloy");
        for &voice in READ_AS_ENGLISH {
            let read = english_reading(voice);
            assert_eq!(read.len(), american.len(), "{voice}");
            for (read, american) in read.iter().zip(&american) {
                assert_eq!(read, american, "{voice}");
            }
        }
    }

    #[test]
    fn english_text_in_a_japanese_voice_reads_as_an_american_voice_does() {
        let american = english_chunks("af_alloy");
        let has_japanese = |text: &str| {
            text.chars()
                .any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}'))
        };
        let kanji_line = "日本語 and emoji 🎉 pass through untouched.";
        for voice in ["jm_kumo", "jf_alpha"] {
            let read = english_chunks(voice);
            assert_eq!(read.len(), american.len(), "{voice}");
            let mut differing = Vec::new();
            for ((text, line), (american_text, american_line)) in read.iter().zip(&american) {
                assert_eq!(text, american_text, "{voice}: chunks differ");
                if has_japanese(text) {
                    differing.push(text.as_str());
                } else {
                    assert_eq!(line, american_line, "{voice}: {text}");
                }
            }
            assert_eq!(differing, [kanji_line], "{voice}");
        }
        // The line with kanji is read, not refused, and its English as an
        // American voice reads the same words.
        let tts::Backend::Local(kumo) = local_backend("jm_kumo") else {
            unreachable!()
        };
        assert_eq!(kumo.refuses(kanji_line), None);
        let tts::Backend::Local(alloy) = local_backend("af_alloy") else {
            unreachable!()
        };
        let english = "and emoji 🎉 pass through untouched.";
        assert_eq!(
            tts::local::phonemes_without_espeak(&kumo, &[kanji_line.to_string()]),
            tts::local::phonemes_without_espeak(&alloy, &[english.to_string()])
        );
    }

    #[test]
    fn a_japanese_voice_set_to_read_english_takes_the_whole_english_path() {
        let files = |voice| {
            tts::local::ModelFiles::new(
                std::path::Path::new("models"),
                settings::ModelVariant::Int8,
                voice,
            )
        };
        let kumo = read_in(files("jm_kumo"), ReadingLanguage::English);
        assert_eq!(kumo.language(), VoiceLanguage::AmericanEnglish);
        let mixed = "東京に行きます。きょうはOKです。";
        assert_eq!(kumo.refuses(mixed), None);
        let backend = tts::Backend::Local(kumo);
        assert_eq!(backend.max_chunk_chars(mixed), 450);
        let text = format!("{mixed} The build passed, and 3 tests ran at 10:30.");
        let tts::Backend::Local(kumo) = &backend else {
            unreachable!()
        };
        let tts::Backend::Local(alloy) = local_backend("af_alloy") else {
            unreachable!()
        };
        assert_eq!(
            sentences_for(&text, &backend),
            sentences_for(&text, &tts::Backend::Local(files("af_alloy")))
        );
        assert_eq!(
            tts::local::phonemes_without_espeak(kumo, std::slice::from_ref(&text)),
            tts::local::phonemes_without_espeak(&alloy, std::slice::from_ref(&text))
        );
    }

    /// English as the local voice reads it must not move unless on purpose:
    /// then run `cargo test -- --ignored write_the_english_phonemes` and
    /// review the fixture's diff.
    #[test]
    fn english_reads_as_the_fixture_says() {
        let read = english_phonemes();
        for (line, (read, expected)) in read.lines().zip(ENGLISH_PHONEMES.lines()).enumerate() {
            assert_eq!(read, expected, "fixture line {}", line + 1);
        }
        assert_eq!(read, ENGLISH_PHONEMES);
    }

    #[test]
    #[ignore = "rewrites the English fixture"]
    fn write_the_english_phonemes() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tts/fixtures/english-phonemes.txt"
        );
        std::fs::write(path, english_phonemes()).unwrap();
    }

    /// 16-bit mono PCM at Kokoro's rate.
    fn write_wav(path: &Path, samples: &[i16]) {
        let data = (samples.len() * 2) as u32;
        let rate = tts::SAMPLE_RATE;
        let mut bytes = Vec::with_capacity(44 + data as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, bytes).unwrap();
    }

    /// Writes Japanese for a listener, as a session would read it:
    /// `MOUTHY_KOKORO_MODELS=<models dir> cargo test -- --ignored japanese_voice --nocapture`.
    #[test]
    #[ignore = "needs Kokoro model files and jm_kumo"]
    fn japanese_voice_reads_kana_aloud() {
        let models = PathBuf::from(std::env::var("MOUTHY_KOKORO_MODELS").unwrap());
        let files =
            || tts::local::ModelFiles::new(&models, settings::ModelVariant::Int8, "jm_kumo");
        for (name, text) in [
            ("preview", preview_text(VoiceLanguage::Japanese)),
            (
                "kana",
                "がっこうのまえで、ラーメンをたべました。コーヒーもおいしかったです！あしたは、さんぽにいきませんか？",
            ),
        ] {
            let backend = tts::Backend::Local(files());
            let sentences = sentences_for(&tts::text::normalize(text), &backend);
            let stop = Arc::new(AtomicBool::new(false));
            let tts::Synth::Local(mut loaded) = backend.open(&stop).unwrap() else {
                unreachable!()
            };
            let mut audio = Vec::new();
            for sentence in &sentences {
                let samples = loaded
                    .synthesize(&sentence.text, 1.0, sentence.ends_sentence)
                    .unwrap();
                assert!(samples.len() > tts::SAMPLE_RATE as usize / 2, "{sentence:?}");
                audio.extend(samples);
            }
            drop(loaded);
            let path = std::env::temp_dir().join(format!("mouthy-ja-{name}.wav"));
            write_wav(&path, &audio);
            println!("{}", path.display());
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
        let current = Some(Session::without_hotkey(stop.clone(), Input::Preview, true));
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
