//! In-process Kokoro-82M: misaki-rs phonemes, ONNX Runtime inference.
//!
//! Files live under `<models>/kokoro-v1.0/`: `<variant>.onnx` (onnx-community
//! export) and `voices/<voice>.bin`. The loaded model stays in memory between
//! sessions; one session at a time holds it.

use super::text::expand_numbers;
use crate::settings::ModelVariant;
use log::{debug, error, info, trace, warn};
use misaki_rs::{Language, G2P};
use ort::environment::Environment;
use ort::logging::LogLevel;
use ort::session::builder::AutoDevicePolicy;
use ort::session::{RunOptions, Session};
use ort::value::{Tensor, TensorRef};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// The folder under the models directory that one Kokoro release lives in.
const KOKORO_DIR: &str = "kokoro-v1.0";
/// Kokoro's context is 512 ids, two of which are the pads around the text.
const MAX_TOKENS: usize = 510;
const STYLE_DIM: usize = 256;
/// How often a running inference checks whether its session was stopped.
const STOP_POLL: Duration = Duration::from_millis(20);

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
const TIES_COMMON: &[(&str, &str)] = &[
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

/// Which misaki lexicon a voice needs: `b*` voices are British English,
/// every other voice is read as American English.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dialect {
    British,
    American,
}

impl Dialect {
    fn for_voice(voice: &str) -> Self {
        if voice.starts_with('b') {
            Dialect::British
        } else {
            Dialect::American
        }
    }

    fn language(self) -> Language {
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
}

impl ModelFiles {
    pub fn new(models_dir: &Path, variant: ModelVariant, voice: &str) -> Self {
        let kokoro = models_dir.join(KOKORO_DIR);
        let file_name = match variant {
            ModelVariant::Int8 => "int8.onnx",
            ModelVariant::Fp16 => "fp16.onnx",
            ModelVariant::Fp32 => "fp32.onnx",
        };
        // The voice name is typed by the user; keep it inside the voices folder.
        let voice_path =
            is_voice_name(voice).then(|| voices_dir(models_dir).join(format!("{voice}.bin")));
        Self {
            model: kokoro.join(file_name),
            voice_id: voice.to_string(),
            voice: voice_path,
        }
    }

