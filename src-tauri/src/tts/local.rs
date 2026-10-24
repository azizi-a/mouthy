//! In-process Kokoro-82M: misaki-rs phonemes, ONNX Runtime inference.
//!
//! Files live under `<models>/kokoro-v1.0/`: `<variant>.onnx` (onnx-community
//! export) and `voices/<voice>.bin`. The loaded model stays in memory between
//! sessions; one session at a time holds it.

use super::espeak;
use super::text::{self, expand_abbreviations, expand_numbers, joins_numbers, spell_out};
use super::SAMPLE_RATE;
use crate::settings::ModelVariant;
use log::{debug, error, info, trace, warn};
use misaki_rs::lexicon::{Lexicon, PhonemeEntry};
use misaki_rs::{Language, G2P};
use ort::environment::Environment;
use ort::logging::LogLevel;
use ort::session::builder::AutoDevicePolicy;
use ort::session::{RunOptions, Session};
use ort::value::{Tensor, TensorRef};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// The folder under the models directory that one Kokoro release lives in.
const KOKORO_DIR: &str = "kokoro-v1.0";
/// The most ids one inference gets: Kokoro's limit. A chunk that fits is read
/// in one go, so its intonation carries across the sentence. ONNX Runtime keeps
/// the memory its longest run needed (~1 GB at this limit) while loaded.
const PIECE_TOKENS: usize = 510;
/// Longer sentences are cut at a clause break before this. English prose runs
/// about one id per character, so a chunk, even with a short tail merged back,
/// fits one inference; numbers and spelled-out words run longer and
/// `split_tokens` cuts those.
pub(super) const MAX_CHUNK_CHARS: usize = 450;
const STYLE_DIM: usize = 256;
/// How often a running inference checks whether its session was stopped.
const STOP_POLL: Duration = Duration::from_millis(20);

/// Kokoro pads every inference with ~0.4 s of silence before the speech and
/// 0.5–0.8 s after it, whatever the punctuation, so untrimmed pieces meet
/// with a gap of about a second. At or below this (about -50 dBFS) is silence.
const SILENCE: f32 = 0.003;
/// Silence kept before the speech, enough for the softest onset.
const LEAD_MS: f32 = 50.0;
/// Silence kept after a sentence, or a piece cut at a pause. With `LEAD_MS`
/// it matches Kokoro's own pause inside one inference: ~380 ms at a comma,
/// ~470 ms at a full stop (at 1×; pauses scale with the speed).
const PAUSE_END_MS: f32 = 400.0;
/// After a piece cut mid-phrase: a breath, not a stop.
const PHRASE_END_MS: f32 = 100.0;

/// Kokoro v1.0 phoneme vocabulary (onnx-community `tokenizer.json`; id 0 is the pad).
#[rustfmt::skip]
const VOCAB: &[(char, i64)] = &[
    (';', 1), (':', 2), (',', 3), ('.', 4), ('!', 5), ('?', 6), ('—', 9), ('…', 10), ('"', 11),
    ('(', 12), (')', 13), ('“', 14), ('”', 15), (' ', 16), ('\u{303}', 17), ('ʣ', 18),
    ('ʥ', 19), ('ʦ', 20), ('ʨ', 21), ('ᵝ', 22), ('\u{ab67}', 23), ('A', 24), ('I', 25),
    ('O', 31), ('Q', 33), ('S', 35), ('T', 36), ('W', 39), ('Y', 41), ('ᵊ', 42), ('a', 43),
    ('b', 44), ('c', 45), ('d', 46), ('e', 47), ('f', 48), ('h', 50), ('i', 51), ('j', 52),
    ('k', 53), ('l', 54), ('m', 55), ('n', 56), ('o', 57), ('p', 58), ('q', 59), ('r', 60),
    ('s', 61), ('t', 62), ('u', 63), ('v', 64), ('w', 65), ('x', 66), ('y', 67), ('z', 68),
    ('ɑ', 69), ('ɐ', 70), ('ɒ', 71), ('æ', 72), ('β', 75), ('ɔ', 76), ('ɕ', 77), ('ç', 78),
    ('ɖ', 80), ('ð', 81), ('ʤ', 82), ('ə', 83), ('ɚ', 85), ('ɛ', 86), ('ɜ', 87), ('ɟ', 90),
    ('ɡ', 92), ('ɥ', 99), ('ɨ', 101), ('ɪ', 102), ('ʝ', 103), ('ɯ', 110), ('ɰ', 111),
    ('ŋ', 112), ('ɳ', 113), ('ɲ', 114), ('ɴ', 115), ('ø', 116), ('ɸ', 118), ('θ', 119),
    ('œ', 120), ('ɹ', 123), ('ɾ', 125), ('ɻ', 126), ('ʁ', 128), ('ɽ', 129), ('ʂ', 130),
    ('ʃ', 131), ('ʈ', 132), ('ʧ', 133), ('ʊ', 135), ('ʋ', 136), ('ʌ', 138), ('ɣ', 139),
    ('ɤ', 140), ('χ', 142), ('ʎ', 143), ('ʒ', 147), ('ʔ', 148), ('ˈ', 156), ('ˌ', 157),
    ('ː', 158), ('ʰ', 162), ('ʲ', 164), ('↓', 169), ('→', 171), ('↗', 172), ('↘', 173),
    ('ᵻ', 177),
];
const SPACE_ID: i64 = 16;
/// `;:,.!?—…`: where a long input can be cut without breaking a phrase.
const PAUSE_IDS: &[i64] = &[1, 2, 3, 4, 5, 6, 9, 10];

/// misaki-rs 0.6 lexicons tie diphthongs and affricates with U+200D, eSpeak
/// style; Kokoro was trained on misaki's single-symbol forms.
pub(super) const TIES_COMMON: &[(&str, &str)] = &[
    ("e\u{200d}ɪ", "A"),
    ("a\u{200d}ɪ", "I"),
    ("a\u{200d}ʊ", "W"),
    ("ɔ\u{200d}ɪ", "Y"),
    ("d\u{200d}ʒ", "ʤ"),
    ("t\u{200d}ʃ", "ʧ"),
    ("ə\u{200d}l", "ᵊl"),
    ("ɬ", "l"),
];
const TIES_GB: &[(&str, &str)] = &[
    ("ə\u{200d}ʊ", "Q"),
    ("i\u{200d}ə", "ɪə"),
    ("e\u{200d}ə", "ɛː"),
];
/// Runs in order: `ː‍ɹ` must go before every other `ː` is dropped.
const TIES_US: &[(&str, &str)] = &[
    ("o\u{200d}ʊ", "O"),
    ("ː\u{200d}ɹ", "ɹ"),
    ("ː", ""),
    ("n\u{329}", "ᵊn"),
];

/// Only one ONNX Runtime environment may exist per process, and it must
/// outlive every session, so it is built once and never dropped.
static ENVIRONMENT: OnceLock<Result<Environment, String>> = OnceLock::new();
/// The loaded model. Speech holds it for a whole session; only the synth and
/// preload threads lock it, never the main thread.
static LOADED: Mutex<Option<LocalSynth>> = Mutex::new(None);
/// Why the last preload failed, until the next one. Apart from `LOADED`, so
/// the settings page can read it while a session holds the model.
static PRELOAD_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Which misaki lexicon a voice needs: `b*` voices are British English,
/// every other voice is read as American English.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Dialect {
    British,
    American,
}

impl Dialect {
    fn for_voice(voice: &str) -> Self {
        if voice.starts_with(['b', 'B']) {
            Dialect::British
        } else {
            Dialect::American
        }
    }

    pub(super) fn language(self) -> Language {
        match self {
            Dialect::British => Language::EnglishGB,
            Dialect::American => Language::EnglishUS,
        }
    }

    fn ties(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Dialect::British => TIES_GB,
            Dialect::American => TIES_US,
        }
    }
}

/// The files one local voice needs.
#[derive(Debug)]
pub struct ModelFiles {
    model: PathBuf,
    voice_id: String,
    /// `None` when the voice name could not be a file name.
    voice: Option<PathBuf>,
    /// The espeak-ng that reads words the lexicon lacks, if any.
    espeak: Option<PathBuf>,
}

impl ModelFiles {
    pub fn new(models_dir: &Path, variant: ModelVariant, voice: &str) -> Self {
        Self {
            model: model_path(models_dir, variant),
            voice_id: voice.to_string(),
            voice: voice_path(models_dir, voice),
            espeak: None,
        }
    }

    pub fn with_espeak(self, espeak: Option<PathBuf>) -> Self {
        Self { espeak, ..self }
    }

