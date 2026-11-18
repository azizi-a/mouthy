//! In-process Kokoro-82M: misaki-rs or espeak-ng phonemes, ONNX Runtime
//! inference.
//!
//! Files live under `<models>/kokoro-v1.0/`: `<variant>.onnx` (onnx-community
//! export) and `voices/<voice>.bin`. The loaded model stays in memory between
//! sessions; one session at a time holds it.

use super::catalog::VoiceLanguage;
use super::english_words::{self, Names};
use super::espeak;
use super::ja;
use super::ja_dictionary::{self, Tagger};
use super::text::{self, expand_abbreviations, expand_numbers, joins_numbers, spell_out, Marks};
use super::voice::{is_voice_name, VoiceSpec};
use super::zh;
use super::SAMPLE_RATE;
use crate::settings::ModelVariant;
use log::{debug, error, info, trace, warn};
use misaki_rs::lexicon::Lexicon;
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
use unicode_normalization::UnicodeNormalization;

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
const MAX_CHUNK_CHARS: usize = 450;
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

/// The language `text` is read in by a voice reading `language`: a Japanese
/// or Chinese voice reads text with none of its language in it as an American
/// voice does, so English in such a voice is unchanged.
fn language_for(language: VoiceLanguage, text: &str) -> VoiceLanguage {
    match language {
        VoiceLanguage::Japanese if !ja::has_japanese(text) => VoiceLanguage::AmericanEnglish,
        VoiceLanguage::Mandarin if !zh::has_chinese(text) => VoiceLanguage::AmericanEnglish,
        language => language,
    }
}

/// The notice for a language espeak-ng reads when it is not installed. Short
/// enough for the overlay; Settings says how to install espeak-ng.
fn espeak_missing(language: VoiceLanguage) -> String {
    let name = match language {
        VoiceLanguage::Spanish => "Spanish",
        VoiceLanguage::French => "French",
        VoiceLanguage::Italian => "Italian",
        VoiceLanguage::BrazilianPortuguese => "Portuguese",
        VoiceLanguage::Hindi => "Hindi",
        _ => "These",
    };
    format!("{name} voices need espeak-ng, or Reading Language set to English.")
}

/// Which misaki lexicon English text is read with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Dialect {
    British,
    American,
}