    /// The first file that is not there, described for the log.
    pub fn missing(&self) -> Option<String> {
        if !self.model.is_file() {
            return Some(self.model.display().to_string());
        }
        match &self.voice {
            None => Some(format!("a voice file for '{}'", self.voice_id)),
            Some(path) if !path.is_file() => Some(path.display().to_string()),
            Some(_) => None,
        }
    }
}

fn voices_dir(models_dir: &Path) -> PathBuf {
    models_dir.join(KOKORO_DIR).join("voices")
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
    match target() {
        Some(files) => ensure_loaded(&mut loaded, &files).map(drop),
        None => {
            if loaded.take().is_some() {
                info!("Unloaded the local voice");
            }
            Ok(())
        }
    }
}

/// Takes the model for one speech session, loading or switching files first.
/// Setting `stop` cuts the inference in progress short.
pub fn open(files: &ModelFiles, stop: &Arc<AtomicBool>) -> Result<Loaded, String> {
    let mut loaded = lock_loaded();
    let synth = ensure_loaded(&mut loaded, files)?;
    // Fresh per session: a late stop from the previous session must not end this one.
    let run_options =
        Arc::new(RunOptions::new().map_err(|e| format!("Cannot prepare ONNX Runtime: {e}"))?);
    synth.run_options = run_options.clone();
    let finished = Arc::new(AtomicBool::new(false));
    interrupt_on_stop(stop.clone(), finished.clone(), run_options);
    Ok(Loaded {
        synth: loaded,
        finished,
    })
}

/// The model, held by one speech session until dropped.
pub struct Loaded {
    synth: MutexGuard<'static, Option<LocalSynth>>,
    finished: Arc<AtomicBool>,
}

impl Loaded {
    pub fn synthesize(&mut self, text: &str, speed: f32) -> Result<Vec<i16>, String> {
        match self.synth.as_mut() {
            Some(synth) => synth.synthesize(text, speed),
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
fn ensure_loaded<'a>(
    loaded: &'a mut Option<LocalSynth>,
    files: &ModelFiles,
) -> Result<&'a mut LocalSynth, String> {
    let model = FileStamp::of(&files.model, "Kokoro model")?;
    let voice_path = files
        .voice
        .as_deref()
        .ok_or_else(|| format!("'{}' is not a voice name", files.voice_id))?;
    let voice = FileStamp::of(voice_path, "voice file")?;
    let dialect = Dialect::for_voice(&files.voice_id);

    match loaded {
        // Each part is swapped only once its replacement has loaded, so a
        // failure leaves the stamps matching what is held.
        Some(synth) => synth.refresh(model, voice, dialect)?,
        None => *loaded = Some(LocalSynth::load(model, voice, dialect)?),
    }
    loaded
        .as_mut()
        .ok_or_else(|| "The local voice is not loaded".into())
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

struct LocalSynth {
    session: Session,
    model: FileStamp,
    voice: Voice,
    g2p: G2P,
    dialect: Dialect,
    run_options: Arc<RunOptions>,
}

impl LocalSynth {
    fn load(model: FileStamp, voice: FileStamp, dialect: Dialect) -> Result<Self, String> {
        let session = load_session(&model.path)?;
        let voice = Voice::load(voice)?;
        let g2p = build_g2p(dialect);
        let run_options =
            Arc::new(RunOptions::new().map_err(|e| format!("Cannot prepare ONNX Runtime: {e}"))?);
        Ok(Self {
            session,
            model,
            voice,
            g2p,
            dialect,
            run_options,
        })
    }

    fn refresh(
        &mut self,
        model: FileStamp,
        voice: FileStamp,
        dialect: Dialect,
    ) -> Result<(), String> {
        if self.model != model {
            self.session = load_session(&model.path)?;
            self.model = model;
        }
        if self.voice.stamp != voice {
            self.voice = Voice::load(voice)?;
        }
        if self.dialect != dialect {
            self.g2p = build_g2p(dialect);
            self.dialect = dialect;
        }
        Ok(())
    }

    /// Speaks `text` as 24 kHz mono PCM. Text with nothing to pronounce gives
    /// no samples. Input longer than Kokoro's context is cut into phrases.
    fn synthesize(&mut self, text: &str, speed: f32) -> Result<Vec<i16>, String> {
        let (raw, _) = self
            .g2p
            .g2p(&expand_numbers(text))
            .map_err(|e| format!("Cannot phonemise the text: {e}"))?;
        let (ids, dropped) = token_ids(&to_kokoro_phonemes(&raw, self.dialect));
        if dropped > 0 {
            debug!("Dropped {dropped} symbols Kokoro has no token for");
        }
        let mut audio = Vec::new();
        for piece in split_tokens(&ids).into_iter().filter(|p| has_phonemes(p)) {
            let waveform = run(
                &mut self.session,
                piece,
                self.voice.style(piece.len()),
                speed,
                &self.run_options,
            )
            .map_err(|e| format!("Local synthesis failed: {e}"))?;
            audio.extend(waveform.iter().copied().map(to_pcm));
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
    if !session
        .inputs()
        .iter()
        .any(|input| input.name() == "input_ids")
    {
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
        .commit_from_file(path)
}

fn build_g2p(dialect: Dialect) -> G2P {
    let started = Instant::now();
    let g2p = G2P::new(dialect.language());
    info!(
        "Loaded the {dialect:?} English lexicon in {} ms",
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
        match VOCAB.iter().find(|(symbol, _)| *symbol == c) {
            Some((_, id)) => ids.push(*id),
            None => dropped += 1,
        }
    }
    (ids, dropped)
}

/// Ids up to the space are punctuation, which alone says nothing.
fn has_phonemes(ids: &[i64]) -> bool {
    ids.iter().any(|&id| id > SPACE_ID)
}

/// Cuts ids into runs Kokoro accepts: after the last pause that fits, else at
/// the last space, else at the limit. Nothing is dropped but the spaces at a
/// cut.
fn split_tokens(ids: &[i64]) -> Vec<&[i64]> {
    let mut pieces = Vec::new();
    let mut rest = trim_spaces(ids);
    while rest.len() > MAX_TOKENS {
        let window = &rest[..MAX_TOKENS];
        let cut_after = |found: Option<usize>| {
            found
                .map(|i| i + 1)
                .filter(|&cut| has_phonemes(&window[..cut]))
        };
        let cut = cut_after(window.iter().rposition(|id| PAUSE_IDS.contains(id)))
            .or_else(|| cut_after(window.iter().rposition(|&id| id == SPACE_ID)))
            .unwrap_or(MAX_TOKENS);
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
    use misaki_rs::lexicon::PhonemeEntry;
    use std::collections::{BTreeSet, HashMap};

    fn kokoro(raw: &str, dialect: Dialect) -> String {
        to_kokoro_phonemes(raw, dialect)
    }

    /// One test per tie the mapping replaces.
    macro_rules! tie_test {
        ($name:ident, $dialect:expr, $raw:expr, $expected:expr) => {
            #[test]
            fn $name() {
                assert_eq!(kokoro($raw, $dialect), $expected);
            }
        };
    }

    tie_test!(tie_ei_is_a, Dialect::British, "ðe\u{200d}ɪ", "ðA");
    tie_test!(tie_ai_is_i, Dialect::British, "fa\u{200d}ɪv", "fIv");
    tie_test!(tie_au_is_w, Dialect::British, "na\u{200d}ʊ", "nW");
    tie_test!(tie_oi_is_y, Dialect::British, "bɔ\u{200d}ɪ", "bY");
    tie_test!(
        tie_dzh_is_affricate,
        Dialect::British,
        "d\u{200d}ʒˈʌmp",
        "ʤˈʌmp"
    );
    tie_test!(
        tie_tsh_is_affricate,
        Dialect::British,
        "ˈiːt\u{200d}ʃ",
        "ˈiːʧ"
    );
    tie_test!(tie_syllabic_l, Dialect::British, "bˈɒtə\u{200d}l", "bˈɒtᵊl");
    tie_test!(lateral_fricative_is_l, Dialect::British, "ɬˈan", "lˈan");
    tie_test!(gb_tie_ou_is_q, Dialect::British, "ɡˈə\u{200d}ʊ", "ɡˈQ");
    tie_test!(
        gb_tie_ia_is_centring,
        Dialect::British,
        "nˈi\u{200d}ə",
        "nˈɪə"
    );
    tie_test!(gb_tie_ea_is_long_e, Dialect::British, "ðe\u{200d}ə", "ðɛː");
    tie_test!(us_tie_ou_is_o, Dialect::American, "ɡˈo\u{200d}ʊ", "ɡˈO");
    tie_test!(
        us_tie_long_r_is_r,
        Dialect::American,
        "bˈɜː\u{200d}ɹd",
        "bˈɜɹd"
    );
    tie_test!(us_drops_length, Dialect::American, "fˈɑːðɚ", "fˈɑðɚ");
    tie_test!(us_syllabic_n, Dialect::American, "bˈʌʔn\u{329}", "bˈʌʔᵊn");
    tie_test!(
        triphthong_keeps_no_tie,
        Dialect::British,
        "fa\u{200d}ɪ\u{200d}ə",
        "fIə"
    );

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

    #[test]
    fn voice_prefix_picks_the_dialect() {
        assert_eq!(Dialect::for_voice("bm_lewis"), Dialect::British);
        assert_eq!(Dialect::for_voice("bf_emma"), Dialect::British);
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
        let ids = vec![43; MAX_TOKENS];
        assert_eq!(split_tokens(&ids), vec![&ids[..]]);
        assert!(split_tokens(&[]).is_empty());
        assert!(split_tokens(&[SPACE_ID, SPACE_ID]).is_empty());
    }

    #[test]
    fn long_input_is_cut_after_the_last_pause_that_fits() {
        // "aaa, aaa, …" well past the limit.
        let phrase = [43, 43, 43, 3, SPACE_ID];
        let ids: Vec<i64> = phrase.iter().copied().cycle().take(1200).collect();
        let pieces = split_tokens(&ids);
        assert!(pieces.len() >= 3);
        for piece in &pieces {
            assert!(piece.len() <= MAX_TOKENS);
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
            .take(600)
            .collect();
        let pieces = split_tokens(&words);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].last(), Some(&43));
        assert!(pieces[0].len() <= MAX_TOKENS);

        let solid = vec![43; 1100];
        let pieces = split_tokens(&solid);
        assert_eq!(
            pieces.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![MAX_TOKENS, MAX_TOKENS, 80]
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
        ids.extend(std::iter::repeat_n(43, 600));
        let pieces = split_tokens(&ids);
        assert!(pieces.iter().all(|p| has_phonemes(p)), "{pieces:?}");
    }

    #[test]
    fn pcm_conversion_clamps() {
        assert_eq!(to_pcm(0.0), 0);
        assert_eq!(to_pcm(1.0), i16::MAX);
        assert_eq!(to_pcm(2.0), i16::MAX);
        assert_eq!(to_pcm(-2.0), -i16::MAX);
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
            ModelFiles::new(models, ModelVariant::Fp16, "x").model,
            Path::new("/models/kokoro-v1.0/fp16.onnx")
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

        let voices = voices_dir(dir.path());
        fs::create_dir_all(&voices).unwrap();
        fs::write(dir.path().join(KOKORO_DIR).join("int8.onnx"), b"model").unwrap();
        assert!(files.missing().unwrap().ends_with("bm_lewis.bin"));

        fs::write(voices.join("bm_lewis.bin"), b"voice").unwrap();
        assert_eq!(files.missing(), None);
        assert!(ModelFiles::new(dir.path(), ModelVariant::Int8, "../x")
            .missing()
            .is_some());
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
            .synthesize("Hello from Mouthy, this is a short test sentence.", 1.0)
            .unwrap();
        let seconds = samples.len() as f32 / crate::tts::SAMPLE_RATE as f32;
        println!("{seconds:.2} s of audio in {:?}", started.elapsed());
        assert!((1.0..8.0).contains(&seconds), "{seconds} s of audio");
        assert!(samples.iter().any(|&s| s != 0));
        assert!(loaded.synthesize("...", 1.0).unwrap().is_empty());

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
        let result = loaded.synthesize(&long, 1.0);
        let stopped_at = stopper.join().unwrap();
        println!("stopped after {:?}", stopped_at.elapsed());
        assert!(result.is_err(), "a stopped inference must not finish");
        assert!(stopped_at.elapsed() < STOP_POLL * 5);
        drop(loaded);

        // The next session gets a fresh, unstopped model.
        let mut loaded = open(&files, &Arc::new(AtomicBool::new(false))).unwrap();
        assert!(!loaded.synthesize("Still working.", 1.0).unwrap().is_empty());
    }
}