    /// The first file that is not there, described for the log.
    pub fn missing(&self) -> Option<String> {
        if self.model_missing() {
            Some(self.model.display().to_string())
        } else if self.voice_missing() {
            Some(match &self.voice {
                Some(path) => path.display().to_string(),
                None => format!("a voice file for '{}'", self.voice_id),
            })
        } else {
            None
        }
    }

    pub fn model_missing(&self) -> bool {
        !self.model.is_file()
    }

    /// Also true for a voice name that could not be a file name.
    pub fn voice_missing(&self) -> bool {
        !self.voice.as_deref().is_some_and(Path::is_file)
    }
}

/// The folder one Kokoro release's files live in.
pub fn kokoro_dir(models_dir: &Path) -> PathBuf {
    models_dir.join(KOKORO_DIR)
}

pub fn model_path(models_dir: &Path, variant: ModelVariant) -> PathBuf {
    let file_name = match variant {
        ModelVariant::Int8 => "int8.onnx",
        ModelVariant::Fp32 => "fp32.onnx",
    };
    kokoro_dir(models_dir).join(file_name)
}

/// `None` when `voice` could not be a file name. Voice names are typed by the
/// user, so this keeps them inside the voices folder.
pub fn voice_path(models_dir: &Path, voice: &str) -> Option<PathBuf> {
    is_voice_name(voice).then(|| voices_dir(models_dir).join(format!("{voice}.bin")))
}

fn voices_dir(models_dir: &Path) -> PathBuf {
    kokoro_dir(models_dir).join("voices")
}

fn is_voice_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Voice ids with a `.bin` file in the voices folder, sorted.
pub fn installed_voices(models_dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(voices_dir(models_dir)) else {
        return Vec::new();
    };
    let mut voices: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "bin") && path.is_file())
        .filter_map(|path| path.file_stem()?.to_str().map(str::to_string))
        .filter(|stem| is_voice_name(stem))
        .collect();
    voices.sort();
    voices
}

/// Loads the files `target` names, or unloads the model when it names none.
/// `target` runs once the lock is held, so when several calls queue behind a
/// session, each sees the settings as they are by then.
pub fn preload(target: impl FnOnce() -> Option<ModelFiles>) -> Result<(), String> {
    let mut loaded = lock_loaded();
    let result = match target() {
        Some(files) => ensure_loaded(&mut loaded, &files),
        None => {
            if loaded.take().is_some() {
                info!("Unloaded the local voice");
            }
            Ok(())
        }
    };
    *lock_preload_error() = result.as_ref().err().cloned();
    result
}

/// Why the last preload failed, if it did.
pub fn preload_error() -> Option<String> {
    lock_preload_error().clone()
}

fn lock_preload_error() -> MutexGuard<'static, Option<String>> {
    PRELOAD_ERROR.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Takes the model for one speech session, loading or switching files first.
/// Setting `stop` cuts the inference in progress short.
pub fn open(files: &ModelFiles, stop: &Arc<AtomicBool>) -> Result<Loaded, String> {
    let mut loaded = lock_loaded();
    // The lock may come free only after a load or another session.
    if stop.load(Ordering::SeqCst) {
        return Err("Stopped before the local voice was ready".into());
    }
    ensure_loaded(&mut loaded, files)?;
    // Fresh per session: a late stop from the previous session must not end this one.
    let run_options =
        Arc::new(RunOptions::new().map_err(|e| format!("Cannot prepare ONNX Runtime: {e}"))?);
    let finished = Arc::new(AtomicBool::new(false));
    interrupt_on_stop(stop.clone(), finished.clone(), run_options.clone());
    Ok(Loaded {
        synth: loaded,
        run_options,
        finished,
    })
}

/// The model, held by one speech session until dropped.
pub struct Loaded {
    synth: MutexGuard<'static, Option<LocalSynth>>,
    run_options: Arc<RunOptions>,
    finished: Arc<AtomicBool>,
}

impl Loaded {
    /// `ends_sentence` is false where the text was cut only for its length.
    pub fn synthesize(
        &mut self,
        text: &str,
        speed: f32,
        ends_sentence: bool,
    ) -> Result<Vec<i16>, String> {
        match self.synth.as_mut() {
            Some(synth) => synth.synthesize(text, speed, ends_sentence, &self.run_options),
            None => Err("The local voice is not loaded".into()),
        }
    }
}

impl Drop for Loaded {
    fn drop(&mut self) {
        self.finished.store(true, Ordering::SeqCst);
    }
}

/// Watches `stop` while a session holds the model and ends its inference once
/// set, so a stopped session frees the model within a poll.
fn interrupt_on_stop(
    stop: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    run_options: Arc<RunOptions>,
) {
    let spawned = thread::Builder::new()
        .name("tts-interrupt".into())
        .spawn(move || {
            while !finished.load(Ordering::SeqCst) {
                if stop.load(Ordering::SeqCst) {
                    if let Err(e) = run_options.terminate() {
                        warn!("Failed to interrupt local synthesis: {e}");
                    }
                    return;
                }
                thread::sleep(STOP_POLL);
            }
        });
    if let Err(e) = spawned {
        warn!("Stopping will wait for the current sentence: {e}");
    }
}

fn lock_loaded() -> MutexGuard<'static, Option<LocalSynth>> {
    LOADED.lock().unwrap_or_else(|poisoned| {
        // A panic mid-inference may have left it half-updated.
        warn!("The local voice lock was poisoned; reloading the model");
        let mut loaded = poisoned.into_inner();
        *loaded = None;
        LOADED.clear_poison();
        loaded
    })
}

/// Makes `loaded` match `files`, reloading only what changed on disk.
fn ensure_loaded(loaded: &mut Option<LocalSynth>, files: &ModelFiles) -> Result<(), String> {
    let model = FileStamp::of(&files.model, "Kokoro model")?;
    let voice_path = files
        .voice
        .as_deref()
        .ok_or_else(|| format!("'{}' is not a voice name", files.voice_id))?;
    let voice = FileStamp::of(voice_path, "voice file")?;
    let reading = Reading {
        dialect: Dialect::for_voice(&files.voice_id),
        espeak: files.espeak.clone(),
    };

    match loaded {
        // Each part is swapped only once its replacement has loaded, so a
        // failure leaves the stamps matching what is held.
        Some(synth) => synth.refresh(model, voice, reading),
        None => {
            *loaded = Some(LocalSynth::load(model, voice, reading)?);
            Ok(())
        }
    }
}

/// A file as loaded: a replaced file at the same path loads again.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