impl Dialect {
    /// British English reads with the British lexicon, any other language
    /// with the American one.
    fn of(language: VoiceLanguage) -> Self {
        match language {
            VoiceLanguage::BritishEnglish => Dialect::British,
            _ => Dialect::American,
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

/// The files one local voice or blend needs.
#[derive(Debug)]
pub struct ModelFiles {
    model: PathBuf,
    /// The voice setting as stored.
    voice_id: String,
    /// `None` when the setting is not a voice or blend.
    voices: Option<Vec<VoiceFile>>,
    /// The language the text is read in.
    language: VoiceLanguage,
    /// Whether espeak-ng reads the English words the lexicon lacks.
    english_fallback: bool,
    /// The espeak-ng to run, if any.
    espeak: Option<PathBuf>,
    /// The Japanese dictionary's folder, when it is installed.
    dictionary: Option<PathBuf>,
}

/// One voice of a blend, or the only one.
#[derive(Debug)]
struct VoiceFile {
    id: String,
    path: PathBuf,
    weight: f32,
}

impl ModelFiles {
    pub fn new(models_dir: &Path, variant: ModelVariant, voice: &str) -> Self {
        let voices: Option<Vec<VoiceFile>> = VoiceSpec::parse(voice).ok().and_then(|spec| {
            spec.parts()
                .iter()
                .map(|(id, weight)| {
                    Some(VoiceFile {
                        id: id.clone(),
                        path: voice_path(models_dir, id)?,
                        weight: *weight,
                    })
                })
                .collect()
        });
        // A blend reads in its first voice's language, as Kokoro-FastAPI does.
        // A custom voice's unknown prefix reads as American English.
        let language = voices
            .as_ref()
            .and_then(|voices| voices.first())
            .and_then(|first| VoiceLanguage::of(&first.id))
            .unwrap_or(VoiceLanguage::AmericanEnglish);
        Self {
            model: model_path(models_dir, variant),
            voice_id: voice.to_string(),
            voices,
            language,
            english_fallback: false,
            espeak: None,
            dictionary: Some(ja_dictionary::dir(models_dir))
                .filter(|dir| ja_dictionary::installed(dir)),
        }
    }

    /// Reads the text in `language` rather than the voice's.
    pub fn reading_as(self, language: VoiceLanguage) -> Self {
        Self { language, ..self }
    }

    pub fn language(&self) -> VoiceLanguage {
        self.language
    }

    pub fn with_english_fallback(self, english_fallback: bool) -> Self {
        Self {
            english_fallback,
            ..self
        }
    }

    pub fn with_espeak(self, espeak: Option<PathBuf>) -> Self {
        Self { espeak, ..self }
    }

    /// Whether espeak-ng reads this language's text, whatever the English
    /// fallback says.
    pub fn needs_espeak(&self) -> bool {
        espeak::voice(self.language).is_some()
    }

    /// What reading this language needs that is not installed, as a notice.
    /// English needs nothing beyond the model files.
    pub fn unmet_requirement(&self) -> Option<String> {
        (self.needs_espeak() && self.espeak.is_none()).then(|| espeak_missing(self.language))
    }

    /// Where `text` is cut into sentences and clauses.
    pub fn marks(&self, text: &str) -> &'static Marks {
        match language_for(self.language, text) {
            VoiceLanguage::Japanese | VoiceLanguage::Mandarin => &Marks::CJK,
            VoiceLanguage::Hindi => &Marks::HINDI,
            _ => &Marks::DEFAULT,
        }
    }

    /// The longest sentence of `text` one inference gets before it is cut.
    pub fn max_chunk_chars(&self, text: &str) -> usize {
        match language_for(self.language, text) {
            VoiceLanguage::Japanese => ja::MAX_CHUNK_CHARS,
            VoiceLanguage::Mandarin => zh::MAX_CHUNK_CHARS,
            _ => MAX_CHUNK_CHARS,
        }
    }

    /// The notice to show instead of reading `text`, when too little of it
    /// can be read: Japanese mostly in kanji, with no dictionary to read them.
    pub fn refuses(&self, text: &str) -> Option<String> {
        match self.language {
            VoiceLanguage::Japanese if self.dictionary.is_none() => {
                ja::refusal(text).map(String::from)
            }
            _ => None,
        }
    }

    /// The first file that is not there, described for the log.
    pub fn missing(&self) -> Option<String> {
        if self.model_missing() {
            return Some(self.model.display().to_string());
        }
        match &self.voices {
            Some(voices) => voices
                .iter()
                .find(|voice| !voice.path.is_file())
                .map(|voice| voice.path.display().to_string()),
            None => Some(format!("a voice file for '{}'", self.voice_id)),
        }
    }

    pub fn model_missing(&self) -> bool {
        !self.model.is_file()
    }

    /// The voices with no file, each once; the whole setting when it is not
    /// a voice or blend.
    pub fn missing_voices(&self) -> Vec<String> {
        let Some(voices) = &self.voices else {
            return vec![self.voice_id.clone()];
        };
        let mut missing: Vec<String> = Vec::new();
        for voice in voices {
            if !voice.path.is_file() && !missing.contains(&voice.id) {
                missing.push(voice.id.clone());
            }
        }
        missing
    }

    fn reading(&self) -> Reading {
        Reading {
            language: self.language,
            english_fallback: self.english_fallback,
            espeak: self.espeak.clone(),
        }
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

/// Voice ids with a `.bin` file in the voices folder, sorted. A name with `-`
/// is left out: the voice setting reads `-` as subtraction and refuses it.
pub fn installed_voices(models_dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(voices_dir(models_dir)) else {
        return Vec::new();
    };
    let mut voices: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "bin") && path.is_file())
        .filter_map(|path| path.file_stem()?.to_str().map(str::to_string))
        .filter(|stem| is_voice_name(stem) && !stem.contains('-'))
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
    let voices = files
        .voices
        .as_deref()
        .ok_or_else(|| format!("'{}' is not a voice or blend", files.voice_id))?;
    let voice = voice_stamps(voices)?;
    let reading = files.reading();
    // A dictionary deleted since `files` was made counts as absent.
    let dictionary = files.dictionary.as_deref().and_then(|dir| {
        let stamp = FileStamp::of(&ja_dictionary::stamp_path(dir), "Japanese dictionary").ok()?;
        Some((dir.to_path_buf(), stamp))
    });

    match loaded {
        // Each part is swapped only once its replacement has loaded, so a
        // failure leaves the stamps matching what is held.
        Some(synth) => synth.refresh(model, voice, reading, dictionary),
        None => {
            *loaded = Some(LocalSynth::load(model, voice, reading, dictionary)?);
            Ok(())
        }
    }
}

/// Each voice file as it is now, with its weight, in path order so the same
/// blend written in another order matches what is loaded.
fn voice_stamps(voices: &[VoiceFile]) -> Result<Vec<(FileStamp, f32)>, String> {
    let mut stamps = voices
        .iter()
        .map(|voice| Ok((FileStamp::of(&voice.path, "voice file")?, voice.weight)))
        .collect::<Result<Vec<_>, String>>()?;
    stamps.sort_by(|a, b| a.0.path.cmp(&b.0.path).then(a.1.total_cmp(&b.1)));
    Ok(stamps)
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

/// How text is read: in which language, with which espeak-ng, and whether it
/// reads the English words the lexicon lacks.
#[derive(Debug, PartialEq, Eq)]
struct Reading {
    language: VoiceLanguage,
    english_fallback: bool,
    espeak: Option<PathBuf>,
}

impl Reading {
    /// The espeak-ng that reads English words the lexicon lacks: none unless
    /// the fallback is on.
    fn english_espeak(&self) -> Option<&PathBuf> {
        self.espeak.as_ref().filter(|_| self.english_fallback)
    }
}

/// How text becomes Kokoro's symbols in the language being read.
enum Phonemizer {
    /// English, from misaki's lexicons; `names` says who reads the
    /// capitalised words they lack.
    Misaki { g2p: Box<G2P>, names: Names },
    /// A whole sentence at a time from espeak-ng.
    Espeak(espeak::Sentences),
}

impl Phonemizer {
    fn new(reading: &Reading) -> Result<Self, String> {
        let Some(voice) = espeak::voice(reading.language) else {
            if reading.language == VoiceLanguage::Mandarin {
                // Here, off the main thread, so the first sentence does not wait.
                zh::load();
            }
            let names = match reading.english_espeak() {
                Some(_) => Names::Espeak,
                None => Names::Here,
            };
            let g2p = Box::new(build_g2p(reading));
            return Ok(Self::Misaki { g2p, names });
        };
        let program = reading
            .espeak
            .clone()
            .ok_or_else(|| espeak_missing(reading.language))?;
        info!(
            "Reading {:?} with espeak-ng at {}",
            reading.language,
            program.display()
        );
        Ok(Self::Espeak(espeak::Sentences::new(program, voice)))
    }

    /// Whether a pause mark can sit inside a word: espeak-ng writes Hindi
    /// `ड़` and `ढ़` as `r.`.
    fn has_marks_in_words(&self) -> bool {
        matches!(self, Self::Espeak(_))
    }

    /// Kokoro ids for `text` in `language`, and how many symbols had no id.
    fn ids(
        &self,
        tagger: Option<&Tagger>,
        language: VoiceLanguage,
        text: &str,
    ) -> Result<(Vec<i64>, usize), String> {
        match self {
            Self::Misaki { g2p, names } => text_to_ids(g2p, *names, tagger, language, text),
            Self::Espeak(sentences) => {
                let (phonemes, unread) = sentences.read(&plain_text(text, language))?;
                let (ids, dropped) = token_ids(&phonemes);
                Ok((ids, unread + dropped))
            }
        }
    }
}

/// The Japanese dictionary as found on disk, and its tagger once Japanese
/// text needed it.
struct JapaneseDictionary {
    found: Option<(PathBuf, FileStamp)>,
    tagger: Option<Result<Tagger, String>>,
}

impl JapaneseDictionary {
    fn new(found: Option<(PathBuf, FileStamp)>) -> Self {
        Self {
            found,
            tagger: None,
        }
    }

    /// The tagger, loaded the first time it is asked for; `None` when there is
    /// no dictionary.
    fn tagger(&mut self) -> Result<Option<&Tagger>, String> {
        let Some((dir, _)) = &self.found else {
            return Ok(None);
        };
        match self.tagger.get_or_insert_with(|| ja_dictionary::load(dir)) {
            Ok(tagger) => Ok(Some(tagger)),
            Err(e) => Err(e.clone()),
        }
    }
}

struct LocalSynth {
    session: Session,
    model: FileStamp,
    voice: Voice,
    phonemizer: Phonemizer,
    reading: Reading,
    dictionary: JapaneseDictionary,
}

impl LocalSynth {
    fn load(
        model: FileStamp,
        voice: Vec<(FileStamp, f32)>,
        reading: Reading,
        dictionary: Option<(PathBuf, FileStamp)>,
    ) -> Result<Self, String> {
        let session = load_session(&model.path)?;
        let voice = Voice::load(voice)?;
        let phonemizer = Phonemizer::new(&reading)?;
        Ok(Self {
            session,
            model,
            voice,
            phonemizer,
            reading,
            dictionary: JapaneseDictionary::new(dictionary),
        })
    }

    fn refresh(
        &mut self,
        model: FileStamp,
        voice: Vec<(FileStamp, f32)>,
        reading: Reading,
        dictionary: Option<(PathBuf, FileStamp)>,
    ) -> Result<(), String> {
        if self.model != model {
            self.session = load_session(&model.path)?;
            self.model = model;
        }
        self.voice.refresh(voice)?;
        if self.reading != reading {
            self.phonemizer = Phonemizer::new(&reading)?;
            self.reading = reading;
        }
        // A dictionary downloaded, deleted or replaced loads afresh when next needed.
        if self.dictionary.found != dictionary {
            self.dictionary = JapaneseDictionary::new(dictionary);
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
        let tagger = match language_for(self.reading.language, text) {
            VoiceLanguage::Japanese => self.dictionary.tagger()?,
            _ => None,
        };
        let (ids, dropped) = self.phonemizer.ids(tagger, self.reading.language, text)?;
        if dropped > 0 {
            debug!("Dropped {dropped} symbols Kokoro has no token for");
        }
        let pieces: Vec<_> = split_tokens(&ids, PIECE_TOKENS, self.phonemizer.has_marks_in_words())
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
    let dialect = Dialect::of(reading.language);
    let (g2p, unknown_words) = match reading.english_espeak() {
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

/// A voice's style table, one 256-wide row per input length: one voice file,
/// or several blended.
struct Voice {
    /// The files as loaded, with their weights, in path order.
    parts: Vec<(FileStamp, f32)>,
    rows: Vec<f32>,
}

impl Voice {
    fn load(parts: Vec<(FileStamp, f32)>) -> Result<Self, String> {
        let tables = parts
            .iter()
            .map(|(stamp, _)| read_style_rows(&stamp.path))
            .collect::<Result<Vec<_>, _>>()?;
        let weighted: Vec<_> = tables
            .iter()
            .zip(&parts)
            .map(|(rows, &(_, weight))| (rows.as_slice(), weight))
            .collect();
        let rows = blend(&weighted)?;
        Ok(Self { parts, rows })
    }

    /// Reloads when a file or a weight differs from what is loaded. A failure
    /// keeps the loaded table and its parts.
    fn refresh(&mut self, parts: Vec<(FileStamp, f32)>) -> Result<(), String> {
        if self.parts != parts {
            *self = Self::load(parts)?;
        }
        Ok(())
    }

    /// The row for `tokens` ids (pads excluded), clamped to the last row.
    fn style(&self, tokens: usize) -> &[f32] {
        style_row(&self.rows, tokens)
    }
}

/// One voice file's rows: raw little-endian f32.
fn read_style_rows(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = fs::read(path)
        .map_err(|e| format!("Cannot read the voice file {}: {e}", path.display()))?;
    parse_style_rows(&bytes).ok_or_else(|| format!("{} is not a Kokoro voice file", path.display()))
}

/// Σ wᵢ/Σw · tableᵢ element by element, as Kokoro-FastAPI blends: the first
/// table scaled, then each other added scaled. Weights are above zero.
fn blend(tables: &[(&[f32], f32)]) -> Result<Vec<f32>, String> {
    let Some((&(first, first_weight), rest)) = tables.split_first() else {
        return Err("A blend needs at least one voice".into());
    };
    if rest.iter().any(|(rows, _)| rows.len() != first.len()) {
        return Err("The blended voice files differ in size, so they cannot be mixed".into());
    }
    let total: f64 = tables.iter().map(|&(_, weight)| f64::from(weight)).sum();
    let share = |weight: f32| (f64::from(weight) / total) as f32;
    let first_share = share(first_weight);
    let mut blended: Vec<f32> = first.iter().map(|value| value * first_share).collect();
    for &(rows, weight) in rest {
        let share = share(weight);
        for (out, value) in blended.iter_mut().zip(rows) {
            *out += value * share;
        }
    }
    Ok(blended)
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
fn text_to_ids(
    g2p: &G2P,
    names: Names,
    tagger: Option<&Tagger>,
    language: VoiceLanguage,
    text: &str,
) -> Result<(Vec<i64>, usize), String> {
    Ok(token_ids(&text_to_phonemes(
        g2p, names, tagger, language, text,
    )?))
}

/// Kokoro's symbols for `text`, before they become ids. Japanese is read with
/// `tagger` when there is one, and from its kana alone when not.
fn text_to_phonemes(
    g2p: &G2P,
    names: Names,
    tagger: Option<&Tagger>,
    language: VoiceLanguage,
    text: &str,
) -> Result<String, String> {
    let language = language_for(language, text);
    if language == VoiceLanguage::Japanese {
        // English inside Japanese text is read as American English.
        let english =
            |latin: &str| text_to_phonemes(g2p, names, None, VoiceLanguage::AmericanEnglish, latin);
        let (phonemes, skipped) = ja::phonemes(&plain_text(text, language), tagger, english)?;
        if skipped > 0 {
            debug!("Skipped {skipped} kanji with no reading");
        }
        return Ok(phonemes);
    }
    if language == VoiceLanguage::Mandarin {
        let english =
            |latin: &str| text_to_phonemes(g2p, names, None, VoiceLanguage::AmericanEnglish, latin);
        let (phonemes, skipped) = zh::phonemes(&plain_text(text, language), english)?;
        if skipped > 0 {
            debug!("Skipped {skipped} Chinese characters with no reading");
        }
        return Ok(phonemes);
    }
    let (raw, _) = g2p
        .g2p(&reading(g2p, names, text, language))
        .map_err(|e| format!("Cannot phonemise the text: {e}"))?;
    Ok(to_kokoro_phonemes(&raw, Dialect::of(language)))
}

/// The symbols the local voice for `files` reads each of `texts` as, without
/// espeak-ng or the Japanese dictionary, so every machine gives the same.
#[cfg(test)]
pub(crate) fn phonemes_without_espeak(files: &ModelFiles, texts: &[String]) -> Vec<String> {
    phonemes_with(files, texts, None)
}

/// The same with the Japanese dictionary, when the test run has it.
#[cfg(test)]
pub(crate) fn phonemes_with_dictionary(
    files: &ModelFiles,
    texts: &[String],
) -> Option<Vec<String>> {
    let tagger = ja_dictionary::tagger_for_test()?;
    Some(phonemes_with(files, texts, Some(tagger)))
}

#[cfg(test)]
fn phonemes_with(files: &ModelFiles, texts: &[String], tagger: Option<&Tagger>) -> Vec<String> {
    let reading = Reading {
        espeak: None,
        ..files.reading()
    };
    let g2p = build_g2p(&reading);
    texts
        .iter()
        .map(|text| text_to_phonemes(&g2p, Names::Here, tagger, reading.language, text).unwrap())
        .collect()
}

/// `text` as misaki-rs should get it: plain characters and, in English,
/// abbreviations and numbers in words, initialisms spelled, and words its
/// lexicons lack read from the words they are made of.
fn reading(g2p: &G2P, names: Names, text: &str, language: VoiceLanguage) -> String {
    let plain = plain_text(text, language);
    if !is_english(language) {
        return plain;
    }
    let words = expand_numbers(&expand_abbreviations(&plain));
    english_words::read_unknown_words(&spell_out(&words, &g2p.lexicon), &g2p.lexicon, names)
}

fn is_english(language: VoiceLanguage) -> bool {
    matches!(
        language,
        VoiceLanguage::AmericanEnglish | VoiceLanguage::BritishEnglish
    )
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
        let Some(mut phonemes) = english_words::initialism(letters, self) else {
            return letters
                .chars()
                .map(String::from)
                .collect::<Vec<_>>()
                .join(" ");
        };
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
/// reads a dash as a plain space. Only English loses its accents: other
/// languages need their letters as written, and espeak-ng misreads a letter
/// followed by a separate accent mark ("e" + U+0301), as macOS often writes it.
fn plain_text(text: &str, language: VoiceLanguage) -> String {
    let fold = is_english(language);
    let composed: String;
    let text = if espeak::voice(language).is_some() {
        composed = text.nfc().collect();
        &composed
    } else {
        text
    };
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        match c {
            // misaki-rs takes `[text](…)` as a pronunciation, and an earlier
            // `[` would swallow the words before a `spell_out` override.
            '[' => out.push('('),
            ']' => out.push(')'),
            '’' | '‘' | 'ʼ' => out.push('\''),
            // misaki keeps these in Chinese, and Kokoro has tokens for them.
            '“' | '”' if language == VoiceLanguage::Mandarin => out.push(c),
            '“' | '”' | '„' => out.push('"'),
            // Minus sign.
            '\u{2212}' => out.push('-'),
            // Accents typed as a separate mark after the letter ("e" + U+0301).
            '\u{300}'..='\u{36f}' if fold && out.ends_with(|c: char| c.is_ascii_alphabetic()) => {}
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
            c if c.is_ascii() || !fold => out.push(c),
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
/// cut. With `marks_in_words`, only a pause before a space or the end counts.
fn split_tokens(ids: &[i64], max: usize, marks_in_words: bool) -> Vec<&[i64]> {
    let mut pieces = Vec::new();
    let mut rest = trim_spaces(ids);
    while rest.len() > max {
        let window = &rest[..max];
        let cut_after = |found: Option<usize>| {
            found
                .map(|i| i + 1)
                .filter(|&cut| has_phonemes(&window[..cut]))
        };
        let ends_word = |i: usize| !marks_in_words || rest.get(i + 1) == Some(&SPACE_ID);
        let pause = (0..max)
            .rev()
            .find(|&i| PAUSE_IDS.contains(&rest[i]) && ends_word(i));
        let cut = cut_after(pause)
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
    use misaki_rs::lexicon::PhonemeEntry;
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

    fn plain_english(text: &str) -> String {
        plain_text(text, VoiceLanguage::BritishEnglish)
    }

    #[test]
    fn only_english_loses_its_accents() {
        let text = "Él dijo “sí” — mañana, cafe\u{301} y ça va.";
        assert_eq!(
            plain_text(text, VoiceLanguage::Spanish),
            "Él dijo \"sí\", mañana, café y ça va."
        );
        assert_eq!(
            plain_text(text, VoiceLanguage::AmericanEnglish),
            "El dijo \"si\", manana, cafe y ca va."
        );
    }

    #[test]
    fn only_english_has_numbers_abbreviations_and_initialisms_rewritten() {
        let g2p = G2P::new(Language::EnglishUS);
        let text = "Dr. Smith paid $5 for the API.";
        assert_eq!(
            reading(&g2p, Names::Here, text, VoiceLanguage::Spanish),
            text
        );
        let english = reading(&g2p, Names::Here, text, VoiceLanguage::AmericanEnglish);
        assert!(
            english.starts_with("Doctor Smith paid five dollars for the [API]("),
            "{english}"
        );
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
            let (read, _) = g2p.g2p(&plain_english(typed)).unwrap();
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
            assert_eq!(plain_english(typed), plain);
            let (read, _) = g2p.g2p(&plain_english(typed)).unwrap();
            let (expected, _) = g2p.g2p(plain).unwrap();
            assert_eq!(read, expected, "{typed}");
        }
        assert!(unknown.lock().unwrap().is_empty(), "{unknown:?}");
        // Not after a plain letter: left for misaki-rs.
        assert_eq!(plain_english("日\u{301}"), "日\u{301}");
    }

    /// Kokoro symbols for `text` as the local voice reads it.
    fn read(g2p: &G2P, language: VoiceLanguage, text: &str) -> String {
        text_to_phonemes(g2p, Names::Here, None, language, text).unwrap()
    }

    #[test]
    fn initialisms_read_as_letter_names() {
        let gb = G2P::new(Language::EnglishGB);
        let read_gb = |text| read(&gb, VoiceLanguage::BritishEnglish, text);
        assert_eq!(read_gb("Your ID"), "jˌɔː ˌIdˈiː");
        assert_eq!(read_gb("the API URL"), "ði ˌApˌiːˈI jˌuːˌɑːˈɛl");
        // Spaced letters would get the article's "ɐ" for this "A".
        assert_eq!(read_gb("I use API keys"), "ˌI jˈuːs ˌApˌiːˈI kˈiːz");
        assert_eq!(read_gb("IDs"), "ˌIdˈiːz");
        assert_eq!(read_gb("The U.S.'s debt"), "ði jˌuːˈɛsɪz dˈɛt");
        assert_eq!(read_gb("the US government"), "ði jˌuːˈɛs ɡˈʌvənmənt");
        assert_eq!(read_gb("at 7 a.m."), "at sˈɛvən ˌAˈɛm.");
        let us = G2P::new(Language::EnglishUS);
        let read_us = |text| read(&us, VoiceLanguage::AmericanEnglish, text);
        assert_eq!(read_us("Your ID"), "jˌʊɹ ˌIdˈi");
        assert_eq!(read_us("macOS"), "mˈæk ˌOˈɛs");
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
                read(&g2p, VoiceLanguage::BritishEnglish, text),
                to_kokoro_phonemes(&raw, Dialect::British),
                "{text}"
            );
        }
    }

    #[test]
    fn camel_case_reads_as_its_words() {
        let g2p = G2P::new(Language::EnglishGB);
        let read_gb = |text| read(&g2p, VoiceLanguage::BritishEnglish, text);
        assert_eq!(read_gb("GitHub"), read_gb("Git Hub"));
        assert_eq!(read_gb("HTMLParser"), "ˌAʧtˌiːˌɛmˈɛl pˈɑːsə");
        assert_eq!(read_gb("iPhone"), "ˈI fˈQn");
        assert_eq!(read_gb("JSON"), "ʤˈAsᵊn");
        assert_eq!(read_gb("YAML"), "jˈam ˌɛl");
    }

    #[test]
    fn a_stop_inside_a_name_is_no_sentence_end() {
        let g2p = G2P::new(Language::EnglishGB);
        let read_gb = |text| read(&g2p, VoiceLanguage::BritishEnglish, text);
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
            reading(&g2p, Names::Here, sentence, VoiceLanguage::BritishEnglish),
            "Doctor Smith met Mister Jones at seven thirty PM."
        );
        let read_gb = |text| read(&g2p, VoiceLanguage::BritishEnglish, text);
        assert_eq!(read_gb(sentence).matches('.').count(), 1);
        assert_eq!(read_gb("Ms. Smith"), "mˈɪz smˈɪθ");
        assert_eq!(read_gb("Baker St. is long."), "bˈAkə stɹˈiːt ɪz lˈɒŋ.");
        assert_eq!(read_gb("fruit, etc. Then"), "fɹˈuːt, ˈɛt sˈɛtəɹə. ðˈɛn");
    }

    #[test]
    fn brackets_in_the_text_keep_their_words() {
        let g2p = G2P::new(Language::EnglishGB);
        let read_gb = |text| read(&g2p, VoiceLanguage::BritishEnglish, text);
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
        let ids = |text| {
            text_to_ids(&g2p, Names::Here, None, VoiceLanguage::BritishEnglish, text).unwrap()
        };
        assert_eq!(ids("\u{2212}5 degrees"), ids("minus five degrees"));
    }

    #[test]
    fn curly_double_quotes_read_as_straight_ones() {
        let (g2p, _) = recording_g2p(Language::EnglishGB);
        let ids = |text| {
            text_to_ids(&g2p, Names::Here, None, VoiceLanguage::BritishEnglish, text).unwrap()
        };
        let (curly, dropped) = ids("She said “no” twice.");
        assert_eq!(dropped, 0);
        assert_eq!((curly.clone(), dropped), ids("She said \"no\" twice."));
        assert!(
            !curly.windows(2).any(|pair| pair == [SPACE_ID, SPACE_ID]),
            "{curly:?}"
        );
        assert_eq!(plain_english("„no“"), "\"no\"");
    }

    #[test]
    fn a_sentence_with_curly_apostrophes_loses_no_symbol() {
        let (g2p, unknown) = recording_g2p(Language::EnglishGB);
        let text = "I don’t think it’s the user’s fault that the naïve café shuts at nine o’clock.";
        let (ids, dropped) =
            text_to_ids(&g2p, Names::Here, None, VoiceLanguage::BritishEnglish, text).unwrap();
        assert_eq!(dropped, 0);
        assert!(has_phonemes(&ids));
        assert!(unknown.lock().unwrap().is_empty(), "{unknown:?}");
    }

    #[test]
    fn invisible_characters_go_and_no_break_spaces_become_spaces() {
        assert_eq!(
            plain_english("soft\u{ad}ware\u{200b} \u{feff}ten\u{a0}pounds"),
            "software ten pounds"
        );
        assert_eq!(plain_english("Straße Œuvre ﬂoor"), "Strasse OEuvre floor");
        assert_eq!(plain_english("日本 🎉"), "日本 🎉");
    }

    #[test]
    fn a_spaced_dash_becomes_a_comma() {
        assert_eq!(
            plain_english("Tea — or coffee – is fine"),
            "Tea, or coffee, is fine"
        );
        assert_eq!(plain_english("Wait, — no."), "Wait, no.");
        for kept in ["well—maybe", "1914 – 1918", "£5 – £10", "— Yes", "fine —"] {
            assert_eq!(plain_english(kept), kept);
        }
        let (g2p, _) = recording_g2p(Language::EnglishGB);
        let (ids, _) = text_to_ids(
            &g2p,
            Names::Here,
            None,
            VoiceLanguage::BritishEnglish,
            "Tea — or coffee",
        )
        .unwrap();
        assert!(ids.contains(&3), "a comma id: {ids:?}");
    }

    fn language_of(voice: &str) -> VoiceLanguage {
        ModelFiles::new(Path::new("/m"), ModelVariant::Int8, voice).language
    }

    #[test]
    fn a_voice_reads_in_its_language_when_this_build_can() {
        for voice in ["bm_lewis", "bf_emma", "BM_Lewis", "bob_custom"] {
            assert_eq!(language_of(voice), VoiceLanguage::BritishEnglish, "{voice}");
        }
        for voice in ["af_heart", "am_adam", "AF_Heart"] {
            assert_eq!(
                language_of(voice),
                VoiceLanguage::AmericanEnglish,
                "{voice}"
            );
        }
        for voice in ["jf_alpha", "jm_kumo", "jf_nezumi+af_heart"] {
            assert_eq!(language_of(voice), VoiceLanguage::Japanese, "{voice}");
        }
        for voice in [
            "zf_xiaobei",
            "zm_yunyang",
            "Zf_xiaobei",
            "zf_xiaobei+bf_emma",
        ] {
            assert_eq!(language_of(voice), VoiceLanguage::Mandarin, "{voice}");
        }
    }

    #[test]
    fn an_unknown_prefix_reads_as_american_english() {
        for voice in ["xx_custom", "qm_custom", "xx_custom+zf_xiaobei"] {
            assert_eq!(
                language_of(voice),
                VoiceLanguage::AmericanEnglish,
                "{voice}"
            );
        }
    }

    #[test]
    fn the_espeak_ng_languages_read_their_own_language() {
        for (voice, language) in [
            ("ef_dora", VoiceLanguage::Spanish),
            ("em_alex", VoiceLanguage::Spanish),
            ("ff_siwis", VoiceLanguage::French),
            ("if_sara", VoiceLanguage::Italian),
            ("im_nicola", VoiceLanguage::Italian),
            ("pf_dora", VoiceLanguage::BrazilianPortuguese),
            ("hf_alpha", VoiceLanguage::Hindi),
            ("HM_Psi", VoiceLanguage::Hindi),
            ("ff_siwis+af_alloy", VoiceLanguage::French),
        ] {
            assert_eq!(language_of(voice), language, "{voice}");
            let files = ModelFiles::new(Path::new("/m"), ModelVariant::Int8, voice);
            assert!(files.needs_espeak(), "{voice}");
        }
        for voice in [
            "af_alloy",
            "bm_lewis",
            "jf_alpha",
            "zf_xiaobei",
            "xx_custom",
        ] {
            let files = ModelFiles::new(Path::new("/m"), ModelVariant::Int8, voice);
            assert!(!files.needs_espeak(), "{voice}");
        }
    }

    #[test]
    fn a_language_espeak_ng_reads_needs_it_installed() {
        let files = |voice| ModelFiles::new(Path::new("/m"), ModelVariant::Int8, voice);
        let program = || Some(PathBuf::from("/opt/homebrew/bin/espeak-ng"));
        assert_eq!(
            files("ef_dora").unmet_requirement().as_deref(),
            Some("Spanish voices need espeak-ng, or Reading Language set to English.")
        );
        assert!(files("pf_dora")
            .unmet_requirement()
            .unwrap()
            .starts_with("Portuguese voices need espeak-ng,"));
        for voice in ["ef_dora", "ff_siwis", "if_sara", "pf_dora", "hf_alpha"] {
            assert!(files(voice).unmet_requirement().is_some(), "{voice}");
            let found = files(voice).with_espeak(program());
            assert_eq!(found.unmet_requirement(), None, "{voice}");
            // The English fallback setting has no say.
            let found = found.with_english_fallback(false);
            assert_eq!(found.unmet_requirement(), None, "{voice}");
        }
        for voice in ["af_alloy", "bm_lewis", "jf_alpha", "zf_xiaobei"] {
            assert_eq!(files(voice).unmet_requirement(), None, "{voice}");
        }
    }

    #[test]
    fn a_language_espeak_ng_reads_cannot_load_without_it() {
        let reading = |espeak| Reading {
            language: VoiceLanguage::French,
            english_fallback: false,
            espeak,
        };
        let error = Phonemizer::new(&reading(None)).err().unwrap();
        assert!(error.starts_with("French voices need espeak-ng"), "{error}");
        let program = PathBuf::from("/opt/homebrew/bin/espeak-ng");
        assert!(matches!(
            Phonemizer::new(&reading(Some(program))),
            Ok(Phonemizer::Espeak(_))
        ));
        let english = Reading {
            language: VoiceLanguage::BritishEnglish,
            english_fallback: false,
            espeak: None,
        };
        assert!(matches!(
            Phonemizer::new(&english),
            Ok(Phonemizer::Misaki { .. })
        ));
    }

    #[test]
    fn a_blend_reads_in_its_first_voices_language() {
        assert_eq!(
            language_of("bm_lewis+af_alloy"),
            VoiceLanguage::BritishEnglish
        );
        assert_eq!(
            language_of("af_alloy(9)+bm_lewis"),
            VoiceLanguage::AmericanEnglish
        );
        assert_eq!(language_of("zf_xiaobei+bf_emma"), VoiceLanguage::Mandarin);
        // Not a voice or blend: loading fails on the voices, not the language.
        assert_eq!(
            language_of("bm_lewis-af_alloy"),
            VoiceLanguage::AmericanEnglish
        );
    }

    #[test]
    fn reading_as_reads_in_the_language_given() {
        let lewis = || ModelFiles::new(Path::new("/m"), ModelVariant::Int8, "bm_lewis");
        let read_as = |language| lewis().reading_as(language).language;
        assert_eq!(
            read_as(VoiceLanguage::AmericanEnglish),
            VoiceLanguage::AmericanEnglish
        );
        assert_eq!(read_as(VoiceLanguage::Spanish), VoiceLanguage::Spanish);
        assert_eq!(read_as(VoiceLanguage::Mandarin), VoiceLanguage::Mandarin);
        let dora = ModelFiles::new(Path::new("/m"), ModelVariant::Int8, "ef_dora");
        assert_eq!(
            dora.reading_as(VoiceLanguage::BritishEnglish).language,
            VoiceLanguage::BritishEnglish
        );
    }

    fn japanese_with(tagger: Option<&Tagger>, text: &str) -> String {
        let g2p =
            build_g2p(&ModelFiles::new(Path::new("/m"), ModelVariant::Int8, "jm_kumo").reading());
        text_to_phonemes(&g2p, Names::Here, tagger, VoiceLanguage::Japanese, text).unwrap()
    }

    fn japanese(text: &str) -> String {
        japanese_with(None, text)
    }

    #[test]
    fn japanese_reads_kana_and_its_latin_words_in_english() {
        assert_eq!(japanese("きょうはOKです。"), "kʲoɯha ˌOkˈA desɨ.");
        assert_eq!(
            japanese("これはiPhone 15です。"),
            "koɾeha ˈI fˈOn fˈɪftin desɨ."
        );
        assert_eq!(japanese("「API」をつかう。"), "“ˌApˌiˈI” oʦɨkaɯ.");
        // With the dictionary: word spaces, は as "wa" and long vowels.
        let Some(tagger) = ja_dictionary::tagger_for_test() else {
            return;
        };
        let tagged = |text: &str| japanese_with(Some(tagger), text);
        assert_eq!(tagged("きょうはOKです。"), "kʲoː βa ˌOkˈA desɨ.");
        assert_eq!(
            tagged("私はiPhone 15を使う。"),
            "βataɕi βa ˈI fˈOn fˈɪftin o ʦɨkaɯ."
        );
    }

    /// The listening test's Japanese paragraph.
    const KANA_PARAGRAPH: &str = "きょうも、いいてんきですね。あさごはんをたべてから、こうえんまでさんぽにいきました。ベンチにすわって、コーヒーをのみながら、ちょっとだけほんをよみました。ラジオからきこえてくるおんがくが、とてもきもちよかったです。あしたもまた、いっしょにいきませんか？";

    #[test]
    fn a_japanese_chunk_fits_one_inference() {
        let files = ModelFiles::new(Path::new("/m"), ModelVariant::Int8, "jf_alpha");
        let g2p = build_g2p(&files.reading());
        let mut paragraphs = vec![(KANA_PARAGRAPH, None)];
        if let Some(tagger) = ja_dictionary::tagger_for_test() {
            paragraphs.push((KANA_PARAGRAPH, Some(tagger)));
            paragraphs.push((ja::KANJI_PROSE, Some(tagger)));
        }
        for (paragraph, tagger) in paragraphs {
            // One sentence of several hundred characters, cut at its commas.
            let long_sentence = paragraph.replace(['。', '？'], "、").repeat(2);
            let text = format!("{paragraph}{long_sentence}");
            let chunks =
                text::split_sentences_with(&text, files.max_chunk_chars(&text), files.marks(&text));
            assert!(chunks.iter().any(|chunk| chunk.text.chars().count() > 120));
            for chunk in &chunks {
                let (ids, dropped) = text_to_ids(
                    &g2p,
                    Names::Here,
                    tagger,
                    VoiceLanguage::Japanese,
                    &chunk.text,
                )
                .unwrap();
                assert_eq!(dropped, 0, "{}", chunk.text);
                assert!(
                    ids.len() <= PIECE_TOKENS,
                    "{} ids: {}",
                    ids.len(),
                    chunk.text
                );
            }
        }
    }

    #[test]
    fn only_japanese_without_the_dictionary_refuses_kanji() {
        let files = |voice| ModelFiles::new(Path::new("/m"), ModelVariant::Int8, voice);
        let kanji = "東京に行きます。";
        assert_eq!(
            files("jm_kumo").refuses(kanji).as_deref(),
            Some("Kanji need the dictionary in Settings → Models")
        );
        assert_eq!(files("jm_kumo").refuses(KANA_PARAGRAPH), None);
        assert_eq!(files("af_alloy").refuses(kanji), None);
        assert_eq!(files("zf_xiaobei").refuses(kanji), None);

        let models = tempfile::tempdir().unwrap();
        ja_dictionary::install_placeholder(models.path());
        let kumo = ModelFiles::new(models.path(), ModelVariant::Int8, "jm_kumo");
        assert_eq!(kumo.refuses(kanji), None);
        assert_eq!(kumo.refuses(ja::KANJI_PROSE), None);
    }

    #[test]
    fn a_chinese_voice_cuts_text_with_no_chinese_as_english() {
        let xiaobei = ModelFiles::new(Path::new("/m"), ModelVariant::Int8, "zf_xiaobei");
        assert_eq!(xiaobei.marks("It works. Try it now."), &Marks::DEFAULT);
        assert_eq!(
            xiaobei.max_chunk_chars("It works. Try it now."),
            MAX_CHUNK_CHARS
        );
        assert_eq!(xiaobei.marks("今天的天气非常好。"), &Marks::CJK);
        assert_eq!(
            xiaobei.max_chunk_chars("今天的天气非常好。"),
            zh::MAX_CHUNK_CHARS
        );
    }

    /// The listening test's Mandarin paragraph.
    const MANDARIN_PARAGRAPH: &str = "今天天气很好，我们一起去公园散步吧。路上，我在银行门口遇到了一位老朋友，他说他重新开始学画画儿了。我们聊了一会儿，觉得时间过得真快：上次见面还是2019年！你明天有空吗？";

    #[test]
    fn a_mandarin_chunk_fits_one_inference() {
        let files = ModelFiles::new(Path::new("/m"), ModelVariant::Int8, "zf_xiaobei");
        let g2p = build_g2p(&files.reading());
        // One sentence of about 300 characters, cut at its commas.
        let long_sentence = MANDARIN_PARAGRAPH
            .replace(['。', '！', '？', '：'], "，")
            .repeat(3);
        let text = format!("{MANDARIN_PARAGRAPH}{long_sentence}");
        let chunks =
            text::split_sentences_with(&text, files.max_chunk_chars(&text), files.marks(&text));
        assert!(chunks.iter().any(|chunk| chunk.text.chars().count() > 80));
        for chunk in &chunks {
            assert!(
                chunk.text.chars().count() <= zh::MAX_CHUNK_CHARS,
                "{}",
                chunk.text
            );
            let (ids, dropped) = text_to_ids(
                &g2p,
                Names::Here,
                None,
                VoiceLanguage::Mandarin,
                &chunk.text,
            )
            .unwrap();
            assert_eq!(dropped, 0, "{}", chunk.text);
            assert!(
                ids.len() <= PIECE_TOKENS,
                "{} ids: {}",
                ids.len(),
                chunk.text
            );
        }
        let cut: Vec<_> = chunks.iter().filter(|chunk| !chunk.ends_sentence).collect();
        assert!(!cut.is_empty());
        assert!(
            cut.iter().all(|chunk| chunk.text.ends_with('，')),
            "{cut:?}"
        );
    }

    #[test]
    fn a_chinese_voice_reads_mandarin_as_misaki_does() {
        let files = ModelFiles::new(Path::new("/m"), ModelVariant::Int8, "zf_xiaobei");
        let g2p = build_g2p(&files.reading());
        let goldens = include_str!("fixtures/mandarin-goldens.txt");
        for (text, expected) in goldens
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split_once('\t'))
        {
            let read = text_to_phonemes(&g2p, Names::Here, None, files.language(), text).unwrap();
            assert_eq!(read, expected, "{text}");
        }
    }

    #[test]
    fn english_inside_mandarin_reads_as_an_american_voice_reads_it() {
        let g2p = G2P::new(Language::EnglishUS);
        let american = |text: &str| {
            text_to_phonemes(
                &g2p,
                Names::Here,
                None,
                VoiceLanguage::AmericanEnglish,
                text,
            )
            .unwrap()
        };
        let mandarin = |text: &str| {
            text_to_phonemes(&g2p, Names::Here, None, VoiceLanguage::Mandarin, text).unwrap()
        };
        for english in [
            "OK",
            "iPhone",
            "Hello, world",
            "naïve café",
            "Tom & Jerry",
            "it’s the user’s fault",
            "well-known",
            "GitHub",
        ] {
            assert_eq!(
                mandarin(&format!("这是{english}。")),
                format!("ꭧɤ↘ʂɨ↘ {}.", american(english)),
                "{english}"
            );
            assert_eq!(mandarin(english), american(english), "{english}");
        }
        assert_eq!(
            mandarin("我用iPhone拍照，很方便。"),
            format!("wo↓jʊ↘ŋ {} pʰai→ꭧau↘, xə↓n fa→ŋpjɛ↘n.", american("iPhone"))
        );
    }

    #[test]
    fn a_japanese_voice_cuts_text_with_no_japanese_as_english() {
        let kumo = ModelFiles::new(Path::new("/m"), ModelVariant::Int8, "jm_kumo");
        assert_eq!(kumo.marks("It works. Try it now."), &Marks::DEFAULT);
        assert_eq!(
            kumo.max_chunk_chars("It works. Try it now."),
            MAX_CHUNK_CHARS
        );
        assert_eq!(kumo.marks("きょうはOKです。"), &Marks::CJK);
        assert_eq!(
            kumo.max_chunk_chars("きょうはOKです。"),
            ja::MAX_CHUNK_CHARS
        );
    }

    #[test]
    fn english_inside_japanese_reads_as_an_american_voice_reads_it() {
        let g2p = G2P::new(Language::EnglishUS);
        let american = |text: &str| {
            text_to_phonemes(
                &g2p,
                Names::Here,
                None,
                VoiceLanguage::AmericanEnglish,
                text,
            )
            .unwrap()
        };
        let japanese = |text: &str| {
            text_to_phonemes(&g2p, Names::Here, None, VoiceLanguage::Japanese, text).unwrap()
        };
        let tagger = ja_dictionary::tagger_for_test();
        let tagged = |text: &str| {
            text_to_phonemes(&g2p, Names::Here, tagger, VoiceLanguage::Japanese, text).unwrap()
        };
        for english in [
            "£4.99",
            "10:30 a.m.",
            "$3.2 billion",
            "50% off",
            "(1) Install it",
            "naïve café",
            "\"quoted\" words",
            "Tom & Jerry",
            "it’s the user’s fault",
            "well-known",
        ] {
            assert_eq!(
                japanese(&format!("これは{english}です。")),
                format!("koɾeha {} desɨ.", american(english)),
                "{english}"
            );
            assert_eq!(japanese(english), american(english), "{english}");
            if tagger.is_some() {
                assert_eq!(
                    tagged(&format!("これは{english}です。")),
                    format!("koɾe βa {} desɨ.", american(english)),
                    "{english}"
                );
                assert_eq!(tagged(english), american(english), "{english}");
            }
        }
    }

    #[test]
    fn espeak_reads_unknown_english_words_only_with_the_fallback_on() {
        let program = PathBuf::from("/opt/homebrew/bin/espeak-ng");
        let reading = |english_fallback, espeak| Reading {
            language: VoiceLanguage::BritishEnglish,
            english_fallback,
            espeak,
        };
        assert_eq!(
            reading(true, Some(program.clone())).english_espeak(),
            Some(&program)
        );
        assert_eq!(reading(false, Some(program.clone())).english_espeak(), None);
        assert_eq!(reading(true, None).english_espeak(), None);
        assert_eq!(reading(false, None).english_espeak(), None);
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
    fn a_blend_is_the_weighted_average_of_the_tables() {
        let a: Vec<f32> = (0..2 * STYLE_DIM).map(|i| i as f32).collect();
        let b: Vec<f32> = (0..2 * STYLE_DIM).map(|i| 1000.0 - i as f32).collect();
        let blended = blend(&[(&a, 2.0), (&b, 1.0)]).unwrap();
        assert_eq!(blended.len(), a.len());
        for (i, value) in blended.iter().enumerate() {
            let expected = (2.0 * i as f32 + 1000.0 - i as f32) / 3.0;
            assert!(
                (value - expected).abs() < 1e-3,
                "{i}: {value} vs {expected}"
            );
        }
        assert_eq!(style_row(&blended, 1)[0], blended[STYLE_DIM]);
    }

    #[test]
    fn one_voice_blends_to_its_own_table() {
        let a: Vec<f32> = (0..2 * STYLE_DIM).map(|i| i as f32 * 0.37 - 5.0).collect();
        assert_eq!(blend(&[(&a, 3.0)]).unwrap(), a);
    }

    #[test]
    fn tables_of_different_lengths_do_not_blend() {
        let two = numbered_rows(2);
        let three = numbered_rows(3);
        assert!(blend(&[(&two, 1.0), (&three, 1.0)]).is_err());
        assert!(blend(&[]).is_err());
    }

    fn write_voice(models: &Path, name: &str, value: f32) {
        let voices = voices_dir(models);
        fs::create_dir_all(&voices).unwrap();
        let bytes: Vec<u8> = std::iter::repeat_n(value.to_le_bytes(), 2 * STYLE_DIM)
            .flatten()
            .collect();
        fs::write(voices.join(format!("{name}.bin")), bytes).unwrap();
    }

    fn stamps(models: &Path, voice: &str) -> Vec<(FileStamp, f32)> {
        let files = ModelFiles::new(models, ModelVariant::Int8, voice);
        voice_stamps(files.voices.as_deref().unwrap()).unwrap()
    }

    #[test]
    fn a_weight_change_reloads_the_blend() {
        let dir = tempfile::tempdir().unwrap();
        write_voice(dir.path(), "af_one", 1.0);
        write_voice(dir.path(), "af_four", 4.0);

        let mut voice = Voice::load(stamps(dir.path(), "af_one+af_four")).unwrap();
        assert_eq!(voice.style(0)[0], 2.5);

        let reweighted = stamps(dir.path(), "af_one(3)+af_four");
        assert_ne!(voice.parts, reweighted);
        voice.refresh(reweighted.clone()).unwrap();
        assert_eq!(voice.parts, reweighted);
        assert_eq!(voice.style(1)[STYLE_DIM - 1], 1.75);

        // The same blend in another order is what is loaded.
        assert_eq!(stamps(dir.path(), "af_four+af_one(3)"), voice.parts);
    }

    #[test]
    fn a_failed_reload_keeps_the_loaded_blend() {
        let dir = tempfile::tempdir().unwrap();
        write_voice(dir.path(), "af_one", 1.0);
        write_voice(dir.path(), "af_four", 4.0);
        let mut voice = Voice::load(stamps(dir.path(), "af_one")).unwrap();
        let loaded = voice.parts.clone();

        let broken = stamps(dir.path(), "af_one+af_four");
        fs::write(voices_dir(dir.path()).join("af_four.bin"), b"short").unwrap();
        assert!(voice.refresh(broken).is_err());
        assert_eq!(voice.parts, loaded);
        assert_eq!(voice.style(0)[0], 1.0);
    }

    #[test]
    fn short_input_is_one_piece() {
        let ids = vec![43; PIECE_TOKENS];
        assert_eq!(split_tokens(&ids, PIECE_TOKENS, false), vec![&ids[..]]);
        assert!(split_tokens(&[], PIECE_TOKENS, false).is_empty());
        assert!(split_tokens(&[SPACE_ID, SPACE_ID], PIECE_TOKENS, false).is_empty());
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
        let pieces = split_tokens(&ids, PIECE_TOKENS, false);
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
        let pieces = split_tokens(&words, PIECE_TOKENS, false);
        assert_eq!(pieces.len(), 4);
        for piece in &pieces {
            assert!(piece.len() <= PIECE_TOKENS);
            assert_eq!((piece.first(), piece.last()), (Some(&43), Some(&43)));
        }

        let solid = vec![43; 2 * PIECE_TOKENS + 100];
        let pieces = split_tokens(&solid, PIECE_TOKENS, false);
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
        let pieces = split_tokens(&ids, PIECE_TOKENS, false);
        assert!(pieces.iter().all(|p| has_phonemes(p)), "{pieces:?}");
    }

    /// Where each piece ends in `ids`.
    fn piece_ends(ids: &[i64], pieces: &[&[i64]]) -> Vec<usize> {
        pieces
            .iter()
            .map(|piece| piece.as_ptr() as usize - ids.as_ptr() as usize)
            .zip(pieces)
            .map(|(offset, piece)| offset / size_of::<i64>() + piece.len())
            .collect()
    }

    #[test]
    fn a_mark_inside_a_word_is_no_place_to_cut() {
        // "lˈʌr.kˌaː", "lˈʌr.kˌaː", …: a "." inside every word.
        let ids: Vec<i64> = [54, 138, 123, 4, 53, 43, SPACE_ID]
            .iter()
            .copied()
            .cycle()
            .take(3 * PIECE_TOKENS)
            .collect();
        let inside = split_tokens(&ids, PIECE_TOKENS, false);
        assert!(inside.iter().any(|piece| piece.last() == Some(&4)));
        let pieces = split_tokens(&ids, PIECE_TOKENS, true);
        for end in piece_ends(&ids, &pieces) {
            assert!(
                ids.get(end).is_none_or(|&id| id == SPACE_ID),
                "cut at {end}"
            );
        }
        assert!(pieces.iter().all(|piece| piece.len() <= PIECE_TOKENS));
    }

    #[test]
    fn a_long_hindi_sentence_is_cut_between_words() {
        let Some(program) = espeak::installed_for_test() else {
            return;
        };
        let files = ModelFiles::new(Path::new("/m"), ModelVariant::Int8, "hf_alpha")
            .with_espeak(Some(program));
        let phonemizer = Phonemizer::new(&files.reading()).unwrap();
        let sentence = format!(
            "{}यह बात सब जानते हैं।",
            "यह बड़ा लड़का और वह बड़ी लड़की पढ़ते हैं और ".repeat(9)
        );
        let chunks = chunks(&files, &sentence);
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        assert!(chunks[0].text.chars().count() > 400);
        let (ids, dropped) = phonemizer
            .ids(None, files.language(), &chunks[0].text)
            .unwrap();
        assert_eq!(dropped, 0);
        let pieces = split_tokens(&ids, PIECE_TOKENS, phonemizer.has_marks_in_words());
        assert!(pieces.len() > 1, "{} ids", ids.len());
        for end in piece_ends(&ids, &pieces) {
            assert!(
                ids.get(end).is_none_or(|&id| id == SPACE_ID),
                "cut at {end}"
            );
        }
    }

    #[test]
    fn a_long_sentence_is_read_in_one_inference() {
        let g2p = G2P::new(Language::EnglishGB);
        let ids = |text: &str| {
            text_to_ids(&g2p, Names::Here, None, VoiceLanguage::BritishEnglish, text)
                .unwrap()
                .0
        };
        let sentence = "The committee that met on the first floor of the old library building \
                        every Thursday afternoon decided after a long and careful discussion \
                        that the annual report should be rewritten from the beginning in plain \
                        language so that every member of the public could understand exactly \
                        what had been done with the money during the year.";
        let chunks = split_sentences(sentence, MAX_CHUNK_CHARS);
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            split_tokens(&ids(&chunks[0].text), PIECE_TOKENS, false).len(),
            1
        );

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
        let files = ModelFiles::new(models, ModelVariant::Int8, "bm_lewis(70)+bm_george(30)");
        assert_eq!(files.model, Path::new("/models/kokoro-v1.0/int8.onnx"));
        let voices: Vec<_> = files
            .voices
            .unwrap()
            .into_iter()
            .map(|voice| (voice.id, voice.path, voice.weight))
            .collect();
        assert_eq!(
            voices,
            vec![
                (
                    "bm_lewis".to_string(),
                    PathBuf::from("/models/kokoro-v1.0/voices/bm_lewis.bin"),
                    70.0
                ),
                (
                    "bm_george".to_string(),
                    PathBuf::from("/models/kokoro-v1.0/voices/bm_george.bin"),
                    30.0
                ),
            ]
        );
        assert_eq!(
            ModelFiles::new(models, ModelVariant::Fp32, "x").model,
            Path::new("/models/kokoro-v1.0/fp32.onnx")
        );
    }

    #[test]
    fn voice_names_cannot_leave_the_voices_folder() {
        let models = Path::new("/models");
        for name in [
            "../secret",
            "a/b",
            "",
            "bm lewis",
            "voice.bin",
            "bm_lewis+../x",
        ] {
            assert!(
                ModelFiles::new(models, ModelVariant::Int8, name)
                    .voices
                    .is_none(),
                "{name:?}"
            );
        }
    }

    #[test]
    fn missing_files_are_reported_model_first() {
        let dir = tempfile::tempdir().unwrap();
        let files = ModelFiles::new(dir.path(), ModelVariant::Int8, "bm_lewis");
        assert!(files.missing().unwrap().ends_with("int8.onnx"));
        assert!(files.model_missing());
        assert_eq!(files.missing_voices(), vec!["bm_lewis"]);

        let voices = voices_dir(dir.path());
        fs::create_dir_all(&voices).unwrap();
        fs::write(dir.path().join(KOKORO_DIR).join("int8.onnx"), b"model").unwrap();
        assert!(files.missing().unwrap().ends_with("bm_lewis.bin"));
        assert!(!files.model_missing());

        fs::write(voices.join("bm_lewis.bin"), b"voice").unwrap();
        assert_eq!(files.missing(), None);
        assert!(files.missing_voices().is_empty());
        for voice in ["../x", "ef_dora", "af_bella+af_sky", "my favourite voice"] {
            let files = ModelFiles::new(dir.path(), ModelVariant::Int8, voice);
            assert!(!files.missing_voices().is_empty(), "{voice}");
            assert!(files.missing().is_some(), "{voice}");
        }
    }

    #[test]
    fn every_absent_voice_of_a_blend_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        write_voice(dir.path(), "bm_lewis", 1.0);
        fs::write(dir.path().join(KOKORO_DIR).join("int8.onnx"), b"model").unwrap();

        let files = ModelFiles::new(
            dir.path(),
            ModelVariant::Int8,
            "bm_lewis+af_alloy+bf_emma(2)",
        );
        assert_eq!(files.missing_voices(), vec!["af_alloy", "bf_emma"]);
        assert!(files.missing().unwrap().ends_with("af_alloy.bin"));

        let files = ModelFiles::new(dir.path(), ModelVariant::Int8, "af_alloy+bm_lewis+af_alloy");
        assert_eq!(files.missing_voices(), vec!["af_alloy"]);
        let files = ModelFiles::new(dir.path(), ModelVariant::Int8, "bm_lewis(2)+bm_lewis");
        assert!(files.missing_voices().is_empty());
        assert_eq!(files.missing(), None);
    }

    #[test]
    fn a_setting_that_is_not_a_voice_counts_as_a_missing_voice() {
        let dir = tempfile::tempdir().unwrap();
        write_voice(dir.path(), "bm_lewis", 1.0);
        fs::write(dir.path().join(KOKORO_DIR).join("int8.onnx"), b"model").unwrap();
        for voice in ["", "   ", "bm_lewis-bm_george", "bm_lewis(0)", "+"] {
            let files = ModelFiles::new(dir.path(), ModelVariant::Int8, voice);
            assert_eq!(files.missing_voices(), vec![voice], "{voice:?}");
            assert!(files.missing().unwrap().contains("a voice file for"));
            let error = ensure_loaded(&mut None, &files).unwrap_err();
            assert!(error.contains("not a voice or blend"), "{voice:?}: {error}");
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

    #[test]
    fn installed_voices_leave_out_names_the_setting_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let voices = voices_dir(dir.path());
        fs::create_dir_all(&voices).unwrap();
        for name in ["bm_lewis.bin", "my-voice.bin", "bm_lewis-bm_george.bin"] {
            fs::write(voices.join(name), b"").unwrap();
        }
        let installed = installed_voices(dir.path());
        assert_eq!(installed, vec!["bm_lewis"]);
        for voice in installed {
            assert!(VoiceSpec::parse(&voice).is_ok(), "{voice}");
        }
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
            let (raw, _) = g2p
                .g2p(&reading(
                    &g2p,
                    Names::Here,
                    text,
                    VoiceLanguage::BritishEnglish,
                ))
                .unwrap();
            let phonemes = to_kokoro_phonemes(&raw, Dialect::British);
            let words = std::mem::take(&mut *unknown.lock().unwrap());
            println!("{text}\n  {phonemes}\n  not in the lexicon: {words:?}");
            assert_eq!(token_ids(&phonemes).1, 0, "{text}");
            oov += words.len();
            let Some(espeak) = &espeak else { continue };
            let (ids, dropped) = text_to_ids(
                espeak,
                Names::Here,
                None,
                VoiceLanguage::BritishEnglish,
                text,
            )
            .unwrap();
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
        for voice in ["bm_lewis", "af_alloy"] {
            let files = ModelFiles::new(&models, ModelVariant::Int8, voice);
            let language = files.language;
            let g2p = G2P::new(Dialect::of(language).language());
            let mut loaded = open(&files, &stop).unwrap();
            let mut seconds = |text: &str| {
                let samples = loaded.synthesize(text, 1.0, true).unwrap();
                let seconds = samples.len() as f32 / SAMPLE_RATE as f32;
                println!(
                    "{voice} {text:32} {seconds:.2} s  {}",
                    read(&g2p, language, text)
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
            let titles = read(&g2p, language, "Dr. Smith met Mr. Jones at 7.30pm.");
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
        let files = files.with_english_fallback(true).with_espeak(Some(program));
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

    /// Needs `bm_lewis.bin` and `bm_george.bin` beside the model:
    /// `MOUTHY_KOKORO_MODELS=<models dir> cargo test -- --ignored blends --nocapture`.
    #[test]
    #[ignore = "needs Kokoro model files"]
    fn local_voice_blends() {
        let models = PathBuf::from(std::env::var("MOUTHY_KOKORO_MODELS").unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let speak = |voice: &str| {
            let files = ModelFiles::new(&models, ModelVariant::Int8, voice);
            let started = Instant::now();
            let samples = open(&files, &stop)
                .unwrap()
                .synthesize("Hello from Mouthy, this is a blended voice.", 1.0, true)
                .unwrap();
            let seconds = samples.len() as f32 / SAMPLE_RATE as f32;
            println!(
                "{voice}: {seconds:.2} s of audio in {:?}",
                started.elapsed()
            );
            assert!((1.0..8.0).contains(&seconds), "{voice}: {seconds} s");
            samples
        };
        let lewis = speak("bm_lewis");
        let george = speak("bm_george");
        let blend = speak("bm_lewis(70)+bm_george(30)");
        assert!(blend != lewis && blend != george);
        // A weight change alone reloads the voice.
        assert!(speak("bm_lewis(30)+bm_george(70)") != blend);
    }

    /// One paragraph for each language espeak-ng reads, as in the listening
    /// test, with the voice that reads it.
    const PARAGRAPHS: &[(&str, &str)] = &[
        (
            "ef_dora",
            "El tren salió de Madrid a las 10:30 y llegó a Sevilla con veinte minutos de \
             retraso. ¿Quién lo habría dicho? En el vagón, el 3,5 % de los viajeros dormía; \
             los demás leían, hablaban o miraban por la ventana. ¡Qué viaje tan largo!",
        ),
        (
            "ff_siwis",
            "Les enfants sont dans le jardin et jouent sous les grands arbres. Le train de \
             8 heures est parti à l'heure, mais il pleuvait déjà à Lyon. « Tu viens ce soir ? » \
             m'a demandé Hélène ; je lui ai répondu que oui.",
        ),
        (
            "if_sara",
            "Il treno per Roma parte alle 10:30 dal binario 3. Ciao, come stai? Bene, grazie: \
             oggi fa caldo, ma domani pioverà. Gli gnocchi della nonna sono i migliori.",
        ),
        (
            "pf_dora",
            "Não sei se vou conseguir chegar às oito. O ônibus estava cheio, e o trânsito, \
             como sempre, parado. Você quer café ou chá? Obrigado pela atenção!",
        ),
        (
            "hf_alpha",
            "आज मौसम बहुत अच्छा है। कल शायद बारिश होगी, इसलिए छाता ज़रूर ले जाना। \
             मैं iPhone use करता हूँ। मेरे पास 25 रुपये हैं।",
        ),
    ];

    /// The chunks `files` reads `paragraph` as, cut as the hotkey would.
    fn chunks(files: &ModelFiles, paragraph: &str) -> Vec<text::Chunk> {
        let text = text::normalize(paragraph);
        text::split_sentences_with(&text, files.max_chunk_chars(&text), files.marks(&text))
    }

    #[test]
    fn each_espeak_ng_language_reads_its_paragraph_without_dropping_a_symbol() {
        let Some(program) = espeak::installed_for_test() else {
            return;
        };
        for &(voice, paragraph) in PARAGRAPHS {
            let files = ModelFiles::new(Path::new("/m"), ModelVariant::Int8, voice)
                .with_espeak(Some(program.clone()));
            let phonemizer = Phonemizer::new(&files.reading()).unwrap();
            let (mut ids, mut chars) = (0, 0);
            for chunk in chunks(&files, paragraph) {
                let (read, dropped) = phonemizer.ids(None, files.language(), &chunk.text).unwrap();
                assert_eq!(dropped, 0, "{voice}: {}", chunk.text);
                assert!(has_phonemes(&read), "{voice}: {}", chunk.text);
                // Language flags and brackets never reach Kokoro.
                assert!(!read.contains(&12) && !read.contains(&13), "{voice}");
                ids += read.len();
                chars += chunk.text.chars().count();
            }
            println!(
                "{voice}: {:.2} ids per character",
                ids as f32 / chars as f32
            );
        }
    }

    /// What `voice` gets from espeak-ng for `text`, and how many symbols it dropped.
    fn espeak_reads(program: &Path, voice: &str, text: &str) -> (String, usize) {
        let language = ModelFiles::new(Path::new("/m"), ModelVariant::Int8, voice).language();
        let sentences =
            espeak::Sentences::new(program.to_path_buf(), espeak::voice(language).unwrap());
        sentences.read(&plain_text(text, language)).unwrap()
    }

    #[test]
    fn decomposed_accents_read_as_composed_ones() {
        let Some(program) = espeak::installed_for_test() else {
            return;
        };
        for (voice, word, kokoro) in [
            ("pf_dora", "coração", "kˌoɾasˈɐ̃ʊ̃"),
            ("ff_siwis", "été", "etˈe"),
            ("ff_siwis", "café", "kafˈe"),
            ("ef_dora", "niño", "nˈiɲo"),
            ("if_sara", "città", "ʧitːˈa"),
        ] {
            let composed: String = word.nfc().collect();
            let decomposed: String = word.nfd().collect();
            assert_ne!(composed, decomposed);
            for form in [composed, decomposed] {
                assert_eq!(
                    espeak_reads(&program, voice, &form),
                    (kokoro.to_string(), 0),
                    "{voice} {form:?}"
                );
            }
        }
    }

    #[test]
    fn hindi_reads_the_same_in_every_normal_form() {
        let Some(program) = espeak::installed_for_test() else {
            return;
        };
        // ड़ and ढ़ never compose under NFC, so it takes them apart.
        let precomposed = "यह ब\u{95c}ा ल\u{95c}का प\u{95d}ता है।";
        let decomposed = "यह ब\u{921}\u{93c}ा ल\u{921}\u{93c}का प\u{922}\u{93c}ता है।";
        assert_eq!(precomposed.nfc().collect::<String>(), decomposed);
        for form in [precomposed, decomposed] {
            assert_eq!(
                espeak_reads(&program, "hf_alpha", form),
                ("jˌəh bˈʌr.aː lˈʌr.kˌaː pˈʌr.hətˌaː hɛː.".to_string(), 0),
                "{form:?}"
            );
        }
    }

    #[test]
    fn a_nasal_vowel_keeps_its_tilde_as_a_symbol_of_its_own() {
        let Some(program) = espeak::installed_for_test() else {
            return;
        };
        for (voice, text, nasal) in [
            ("ff_siwis", "Un bon vin blanc.", "ɔ\u{303}"),
            ("pf_dora", "Tem um mundo.", "u\u{303}"),
        ] {
            let decomposed: String = text.nfd().collect();
            let (phonemes, dropped) = espeak_reads(&program, voice, &decomposed);
            assert_eq!(dropped, 0, "{phonemes}");
            assert!(phonemes.contains(nasal), "{phonemes}");
            assert!(token_ids(&phonemes).0.contains(&17), "{phonemes}");
        }
    }

    /// Writes each paragraph as read by its voice, for the voices downloaded,
    /// to `mouthy-<voice>.wav` in `MOUTHY_WAV_DIR` or the temporary folder:
    /// `MOUTHY_KOKORO_MODELS=<models dir> cargo test -- --ignored paragraphs --nocapture`.
    #[test]
    #[ignore = "needs Kokoro model files and espeak-ng"]
    fn local_voices_read_their_language_paragraphs() {
        let models = PathBuf::from(std::env::var("MOUTHY_KOKORO_MODELS").unwrap());
        let out = std::env::var_os("MOUTHY_WAV_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let program = espeak::find().expect("espeak-ng is installed");
        let stop = Arc::new(AtomicBool::new(false));
        for &(voice, paragraph) in PARAGRAPHS {
            let files = ModelFiles::new(&models, ModelVariant::Int8, voice)
                .with_espeak(Some(program.clone()));
            if let Some(missing) = files.missing() {
                println!("{voice}: skipped, missing {missing}");
                continue;
            }
            let started = Instant::now();
            let mut loaded = open(&files, &stop).unwrap();
            let mut samples = Vec::new();
            for chunk in chunks(&files, paragraph) {
                samples.extend(
                    loaded
                        .synthesize(&chunk.text, 1.0, chunk.ends_sentence)
                        .unwrap(),
                );
            }
            drop(loaded);
            let seconds = samples.len() as f32 / SAMPLE_RATE as f32;
            assert!(seconds > 3.0, "{voice}: {seconds} s");
            let path = out.join(format!("mouthy-{voice}.wav"));
            write_wav(&path, &samples);
            println!(
                "{voice}: {seconds:.1} s of audio in {:?}, {}",
                started.elapsed(),
                path.display()
            );
        }
    }

    /// Writes `ef_dora` reading English text as English, as the English reading
    /// language has it, to `mouthy-ef_dora-as-english.wav` beside the paragraphs:
    /// `MOUTHY_KOKORO_MODELS=<models dir> cargo test -- --ignored as_english --nocapture`.
    #[test]
    #[ignore = "needs Kokoro model files"]
    fn a_spanish_voice_reads_english_as_english() {
        let models = PathBuf::from(std::env::var("MOUTHY_KOKORO_MODELS").unwrap());
        let out = std::env::var_os("MOUTHY_WAV_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let files = ModelFiles::new(&models, ModelVariant::Int8, "ef_dora")
            .reading_as(VoiceLanguage::AmericanEnglish);
        assert!(!files.needs_espeak());
        let text =
            "The build passed, and all 37 tests ran in 12 seconds. Dr. Smith will review it.";
        let stop = Arc::new(AtomicBool::new(false));
        let mut loaded = open(&files, &stop).unwrap();
        let mut samples = Vec::new();
        for chunk in chunks(&files, text) {
            samples.extend(
                loaded
                    .synthesize(&chunk.text, 1.0, chunk.ends_sentence)
                    .unwrap(),
            );
        }
        drop(loaded);
        let seconds = samples.len() as f32 / SAMPLE_RATE as f32;
        assert!(seconds > 3.0, "{seconds} s");
        let path = out.join("mouthy-ef_dora-as-english.wav");
        write_wav(&path, &samples);
        println!("ef_dora as English: {seconds:.1} s, {}", path.display());
    }

    /// 16-bit mono PCM at Kokoro's rate.
    fn write_wav(path: &Path, samples: &[i16]) {
        let data = u32::try_from(samples.len() * 2).unwrap();
        let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        fs::write(path, bytes).unwrap();
    }
}