impl FileStamp {
    fn of(path: &Path, what: &str) -> Result<Self, String> {
        let metadata = fs::metadata(path)
            .map_err(|e| format!("Cannot read the {what} at {}: {e}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

/// How the G2P reads text: which lexicon, and what reads the words it lacks.
#[derive(Debug, PartialEq, Eq)]
struct Reading {
    dialect: Dialect,
    espeak: Option<PathBuf>,
}

struct LocalSynth {
    session: Session,
    model: FileStamp,
    voice: Voice,
    g2p: G2P,
    reading: Reading,
}

impl LocalSynth {
    fn load(model: FileStamp, voice: FileStamp, reading: Reading) -> Result<Self, String> {
        let session = load_session(&model.path)?;
        let voice = Voice::load(voice)?;
        let g2p = build_g2p(&reading);
        Ok(Self {
            session,
            model,
            voice,
            g2p,
            reading,
        })
    }

    fn refresh(
        &mut self,
        model: FileStamp,
        voice: FileStamp,
        reading: Reading,
    ) -> Result<(), String> {
        if self.model != model {
            self.session = load_session(&model.path)?;
            self.model = model;
        }
        if self.voice.stamp != voice {
            self.voice = Voice::load(voice)?;
        }
        if self.reading != reading {
            self.g2p = build_g2p(&reading);
            self.reading = reading;
        }
        Ok(())
    }

    /// Speaks `text` as 24 kHz mono PCM. Text with nothing to pronounce gives
    /// no samples. Long input is cut into phrases.
    fn synthesize(
        &mut self,
        text: &str,
        speed: f32,
        ends_sentence: bool,
        run_options: &RunOptions,
    ) -> Result<Vec<i16>, String> {
        let (ids, dropped) = text_to_ids(&self.g2p, self.reading.dialect, text)?;
        if dropped > 0 {
            debug!("Dropped {dropped} symbols Kokoro has no token for");
        }
        let pieces: Vec<_> = split_tokens(&ids, PIECE_TOKENS)
            .into_iter()
            .filter(|p| has_phonemes(p))
            .collect();
        let keep = |ms: f32| (ms / speed * SAMPLE_RATE as f32 / 1000.0) as usize;
        let mut audio = Vec::new();
        for (i, piece) in pieces.iter().enumerate() {
            let waveform = run(
                &mut self.session,
                piece,
                self.voice.style(piece.len()),
                speed,
                run_options,
            )
            .map_err(|e| format!("Local synthesis failed: {e}"))?;
            let ends_sentence = ends_sentence && i + 1 == pieces.len();
            let pause = keep(pause_after_ms(piece, ends_sentence));
            let speech = trim_silence(&waveform, keep(LEAD_MS), pause);
            audio.extend(speech.iter().copied().map(to_pcm));
        }
        Ok(audio)
    }
}

fn environment() -> Result<&'static Environment, String> {
    ENVIRONMENT
        .get_or_init(|| {
            let environment = ort::init()
                .with_name("mouthy")
                .with_telemetry(false)
                .with_logger(Arc::new(log_ort))
                .build()
                .map_err(|e| format!("Cannot start ONNX Runtime: {e}"))?;
            // A custom logger starts at verbose.
            environment.set_log_level(LogLevel::Warning);
            Ok(environment)
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn log_ort(level: LogLevel, _category: &str, _id: &str, _location: &str, message: &str) {
    match level {
        LogLevel::Fatal | LogLevel::Error => error!(target: "ort", "{message}"),
        LogLevel::Warning => warn!(target: "ort", "{message}"),
        LogLevel::Info => debug!(target: "ort", "{message}"),
        LogLevel::Verbose => trace!(target: "ort", "{message}"),
    }
}

fn load_session(path: &Path) -> Result<Session, String> {
    let environment = environment()?;
    let started = Instant::now();
    let session = build_session(environment, path)
        .map_err(|e| format!("Cannot load the Kokoro model at {}: {e}", path.display()))?;
    let has_input = |name: &str| session.inputs().iter().any(|input| input.name() == name);
    let has_output = |name: &str| session.outputs().iter().any(|output| output.name() == name);
    if !(has_input("input_ids") && has_output("waveform")) {
        return Err(format!(
            "{} is not the onnx-community Kokoro export",
            path.display()
        ));
    }
    info!(
        "Loaded the Kokoro model {} in {} ms",
        path.display(),
        started.elapsed().as_millis()
    );
    Ok(session)
}

fn build_session(environment: &Environment, path: &Path) -> ort::Result<Session> {
    Session::builder(environment)?
        // rc.14's default policy routes to CoreML on macOS: ~3 s loads, slower runs.
        .with_auto_device(AutoDevicePolicy::PreferCPU)?
        // Spinning costs CPU between sentences and gained nothing measurable.
        .with_intra_op_spinning(false)?
        // A memory plan per input length fragments the arena as lengths vary,
        // so it grew ~40% past the longest run's need; no slower without.
        .with_memory_pattern(false)?
        .commit_from_file(path)
}

fn build_g2p(reading: &Reading) -> G2P {
    let started = Instant::now();
    let dialect = reading.dialect;
    let (g2p, unknown_words) = match &reading.espeak {
        Some(program) => (espeak::g2p(dialect, program.clone()), "espeak-ng"),
        None => (G2P::new(dialect.language()), "spelling"),
    };
    info!(
        "Loaded the {dialect:?} English lexicon ({unknown_words} for unknown words) in {} ms",
        started.elapsed().as_millis()
    );
    g2p
}

fn run(
    session: &mut Session,
    ids: &[i64],
    style: &[f32],
    speed: f32,
    run_options: &RunOptions,
) -> ort::Result<Vec<f32>> {
    let mut input = Vec::with_capacity(ids.len() + 2);
    input.push(0);
    input.extend_from_slice(ids);
    input.push(0);
    let len = input.len();
    let outputs = session.run_with_options(
        ort::inputs![
            "input_ids" => Tensor::from_array(([1usize, len], input))?,
            "style" => TensorRef::from_array_view(([1usize, STYLE_DIM], style))?,
            "speed" => Tensor::from_array(([1usize], vec![speed]))?,
        ],
        run_options,
    )?;
    let (_, waveform) = outputs["waveform"].try_extract_tensor::<f32>()?;
    Ok(waveform.to_vec())
}

fn to_pcm(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
}

/// Cuts `waveform` to its sound plus at most `lead` samples of silence before
/// and `trail` after. All silence gives nothing.
fn trim_silence(waveform: &[f32], lead: usize, trail: usize) -> &[f32] {
    let loud = |sample: &f32| sample.abs() > SILENCE;
    let (Some(first), Some(last)) = (
        waveform.iter().position(loud),
        waveform.iter().rposition(loud),
    ) else {
        return &[];
    };
    let end = (last + 1).saturating_add(trail).min(waveform.len());
    &waveform[first.saturating_sub(lead)..end]
}

/// The pause to leave after a piece. A sentence end gets a full pause
/// whatever its punctuation: misaki drops some marks, and a line break ends a
/// sentence without any.
fn pause_after_ms(piece: &[i64], ends_sentence: bool) -> f32 {
    if ends_sentence || piece.last().is_some_and(|id| PAUSE_IDS.contains(id)) {
        PAUSE_END_MS
    } else {
        PHRASE_END_MS
    }
}

/// A voice's style table: one 256-wide row per input length, raw
/// little-endian f32.
struct Voice {
    stamp: FileStamp,
    rows: Vec<f32>,
}

impl Voice {
    fn load(stamp: FileStamp) -> Result<Self, String> {
        let bytes = fs::read(&stamp.path)
            .map_err(|e| format!("Cannot read the voice file {}: {e}", stamp.path.display()))?;
        let rows = parse_style_rows(&bytes)
            .ok_or_else(|| format!("{} is not a Kokoro voice file", stamp.path.display()))?;
        Ok(Self { stamp, rows })
    }

    /// The row for `tokens` ids (pads excluded), clamped to the last row.
    fn style(&self, tokens: usize) -> &[f32] {
        style_row(&self.rows, tokens)
    }
}

fn parse_style_rows(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4 * STYLE_DIM) {
        return None;
    }
    Some(
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect(),
    )
}

fn style_row(rows: &[f32], tokens: usize) -> &[f32] {
    let row = tokens.min(rows.len() / STYLE_DIM - 1);
    &rows[row * STYLE_DIM..(row + 1) * STYLE_DIM]
}

/// Kokoro ids for `text`, and how many symbols had no id.
fn text_to_ids(g2p: &G2P, dialect: Dialect, text: &str) -> Result<(Vec<i64>, usize), String> {
    let (raw, _) = g2p
        .g2p(&reading(g2p, text))
        .map_err(|e| format!("Cannot phonemise the text: {e}"))?;
    Ok(token_ids(&to_kokoro_phonemes(&raw, dialect)))
}

/// `text` as misaki-rs should get it: plain characters, abbreviations and
/// numbers in words and initialisms spelled.
fn reading(g2p: &G2P, text: &str) -> String {
    let words = expand_numbers(&expand_abbreviations(&plain_text(text)));
    spell_out(&words, &g2p.lexicon)
}

/// Initialisms read as misaki reads a proper noun it spells: each letter's
/// name from the gold lexicon, the last one stressed ("ˌApˌiːˈI"). They reach
/// misaki as a `[letters](/phonemes/)` override, because its tagger misreads
/// spaced letters: "I use A P I keys" tags the "A" as an article, "ɐ".
impl text::Lexicon for Lexicon {
    fn has(&self, word: &str) -> bool {
        self.golds.contains_key(word)
    }

    fn spell(&self, letters: &str, plural: bool) -> String {
        let names: Option<String> = letters
            .chars()
            .map(|c| match self.golds.get(&c.to_string()) {
                Some(PhonemeEntry::Simple(name)) => Some(name.as_str()),
                _ => None,
            })
            .collect();
        let Some(names) = names else {
            return letters
                .chars()
                .map(String::from)
                .collect::<Vec<_>>()
                .join(" ");
        };
        // misaki's `get_nnp`: every stress secondary, then the last primary.
        let mut phonemes = self.apply_stress(&names, Some(0.0));
        if let Some(at) = phonemes.rfind('ˌ') {
            phonemes.replace_range(at..at + 'ˌ'.len_utf8(), "ˈ");
        }
        let s = if plural {
            phonemes = self.append_s(&phonemes);
            "s"
        } else {
            ""
        };
        format!("[{letters}{s}](/{phonemes}/)")
    }
}

/// Accented and ligature letters as the plain letters misaki-rs looks up;
/// its lexicons hold ASCII words only.
const FOLDS: &[(&str, &str)] = &[
    ("àáâãäåāăą", "a"),
    ("ÀÁÂÃÄÅĀĂĄ", "A"),
    ("çćĉċč", "c"),
    ("ÇĆĈĊČ", "C"),
    ("ďđ", "d"),
    ("ĎĐ", "D"),
    ("èéêëēĕėęě", "e"),
    ("ÈÉÊËĒĔĖĘĚ", "E"),
    ("ĝğġģ", "g"),
    ("ĜĞĠĢ", "G"),
    ("ìíîïĩīĭįı", "i"),
    ("ÌÍÎÏĨĪĬĮİ", "I"),
    ("ĺļľŀł", "l"),
    ("ĹĻĽĿŁ", "L"),
    ("ñńņň", "n"),
    ("ÑŃŅŇ", "N"),
    ("òóôõöøōŏő", "o"),
    ("ÒÓÔÕÖØŌŎŐ", "O"),
    ("ŕŗř", "r"),
    ("ŔŖŘ", "R"),
    ("śŝşšș", "s"),
    ("ŚŜŞŠȘ", "S"),
    ("ţťŧț", "t"),
    ("ŢŤŦȚ", "T"),
    ("ùúûüũūŭůűų", "u"),
    ("ÙÚÛÜŨŪŬŮŰŲ", "U"),
    ("ýÿŷ", "y"),
    ("ÝŸŶ", "Y"),
    ("źżž", "z"),
    ("ŹŻŽ", "Z"),
    ("ß", "ss"),
    ("æ", "ae"),
    ("Æ", "AE"),
    ("œ", "oe"),
    ("Œ", "OE"),
    ("ﬀ", "ff"),
    ("ﬁ", "fi"),
    ("ﬂ", "fl"),
    ("ﬃ", "ffi"),
    ("ﬄ", "ffl"),
    ("ﬅﬆ", "st"),
];

/// Text as misaki-rs reads it best. It splits a word at a curly apostrophe
/// ("don’t" → "don", "❓", "t"), spells an unknown word letter by letter and
/// reads a dash as a plain space.
fn plain_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        match c {
            // misaki-rs takes `[text](…)` as a pronunciation, and an earlier
            // `[` would swallow the words before a `spell_out` override.
            '[' => out.push('('),
            ']' => out.push(')'),
            '’' | '‘' | 'ʼ' => out.push('\''),
            '“' | '”' | '„' => out.push('"'),
            // Minus sign.
            '\u{2212}' => out.push('-'),
            // Accents typed as a separate mark after the letter ("e" + U+0301).
            '\u{300}'..='\u{36f}' if out.ends_with(|c: char| c.is_ascii_alphabetic()) => {}
            // Soft hyphen, zero-width space, byte order mark.
            '\u{ad}' | '\u{200b}' | '\u{feff}' => {}
            '\u{a0}' => out.push(' '),
            '—' | '–' if is_spaced_dash(&text[..i], &text[i + c.len_utf8()..]) => {
                // A comma keeps the dash's pause.
                out.truncate(out.trim_end().len());
                if !out.is_empty() && !out.ends_with([',', ';', ':', '.', '!', '?']) {
                    out.push(',');
                }
            }
            c if c.is_ascii() => out.push(c),
            c => match FOLDS.iter().find(|(from, _)| from.contains(c)) {
                Some((_, to)) => out.push_str(to),
                None => out.push(c),
            },
        }
    }
    out
}

/// " — " between words; "1914 – 1918" is a range.
fn is_spaced_dash(before: &str, after: &str) -> bool {
    before.ends_with(char::is_whitespace)
        && after.starts_with(char::is_whitespace)
        && !joins_numbers(before, after)
}

/// Turns misaki-rs output into the symbols Kokoro was trained on: tied
/// diphthongs and affricates become single symbols, and spacing follows
/// misaki's Python output.
fn to_kokoro_phonemes(raw: &str, dialect: Dialect) -> String {
    let untied = TIES_COMMON
        .iter()
        .chain(dialect.ties())
        .fold(raw.to_string(), |s, (from, to)| s.replace(from, to))
        // Triphthongs such as `a‍ɪ‍ə` keep their second tie.
        .replace('\u{200d}', "");
    tidy_spacing(&untied)
}

/// misaki-rs ends every token with a space, punctuation included; Kokoro was
/// trained with punctuation attached to the word before it.
fn tidy_spacing(phonemes: &str) -> String {
    let mut out = String::with_capacity(phonemes.len());
    for c in phonemes.chars() {
        match c {
            ' ' if out.is_empty() || out.ends_with(' ') => {}
            ';' | ':' | ',' | '.' | '!' | '?' => {
                if out.ends_with(' ') {
                    out.pop();
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.trim_end().to_string()
}

/// Ids for every symbol Kokoro knows, and how many symbols it had to drop
/// (misaki's `❓` for characters it cannot read).
fn token_ids(phonemes: &str) -> (Vec<i64>, usize) {
    let mut ids = Vec::with_capacity(phonemes.len());
    let mut dropped = 0;
    for c in phonemes.chars() {
        match vocab_id(c) {
            Some(id) => ids.push(id),
            None => dropped += 1,
        }
    }
    (ids, dropped)
}

fn vocab_id(c: char) -> Option<i64> {
    VOCAB
        .iter()
        .find(|(symbol, _)| *symbol == c)
        .map(|(_, id)| *id)
}

pub(super) fn in_vocab(c: char) -> bool {
    vocab_id(c).is_some()
}

/// Ids up to the space are punctuation, which alone says nothing.
fn has_phonemes(ids: &[i64]) -> bool {
    ids.iter().any(|&id| id > SPACE_ID)
}

/// Cuts ids into runs of at most `max`: after the last pause that fits, else
/// at the last space, else at `max`. Nothing is dropped but the spaces at a
/// cut.
fn split_tokens(ids: &[i64], max: usize) -> Vec<&[i64]> {
    let mut pieces = Vec::new();
    let mut rest = trim_spaces(ids);
    while rest.len() > max {
        let window = &rest[..max];
        let cut_after = |found: Option<usize>| {
            found
                .map(|i| i + 1)
                .filter(|&cut| has_phonemes(&window[..cut]))
        };
        let cut = cut_after(window.iter().rposition(|id| PAUSE_IDS.contains(id)))
            .or_else(|| cut_after(window.iter().rposition(|&id| id == SPACE_ID)))
            .unwrap_or(max);
        let (piece, tail) = rest.split_at(cut);
        pieces.push(trim_spaces(piece));
        rest = trim_spaces(tail);
    }
    if !rest.is_empty() {
        pieces.push(rest);
    }
    pieces
}

fn trim_spaces(mut run: &[i64]) -> &[i64] {
    while let [SPACE_ID, rest @ ..] = run {
        run = rest;
    }
    while let [rest @ .., SPACE_ID] = run {
        run = rest;
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::text::split_sentences;
    use misaki_rs::fallback::FallbackError;
    use misaki_rs::Fallback;
    use std::collections::{BTreeSet, HashMap};

    fn kokoro(raw: &str, dialect: Dialect) -> String {
        to_kokoro_phonemes(raw, dialect)
    }

    #[test]
    fn british_ties_become_single_symbols() {
        // The triphthong's second tie has no symbol of its own.
        assert_eq!(
            kokoro("ɡˈə\u{200d}ʊ fa\u{200d}ɪ\u{200d}ə", Dialect::British),
            "ɡˈQ fIə"
        );
    }

    #[test]
    fn american_ties_become_single_symbols() {
        // `ː‍ɹ` must be replaced before every other `ː` is dropped.
        assert_eq!(
            kokoro("bˈɜː\u{200d}ɹd fˈɑːðɚ", Dialect::American),
            "bˈɜɹd fˈɑðɚ"
        );
    }

    #[test]
    fn gb_ties_stay_british() {
        // US-only rows must not touch British output, and vice versa.
        assert_eq!(kokoro("ɡˈo\u{200d}ʊ fˈɑː", Dialect::British), "ɡˈoʊ fˈɑː");
        assert_eq!(kokoro("ɡˈə\u{200d}ʊ", Dialect::American), "ɡˈəʊ");
    }

    #[test]
    fn spacing_collapses_runs_and_trims() {
        assert_eq!(tidy_spacing("  a   b  "), "a b");
    }

    #[test]
    fn spacing_attaches_punctuation_to_the_word_before() {
        assert_eq!(
            tidy_spacing("wˈɛl , həlˈQ ðɛː . ɔːl ; ðɪs : wˈʌt ? jˈɛs ! "),
            "wˈɛl, həlˈQ ðɛː. ɔːl; ðɪs: wˈʌt? jˈɛs!"
        );
    }

    #[test]
    fn unknown_symbols_are_dropped_and_counted() {
        let (ids, dropped) = token_ids("a❓b");
        assert_eq!(ids, vec![43, 44]);
        assert_eq!(dropped, 1);
    }

    #[test]
    fn vocab_ids_are_unique() {
        let ids: BTreeSet<i64> = VOCAB.iter().map(|(_, id)| *id).collect();
        let symbols: BTreeSet<char> = VOCAB.iter().map(|(c, _)| *c).collect();
        assert_eq!(ids.len(), VOCAB.len());
        assert_eq!(symbols.len(), VOCAB.len());
        assert!(!ids.contains(&0), "0 is the pad");
    }

    fn lexicon_symbols(entries: &HashMap<String, PhonemeEntry>) -> impl Iterator<Item = &str> {
        entries.values().flat_map(|entry| match entry {
            PhonemeEntry::Simple(phonemes) => vec![phonemes.as_str()],
            PhonemeEntry::Tagged(tagged) => tagged.values().flatten().map(String::as_str).collect(),
        })
    }

    fn unmapped(dialect: Dialect, lexicons: &[HashMap<String, PhonemeEntry>]) -> BTreeSet<char> {
        let mut missing = BTreeSet::new();
        for phonemes in lexicons.iter().flat_map(lexicon_symbols) {
            for c in kokoro(phonemes, dialect).chars() {
                if !VOCAB.iter().any(|(symbol, _)| *symbol == c) {
                    missing.insert(c);
                }
            }
        }
        missing
    }

    #[test]
    fn every_british_lexicon_symbol_maps_into_the_vocab() {
        let lexicons = [
            misaki_rs::data::load_gb_gold(),
            misaki_rs::data::load_gb_silver(),
        ];
        assert_eq!(unmapped(Dialect::British, &lexicons), BTreeSet::new());
    }

    #[test]
    fn every_american_lexicon_symbol_maps_into_the_vocab() {
        let lexicons = [
            misaki_rs::data::load_us_gold(),
            misaki_rs::data::load_us_silver(),
        ];
        assert_eq!(unmapped(Dialect::American, &lexicons), BTreeSet::new());
    }

    #[test]
    fn british_sentences_tokenise_without_drops() {
        let g2p = G2P::new(Language::EnglishGB);
        for text in [
            "The quick brown fox jumps over the lazy dog near the riverbank in Oxford.",
            "I can't believe it's already October; we haven't even booked the hotel!",
            "Is this your coat, or hers?",
        ] {
            let (raw, _) = g2p.g2p(text).unwrap();
            let phonemes = to_kokoro_phonemes(&raw, Dialect::British);
            let (ids, dropped) = token_ids(&phonemes);
            assert_eq!(dropped, 0, "{text} -> {phonemes}");
            assert!(!ids.is_empty());
            assert!(
                !phonemes.contains("  ") && !phonemes.contains(" ."),
                "{phonemes}"
            );
        }
    }

    /// Records the words misaki-rs has no entry for, which it would otherwise
    /// spell letter by letter.
    struct Unknown(Arc<Mutex<Vec<String>>>);

    impl Fallback for Unknown {
        fn phonemize(&self, word: &str) -> Result<String, FallbackError> {
            self.0.lock().unwrap().push(word.to_string());
            Ok(String::new())
        }
    }

    fn recording_g2p(language: Language) -> (G2P, Arc<Mutex<Vec<String>>>) {
        let unknown = Arc::new(Mutex::new(Vec::new()));
        let g2p = G2P::with_fallback(language, Some(Box::new(Unknown(unknown.clone()))));
        (g2p, unknown)
    }

    #[test]
    fn curly_apostrophes_and_accents_read_as_plain_words() {
        let (g2p, unknown) = recording_g2p(Language::EnglishGB);
        for (typed, plain) in [
            ("don’t", "don't"),
            ("it’s", "it's"),
            ("user’s", "user's"),
            ("o’clock", "o'clock"),
            ("café", "cafe"),
            ("naïve", "naive"),
            ("ﬁnal", "final"),
            ("donʼt", "don't"),
            ("‘Café’", "'Cafe'"),
        ] {
            let (read, _) = g2p.g2p(&plain_text(typed)).unwrap();
            let (expected, _) = g2p.g2p(plain).unwrap();
            assert_eq!(read, expected, "{typed}");
            assert!(!read.contains('❓'), "{typed} -> {read}");
        }
        assert!(unknown.lock().unwrap().is_empty(), "{unknown:?}");
    }

    #[test]
    fn decomposed_accents_read_as_plain_words() {
        let (g2p, unknown) = recording_g2p(Language::EnglishGB);
        for (typed, plain) in [("cafe\u{301}", "cafe"), ("re\u{301}sume\u{301}", "resume")] {
            assert_eq!(plain_text(typed), plain);
            let (read, _) = g2p.g2p(&plain_text(typed)).unwrap();
            let (expected, _) = g2p.g2p(plain).unwrap();
            assert_eq!(read, expected, "{typed}");
        }
        assert!(unknown.lock().unwrap().is_empty(), "{unknown:?}");
        // Not after a plain letter: left for misaki-rs.
        assert_eq!(plain_text("日\u{301}"), "日\u{301}");
    }

    /// Kokoro symbols for `text` as the local voice reads it.
    fn read(g2p: &G2P, dialect: Dialect, text: &str) -> String {
        let (raw, _) = g2p.g2p(&reading(g2p, text)).unwrap();
        to_kokoro_phonemes(&raw, dialect)
    }

    #[test]
    fn initialisms_read_as_letter_names() {
        let gb = G2P::new(Language::EnglishGB);
        let read_gb = |text| read(&gb, Dialect::British, text);
        assert_eq!(read_gb("Your ID"), "jˌɔː ˌIdˈiː");
        assert_eq!(read_gb("the API URL"), "ði ˌApˌiːˈI jˌuːˌɑːˈɛl");
        // Spaced letters would get the article's "ɐ" for this "A".
        assert_eq!(read_gb("I use API keys"), "ˌI jˈuːs ˌApˌiːˈI kˈiːz");
        assert_eq!(read_gb("IDs"), "ˌIdˈiːz");
        assert_eq!(read_gb("The U.S.'s debt"), "ði jˌuːˈɛsɪz dˈɛt");
        assert_eq!(read_gb("the US government"), "ði jˌuːˈɛs ɡˈʌvənmənt");
        assert_eq!(read_gb("at 7 a.m."), "at sˈɛvən ˌAˈɛm.");
        let us = G2P::new(Language::EnglishUS);
        assert_eq!(read(&us, Dialect::American, "Your ID"), "jˌʊɹ ˌIdˈi");
        assert_eq!(read(&us, Dialect::American, "macOS"), "mˈæk ˌOˈɛs");
    }

    #[test]
    fn words_in_capitals_and_known_names_read_as_before() {
        let g2p = G2P::new(Language::EnglishGB);
        for text in [
            "NASA",
            "RAM",
            "GIF",
            "iOS 17",
            "DO IT NOW",
            "UNESCO",
            "COVID-19",
        ] {
            let (raw, _) = g2p.g2p(text).unwrap();
            assert_eq!(
                read(&g2p, Dialect::British, text),
                to_kokoro_phonemes(&raw, Dialect::British),
                "{text}"
            );
        }
    }

    #[test]
    fn camel_case_reads_as_its_words() {
        let g2p = G2P::new(Language::EnglishGB);
        let read_gb = |text| read(&g2p, Dialect::British, text);
        assert_eq!(read_gb("GitHub"), read_gb("Git Hub"));
        assert_eq!(read_gb("HTMLParser"), "ˌAʧtˌiːˌɛmˈɛl pˈɑːsə");
        assert_eq!(read_gb("iPhone"), "ˈI fˈQn");
        assert_eq!(read_gb("JSON"), "ʤˈAsᵊn");
        assert_eq!(read_gb("YAML"), "jˈam ˌɛl");
    }

    #[test]
    fn a_stop_inside_a_name_is_no_sentence_end() {
        let g2p = G2P::new(Language::EnglishGB);
        let read_gb = |text| read(&g2p, Dialect::British, text);
        assert_eq!(
            read_gb("Use Node.js today."),
            "jˈuːs nˈQd dˈɒt ʤˌAˈɛs tədˈA."
        );
        assert_eq!(read_gb("Tools, e.g. hammers."), "tˈuːlz, ˌiːʤˈiː hˈaməz.");
        assert_eq!(read_gb("the U.S. team"), "ði jˌuːˈɛs tˈiːm");
        assert_eq!(read_gb("Live in the U.S."), "lˈɪv ɪn ði jˌuːˈɛs.");
        assert!(read_gb("in the U.S. Analysts expect").contains("jˌuːˈɛs. "));
        assert_eq!(
            read_gb("README.md and ort.pyke.io"),
            "ɹˈiːd mˌiː dˈɒt ˌɛmdˈiː and ˈɔːt dˈɒt pˈIk dˈɒt ˈIQ"
        );
        assert_eq!(read_gb("example.com"), read_gb("example dot com"));
        assert_eq!(
            read_gb("Get v2.0 or 3.14"),
            read_gb("Get version two point zero or three point one four")
        );
    }

    #[test]
    fn titles_and_abbreviations_lose_their_stop() {
        let g2p = G2P::new(Language::EnglishGB);
        let sentence = "Dr. Smith met Mr. Jones at 7.30pm.";
        assert_eq!(
            reading(&g2p, sentence),
            "Doctor Smith met Mister Jones at seven thirty PM."
        );
        let read_gb = |text| read(&g2p, Dialect::British, text);
        assert_eq!(read_gb(sentence).matches('.').count(), 1);
        assert_eq!(read_gb("Ms. Smith"), "mˈɪz smˈɪθ");
        assert_eq!(read_gb("Baker St. is long."), "bˈAkə stɹˈiːt ɪz lˈɒŋ.");
        assert_eq!(read_gb("fruit, etc. Then"), "fɹˈuːt, ˈɛt sˈɛtəɹə. ðˈɛn");
    }

    #[test]
    fn brackets_in_the_text_keep_their_words() {
        let g2p = G2P::new(Language::EnglishGB);
        let read_gb = |text| read(&g2p, Dialect::British, text);
        for (text, parenthesised) in [
            (
                "He said [the CEO] would resign.",
                "He said (the CEO) would resign.",
            ),
            (
                "[TODO: fix the API before release]",
                "(TODO: fix the API before release)",
            ),
            (
                "Use the PDF (see [the FAQ]).",
                "Use the PDF (see (the FAQ)).",
            ),
            (
                "Read [the API docs][1] first.",
                "Read (the API docs)(1) first.",
            ),
        ] {
            assert_eq!(read_gb(text), read_gb(parenthesised), "{text}");
        }
        assert_eq!(
            read_gb("He said [the CEO] would resign."),
            "hˌiː sˈɛd ðə sˌiːˌiːˈQ wʊd ɹɪzˈIn."
        );
        assert_eq!(
            read_gb("[TODO: fix the API before release]"),
            "tə dˈuː: fˈɪks ði ˌApˌiːˈI bɪfˌɔː ɹɪlˈiːs"
        );
    }

    #[test]
    fn every_letter_has_a_stressed_name_in_both_lexicons() {
        for gold in [
            misaki_rs::data::load_gb_gold(),
            misaki_rs::data::load_us_gold(),
        ] {
            for letter in 'A'..='Z' {
                match gold.get(&letter.to_string()) {
                    Some(PhonemeEntry::Simple(name)) => {
                        assert!(name.contains('ˈ'), "{letter}: {name}")
                    }
                    other => panic!("{letter}: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn a_minus_sign_reads_as_minus() {
        let (g2p, _) = recording_g2p(Language::EnglishGB);
        let ids = |text| text_to_ids(&g2p, Dialect::British, text).unwrap();
        assert_eq!(ids("\u{2212}5 degrees"), ids("minus five degrees"));
    }

    #[test]
    fn curly_double_quotes_read_as_straight_ones() {
        let (g2p, _) = recording_g2p(Language::EnglishGB);
        let ids = |text| text_to_ids(&g2p, Dialect::British, text).unwrap();
        let (curly, dropped) = ids("She said “no” twice.");
        assert_eq!(dropped, 0);
        assert_eq!((curly.clone(), dropped), ids("She said \"no\" twice."));
        assert!(
            !curly.windows(2).any(|pair| pair == [SPACE_ID, SPACE_ID]),
            "{curly:?}"
        );
        assert_eq!(plain_text("„no“"), "\"no\"");
    }

    #[test]
    fn a_sentence_with_curly_apostrophes_loses_no_symbol() {
        let (g2p, unknown) = recording_g2p(Language::EnglishGB);
        let text = "I don’t think it’s the user’s fault that the naïve café shuts at nine o’clock.";
        let (ids, dropped) = text_to_ids(&g2p, Dialect::British, text).unwrap();
        assert_eq!(dropped, 0);
        assert!(has_phonemes(&ids));
        assert!(unknown.lock().unwrap().is_empty(), "{unknown:?}");
    }

    #[test]
    fn invisible_characters_go_and_no_break_spaces_become_spaces() {
        assert_eq!(
            plain_text("soft\u{ad}ware\u{200b} \u{feff}ten\u{a0}pounds"),
            "software ten pounds"
        );
        assert_eq!(plain_text("Straße Œuvre ﬂoor"), "Strasse OEuvre floor");
        assert_eq!(plain_text("日本 🎉"), "日本 🎉");
    }

    #[test]
    fn a_spaced_dash_becomes_a_comma() {
        assert_eq!(
            plain_text("Tea — or coffee – is fine"),
            "Tea, or coffee, is fine"
        );
        assert_eq!(plain_text("Wait, — no."), "Wait, no.");
        for kept in ["well—maybe", "1914 – 1918", "£5 – £10", "— Yes", "fine —"] {
            assert_eq!(plain_text(kept), kept);
        }
        let (g2p, _) = recording_g2p(Language::EnglishGB);
        let (ids, _) = text_to_ids(&g2p, Dialect::British, "Tea — or coffee").unwrap();
        assert!(ids.contains(&3), "a comma id: {ids:?}");
    }

    #[test]
    fn voice_prefix_picks_the_dialect() {
        assert_eq!(Dialect::for_voice("bm_lewis"), Dialect::British);
        assert_eq!(Dialect::for_voice("bf_emma"), Dialect::British);
        assert_eq!(Dialect::for_voice("BM_Lewis"), Dialect::British);
        assert_eq!(Dialect::for_voice("af_heart"), Dialect::American);
        assert_eq!(Dialect::for_voice("am_adam"), Dialect::American);
        assert_eq!(Dialect::for_voice("ef_dora"), Dialect::American);
    }

    fn numbered_rows(count: usize) -> Vec<f32> {
        (0..count)
            .flat_map(|row| std::iter::repeat_n(row as f32, STYLE_DIM))
            .collect()
    }

    #[test]
    fn style_row_is_the_token_count() {
        let rows = numbered_rows(510);
        assert_eq!(style_row(&rows, 0)[0], 0.0);
        assert_eq!(style_row(&rows, 17)[0], 17.0);
        assert_eq!(style_row(&rows, 17).len(), STYLE_DIM);
        assert_eq!(style_row(&rows, 509)[STYLE_DIM - 1], 509.0);
    }

    #[test]
    fn style_row_clamps_to_the_last_row() {
        let rows = numbered_rows(510);
        assert_eq!(style_row(&rows, 510)[0], 509.0);
        assert_eq!(style_row(&rows, 10_000)[0], 509.0);
    }

    #[test]
    fn voice_file_parses_little_endian_rows() {
        let mut bytes = Vec::new();
        for value in [1.5f32, -2.0] {
            for _ in 0..STYLE_DIM {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        let rows = parse_style_rows(&bytes).unwrap();
        assert_eq!(rows.len(), 2 * STYLE_DIM);
        assert_eq!(style_row(&rows, 1)[0], -2.0);
    }

    #[test]
    fn voice_file_rejects_a_partial_row() {
        assert!(parse_style_rows(&[]).is_none());
        assert!(parse_style_rows(&[0; 4 * STYLE_DIM - 4]).is_none());
        assert!(parse_style_rows(&[0; 4 * STYLE_DIM + 1]).is_none());
    }

    #[test]
    fn short_input_is_one_piece() {
        let ids = vec![43; PIECE_TOKENS];
        assert_eq!(split_tokens(&ids, PIECE_TOKENS), vec![&ids[..]]);
        assert!(split_tokens(&[], PIECE_TOKENS).is_empty());
        assert!(split_tokens(&[SPACE_ID, SPACE_ID], PIECE_TOKENS).is_empty());
    }

    #[test]
    fn long_input_is_cut_after_the_last_pause_that_fits() {
        // "aaa, aaa, …" well past the limit.
        let phrase = [43, 43, 43, 3, SPACE_ID];
        let ids: Vec<i64> = phrase
            .iter()
            .copied()
            .cycle()
            .take(8 * PIECE_TOKENS)
            .collect();
        let pieces = split_tokens(&ids, PIECE_TOKENS);
        assert!(pieces.len() >= 8);
        for piece in &pieces {
            assert!(piece.len() <= PIECE_TOKENS);
            assert_ne!(piece.first(), Some(&SPACE_ID));
        }
        for piece in &pieces[..pieces.len() - 1] {
            assert_eq!(piece.last(), Some(&3), "cut after a comma");
        }
        let kept: usize = pieces.iter().map(|p| p.len()).sum();
        let spaces_at_cuts = pieces.len() - 1;
        assert_eq!(
            kept + spaces_at_cuts,
            ids.len() - 1,
            "only cut spaces are lost"
        );
    }

    #[test]
    fn long_input_without_pauses_is_cut_at_a_space_then_anywhere() {
        let words: Vec<i64> = [43, 43, 43, 43, SPACE_ID]
            .iter()
            .copied()
            .cycle()
            .take(4 * PIECE_TOKENS)
            .collect();
        let pieces = split_tokens(&words, PIECE_TOKENS);
        assert_eq!(pieces.len(), 4);
        for piece in &pieces {
            assert!(piece.len() <= PIECE_TOKENS);
            assert_eq!((piece.first(), piece.last()), (Some(&43), Some(&43)));
        }

        let solid = vec![43; 2 * PIECE_TOKENS + 100];
        let pieces = split_tokens(&solid, PIECE_TOKENS);
        assert_eq!(
            pieces.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![PIECE_TOKENS, PIECE_TOKENS, 100]
        );
    }

    #[test]
    fn punctuation_alone_has_no_phonemes() {
        assert!(!has_phonemes(&[]));
        assert!(!has_phonemes(&[4, SPACE_ID, 4, 1, 10]));
        assert!(has_phonemes(&[4, 43]));
    }

    #[test]
    fn a_cut_never_leaves_a_piece_of_punctuation_alone() {
        // A comma first, then one long word: cutting after the comma would
        // send a piece with nothing to say.
        let mut ids = vec![3, SPACE_ID];
        ids.extend(std::iter::repeat_n(43, 2 * PIECE_TOKENS));
        let pieces = split_tokens(&ids, PIECE_TOKENS);
        assert!(pieces.iter().all(|p| has_phonemes(p)), "{pieces:?}");
    }

    #[test]
    fn a_long_sentence_is_read_in_one_inference() {
        let g2p = G2P::new(Language::EnglishGB);
        let ids = |text: &str| text_to_ids(&g2p, Dialect::British, text).unwrap().0;
        let sentence = "The committee that met on the first floor of the old library building \
                        every Thursday afternoon decided after a long and careful discussion \
                        that the annual report should be rewritten from the beginning in plain \
                        language so that every member of the public could understand exactly \
                        what had been done with the money during the year.";
        let chunks = split_sentences(sentence, MAX_CHUNK_CHARS);
        assert_eq!(chunks.len(), 1);
        assert_eq!(split_tokens(&ids(&chunks[0].text), PIECE_TOKENS).len(), 1);

        // As long as a chunk gets: the limit plus a tail too short to stand alone.
        let prose = sentence.repeat(2);
        let longest: String = prose.chars().take(MAX_CHUNK_CHARS + 20).collect();
        assert!(
            ids(&longest).len() <= PIECE_TOKENS,
            "{} ids",
            ids(&longest).len()
        );
    }

    #[test]
    fn pcm_conversion_clamps() {
        assert_eq!(to_pcm(0.0), 0);
        assert_eq!(to_pcm(1.0), i16::MAX);
        assert_eq!(to_pcm(2.0), i16::MAX);
        assert_eq!(to_pcm(-2.0), -i16::MAX);
    }

    /// `silent` samples of near-silence, `loud` of sound, then `silent` again.
    fn padded(silent: usize, loud: usize) -> Vec<f32> {
        let mut waveform = vec![0.002; silent];
        waveform.extend((0..loud).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }));
        waveform.extend(vec![-0.002; silent]);
        waveform
    }

    #[test]
    fn long_silence_is_trimmed_to_the_caps() {
        let waveform = padded(100, 10);
        let trimmed = trim_silence(&waveform, 5, 20);
        assert_eq!(trimmed.len(), 5 + 10 + 20);
        assert_eq!(trimmed[5], 0.5);
        assert_eq!(trimmed[14], -0.5);
    }

    #[test]
    fn silence_within_the_caps_is_kept_whole() {
        for silent in [0, 3] {
            let waveform = padded(silent, 10);
            assert_eq!(trim_silence(&waveform, 5, 20), &waveform[..]);
        }
    }

    #[test]
    fn all_silence_gives_nothing() {
        assert!(trim_silence(&padded(50, 0), 5, 20).is_empty());
        assert!(trim_silence(&[SILENCE, -SILENCE], 5, 20).is_empty());
        assert!(trim_silence(&[], 5, 20).is_empty());
    }

    #[test]
    fn a_piece_cut_at_a_pause_keeps_it() {
        let pause = |phonemes: &str| pause_after_ms(&token_ids(phonemes).0, false);
        assert_eq!(pause("ab."), PAUSE_END_MS);
        assert_eq!(pause("ab,"), PAUSE_END_MS);
        assert_eq!(pause("ab"), PHRASE_END_MS);
        assert_eq!(pause("a. b"), PHRASE_END_MS);
    }

    #[test]
    fn a_sentence_end_pauses_without_punctuation() {
        assert_eq!(pause_after_ms(&token_ids("ab").0, true), PAUSE_END_MS);
        assert_eq!(pause_after_ms(&token_ids("ab.").0, true), PAUSE_END_MS);
    }

    #[test]
    fn model_files_follow_the_documented_layout() {
        let models = Path::new("/models");
        let files = ModelFiles::new(models, ModelVariant::Int8, "bm_lewis");
        assert_eq!(files.model, Path::new("/models/kokoro-v1.0/int8.onnx"));
        assert_eq!(
            files.voice.as_deref(),
            Some(Path::new("/models/kokoro-v1.0/voices/bm_lewis.bin"))
        );
        assert_eq!(
            ModelFiles::new(models, ModelVariant::Fp32, "x").model,
            Path::new("/models/kokoro-v1.0/fp32.onnx")
        );
    }

    #[test]
    fn voice_names_cannot_leave_the_voices_folder() {
        let models = Path::new("/models");
        for name in ["../secret", "a/b", "", "bm lewis", "voice.bin"] {
            assert_eq!(
                ModelFiles::new(models, ModelVariant::Int8, name).voice,
                None,
                "{name:?}"
            );
        }
    }

    #[test]
    fn missing_files_are_reported_model_first() {
        let dir = tempfile::tempdir().unwrap();
        let files = ModelFiles::new(dir.path(), ModelVariant::Int8, "bm_lewis");
        assert!(files.missing().unwrap().ends_with("int8.onnx"));
        assert!(files.model_missing() && files.voice_missing());

        let voices = voices_dir(dir.path());
        fs::create_dir_all(&voices).unwrap();
        fs::write(dir.path().join(KOKORO_DIR).join("int8.onnx"), b"model").unwrap();
        assert!(files.missing().unwrap().ends_with("bm_lewis.bin"));
        assert!(!files.model_missing() && files.voice_missing());

        fs::write(voices.join("bm_lewis.bin"), b"voice").unwrap();
        assert_eq!(files.missing(), None);
        assert!(!files.model_missing() && !files.voice_missing());
        for voice in ["../x", "ef_dora", "af_bella+af_sky", "my favourite voice"] {
            let files = ModelFiles::new(dir.path(), ModelVariant::Int8, voice);
            assert!(files.voice_missing(), "{voice}");
            assert!(files.missing().is_some(), "{voice}");
        }
    }

    #[test]
    fn a_failed_preload_is_kept_until_the_next_one() {
        let dir = tempfile::tempdir().unwrap();
        let error = preload(|| Some(ModelFiles::new(dir.path(), ModelVariant::Int8, "bm_lewis")))
            .unwrap_err();
        assert_eq!(preload_error(), Some(error));
        preload(|| None).unwrap();
        assert_eq!(preload_error(), None);
    }

    #[test]
    fn installed_voices_lists_bin_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(installed_voices(dir.path()).is_empty(), "no folder yet");

        let voices = voices_dir(dir.path());
        fs::create_dir_all(voices.join("nested.bin")).unwrap();
        for name in ["bm_lewis.bin", "af_heart.bin", "notes.txt", "bad name.bin"] {
            fs::write(voices.join(name), b"").unwrap();
        }
        assert_eq!(installed_voices(dir.path()), vec!["af_heart", "bm_lewis"]);
    }

    /// The P2.2 spike's corpus, for judging the lexicons by ear and eye, and
    /// espeak-ng's readings when it is installed:
    /// `cargo test -- --ignored corpus --nocapture`.
    #[test]
    #[ignore = "prints a report for a human"]
    fn corpus_report() {
        const CORPUS: &[&str] = &[
            "The Bank of England held interest rates at 4.5% on Thursday, citing persistent inflation in the services sector.",
            "Ministers are expected to publish the long-delayed housing white paper before Parliament rises for the summer recess.",
            "Heavy rain caused flooding across parts of Yorkshire and Lancashire, with several train services cancelled overnight.",
            "The government said the new rules would come into force on 1 April 2027, subject to a final vote in the Commons.",
            "Shares in the retailer fell sharply after it warned that profits would be lower than analysts had forecast.",
            "Kubernetes schedules pods onto nodes, and the kubelet restarts any container whose liveness probe fails.",
            "Run the migration against PostgreSQL before deploying, otherwise the API will return a 500 error.",
            "The async runtime spawns a worker thread per core; call block_on only from a dedicated thread.",
            "Set the RUST_LOG environment variable to debug to see every HTTP request the client makes.",
            "Clone the repository from GitHub, then run npm install and start the dev server with Vite.",
            "We drove from Leicester to Loughborough, then on to Worcester and Gloucester before dark.",
            "Mr Featherstonehaugh and Mrs Cholmondeley were seated beside Siobhan and Niamh at dinner.",
            "The train from Edinburgh calls at Berwick-upon-Tweed, Alnmouth and Newcastle.",
            "She grew up in Keighley, studied at Magdalen College, Oxford, and now lives near Southwark.",
            "The tickets cost £45 each, or £120 for a family of four.",
            "Revenue rose to $3.2 billion in the third quarter, up 12 percent from a year earlier.",
            "The meeting is at 10:30 on Tuesday 14 October 2026, in room 2B.",
            "Prices have roughly doubled since the 1990s, and the average home now costs £285,000.",
            "In 1666 the Great Fire of London destroyed more than 13,000 houses.",
            "Version 2.0.1 fixes 37 bugs and adds support for 3rd-party plugins.",
            "I can't believe it's already October; we haven't even booked the hotel.",
        ];
        let (g2p, unknown) = recording_g2p(Language::EnglishGB);
        let espeak = espeak::find().map(|program| espeak::g2p(Dialect::British, program));
        let plain = G2P::new(Language::EnglishGB);
        let (mut oov, mut read_by_espeak) = (0, 0);
        for text in CORPUS {
            let (raw, _) = g2p.g2p(&reading(&g2p, text)).unwrap();
            let phonemes = to_kokoro_phonemes(&raw, Dialect::British);
            let words = std::mem::take(&mut *unknown.lock().unwrap());
            println!("{text}\n  {phonemes}\n  not in the lexicon: {words:?}");
            assert_eq!(token_ids(&phonemes).1, 0, "{text}");
            oov += words.len();
            let Some(espeak) = &espeak else { continue };
            let (ids, dropped) = text_to_ids(espeak, Dialect::British, text).unwrap();
            assert_eq!(dropped, 0, "{text}");
            assert!(has_phonemes(&ids));
            for word in &words {
                let (read, _) = espeak.g2p(word).unwrap();
                let (spelled, _) = plain.g2p(word).unwrap();
                println!("  espeak-ng: {word} -> {}", read.trim());
                read_by_espeak += usize::from(read != spelled);
            }
        }
        println!("{oov} words not in the lexicon, all spelled out without espeak-ng");
        match espeak {
            Some(_) => println!(
                "with espeak-ng: {read_by_espeak} read, {} spelled out",
                oov - read_by_espeak
            ),
            None => println!("espeak-ng is not installed"),
        }
    }

    #[test]
    fn a_session_stopped_while_waiting_loads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let files = ModelFiles::new(dir.path(), ModelVariant::Int8, "bm_lewis");
        let stopped = Arc::new(AtomicBool::new(true));
        let error = open(&files, &stopped).err().unwrap();
        assert!(error.starts_with("Stopped"), "{error}");
    }

    /// Prints how each dialect reads initialisms, with the audio's length:
    /// `MOUTHY_KOKORO_MODELS=<models dir> cargo test -- --ignored initialisms --nocapture`.
    #[test]
    #[ignore = "needs Kokoro model files"]
    fn local_voice_reads_initialisms() {
        let models = PathBuf::from(std::env::var("MOUTHY_KOKORO_MODELS").unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        for (voice, language) in [
            ("bm_lewis", Language::EnglishGB),
            ("af_alloy", Language::EnglishUS),
        ] {
            let g2p = G2P::new(language);
            let dialect = Dialect::for_voice(voice);
            let files = ModelFiles::new(&models, ModelVariant::Int8, voice);
            let mut loaded = open(&files, &stop).unwrap();
            let mut seconds = |text: &str| {
                let samples = loaded.synthesize(text, 1.0, true).unwrap();
                let seconds = samples.len() as f32 / SAMPLE_RATE as f32;
                println!(
                    "{voice} {text:32} {seconds:.2} s  {}",
                    read(&g2p, dialect, text)
                );
                seconds
            };
            for text in [
                "Install macOS.",
                "Node.js is fast.",
                "Call the API URL.",
                "NASA launched it.",
                "Update to iOS 17.",
                "Dr. Smith met Mr. Jones at 7.30pm.",
            ] {
                assert!(seconds(text) > 0.5, "{text}");
            }
            let titles = read(&g2p, dialect, "Dr. Smith met Mr. Jones at 7.30pm.");
            assert_eq!(titles.matches('.').count(), 1, "{titles}");
            // Letter names take longer than the one syllable of "earl".
            let spelled = seconds("URL");
            let word = seconds("earl");
            assert!(spelled > word + 0.1, "{spelled} s against {word} s");
        }
    }

    /// Needs the model files: `MOUTHY_KOKORO_MODELS=<models dir> cargo test -- --ignored local`.
    #[test]
    #[ignore = "needs Kokoro model files"]
    fn local_voice_speaks_and_stops() {
        let models = PathBuf::from(std::env::var("MOUTHY_KOKORO_MODELS").unwrap());
        let files = ModelFiles::new(&models, ModelVariant::Int8, "bm_lewis");
        let stop = Arc::new(AtomicBool::new(false));

        let mut loaded = open(&files, &stop).unwrap();
        let started = Instant::now();
        let samples = loaded
            .synthesize(
                "Hello from Mouthy, this is a short test sentence.",
                1.0,
                true,
            )
            .unwrap();
        let seconds = samples.len() as f32 / SAMPLE_RATE as f32;
        println!("{seconds:.2} s of audio in {:?}", started.elapsed());
        assert!((1.0..8.0).contains(&seconds), "{seconds} s of audio");
        assert!(samples.iter().any(|&s| s != 0));
        // Kokoro's ~0.4 s lead-in is trimmed.
        let sound = samples.iter().position(|&s| s.abs() > 100).unwrap();
        assert!(
            sound < SAMPLE_RATE as usize / 10,
            "{sound} samples before the sound"
        );
        assert!(loaded.synthesize("...", 1.0, true).unwrap().is_empty());

        // A long input, stopped mid-way, ends within a poll or two.
        let long = "The committee met again after the recess, and nobody spoke. ".repeat(8);
        let stopper = {
            let stop = stop.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(100));
                stop.store(true, Ordering::SeqCst);
                Instant::now()
            })
        };
        let result = loaded.synthesize(&long, 1.0, true);
        let stopped_at = stopper.join().unwrap();
        println!("stopped after {:?}", stopped_at.elapsed());
        assert!(result.is_err(), "a stopped inference must not finish");
        assert!(stopped_at.elapsed() < STOP_POLL * 5);
        drop(loaded);

        // The next session gets a fresh, unstopped model.
        let mut loaded = open(&files, &Arc::new(AtomicBool::new(false))).unwrap();
        assert!(!loaded
            .synthesize("Still working.", 1.0, true)
            .unwrap()
            .is_empty());
        drop(loaded);

        // Turning espeak-ng on swaps the G2P; unknown words take less audio
        // than their letter-by-letter spelling.
        let Some(program) = espeak::find() else {
            return;
        };
        let names = "Siobhan from Keighley runs Kubernetes.";
        let stop = Arc::new(AtomicBool::new(false));
        let spelled = open(&files, &stop)
            .unwrap()
            .synthesize(names, 1.0, true)
            .unwrap();
        let files = files.with_espeak(Some(program));
        let read = open(&files, &stop)
            .unwrap()
            .synthesize(names, 1.0, true)
            .unwrap();
        println!(
            "{} samples spelled, {} read by espeak-ng",
            spelled.len(),
            read.len()
        );
        assert!(!read.is_empty() && read.len() < spelled.len());
    }
}
