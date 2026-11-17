//! A separately installed espeak-ng: it reads the English words misaki-rs has
//! no entry for, and whole sentences in the languages Kokoro's own pipeline
//! reads through it. espeak-ng is GPL-3, so it only ever runs as its own
//! program, given the text on its command line and read back as text; Mouthy
//! never links, downloads or ships it.

use super::catalog::VoiceLanguage;
use super::local::{in_vocab, Dialect, TIES_COMMON};
use super::text::CLOSERS;
use log::{debug, info, warn};
use misaki_rs::fallback::FallbackError;
use misaki_rs::{Fallback, G2P};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// Homebrew's prefixes and the system's, checked before `PATH`: an app opened
/// from Finder or a desktop menu gets a `PATH` without Homebrew.
const KNOWN_LOCATIONS: &[&str] = &[
    "/opt/homebrew/bin/espeak-ng",
    "/usr/local/bin/espeak-ng",
    "/home/linuxbrew/.linuxbrew/bin/espeak-ng",
    "/usr/bin/espeak-ng",
];
/// The longest wait for one word. Speech waits for it, so it stays short.
const TIMEOUT: Duration = Duration::from_millis(500);
/// The longest wait for one sentence, which is all a sentence's audio waits on.
const SENTENCE_TIMEOUT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(1);
/// How long output may still be on its way once the program has exited.
const OUTPUT_GRACE: Duration = Duration::from_millis(100);
/// Far more than any sentence gives. A program that writes more blocks on
/// the full pipe and is stopped at its timeout, so memory stays bounded.
const OUTPUT_LIMIT: u64 = 1 << 20;
/// Longer "words" are spelled out rather than sent.
const LONGEST_WORD: usize = 48;
/// Pronunciations kept per G2P; the cache starts over when full.
const CACHE_LIMIT: usize = 10_000;

static REPORTED_MISSING: AtomicBool = AtomicBool::new(false);

/// eSpeak's symbols as Kokoro was trained on them, after hexgrad misaki's
/// `espeak.py` (`T` is Kokoro v1.0's flap). Runs after the ties, so `e‍ɪ` and
/// `e‍ə` are gone before a lone `e` becomes `A`.
const SYMBOLS: &[(&str, &str)] = &[
    ("e", "A"),
    ("ʲo", "jo"),
    ("ʲə", "jə"),
    ("ʲ", ""),
    ("ɚ", "əɹ"),
    ("r", "ɹ"),
    ("x", "k"),
    ("ç", "k"),
    ("ɐ", "ə"),
    ("\u{303}", ""),
    ("o", "ɔ"),
    ("ɾ", "T"),
    ("ʔ", "t"),
];
/// After a glottal stop, a syllabic n is a plain n: "button" is `bˈʌtn`.
const GLOTTAL_N: &[(&str, &str)] = &[("ʔˌn\u{329}", "ʔn"), ("ʔn\u{329}", "ʔn")];
const GB: &[(&str, &str)] = &[
    ("ə\u{200d}ʊ", "Q"),
    ("i\u{200d}ə", "ɪə"),
    ("e\u{200d}ə", "ɛː"),
    ("iə", "ɪə"),
];
/// In order: a rhotic `ɜː` keeps its r, then every other length mark goes.
const US: &[(&str, &str)] = &[
    ("o\u{200d}ʊ", "O"),
    ("ɜː\u{200d}ɹ", "ɜɹ"),
    ("ɜːɹ", "ɜɹ"),
    ("ɜː", "ɜɹ"),
    ("ɪə", "iə"),
    ("ː\u{200d}ɹ", "ɹ"),
    ("ː", ""),
];
const SYLLABIC: char = '\u{329}';
const TIE: char = '\u{200d}';

/// misaki's `EspeakG2P` replacements, in its order, for the sentence
/// languages; its `^` tie is espeak-ng's U+200D. Nasal vowels keep U+0303,
/// which Kokoro has a token for.
const SENTENCE_TIES: &[(&str, &str)] = &[
    ("a\u{200d}ɪ", "I"),
    ("a\u{200d}ʊ", "W"),
    ("d\u{200d}z", "ʣ"),
    ("d\u{200d}ʒ", "ʤ"),
    ("e\u{200d}ɪ", "A"),
    ("o\u{200d}ʊ", "O"),
    ("s\u{200d}s", "S"),
    ("t\u{200d}s", "ʦ"),
    ("t\u{200d}ʃ", "ʧ"),
    ("ɔ\u{200d}ɪ", "Y"),
    ("ə\u{200d}ʊ", "Q"),
];
/// Where espeak-ng ends a clause, and so an output line, when whitespace or
/// the end follows (closing quotes and brackets may come between): "3,5",
/// "1.000" and "10:30" stay one number. Its own rule differs at the edges,
/// such as a `¿` after a word or a `.` before a lower-case one; the line
/// count check catches those.
const CLAUSE_MARKS: &[char] = &[',', ';', ':', '.', '!', '?', '…', '—', '–', '।', '॥'];

/// The espeak-ng program, if one is installed where Mouthy looks.
pub fn find() -> Option<PathBuf> {
    find_in(KNOWN_LOCATIONS, std::env::var_os("PATH"))
}

/// The espeak-ng to use for unknown words, or `None` when the setting is off
/// or there is none; the first miss is logged.
pub fn find_if_enabled(enabled: bool) -> Option<PathBuf> {
    if !enabled {
        return None;
    }
    let found = find();
    if found.is_none() && !REPORTED_MISSING.swap(true, Ordering::Relaxed) {
        info!("espeak-ng was not found, so unknown words are spelled out");
    }
    found
}

fn find_in(known: &[&str], path_var: Option<OsString>) -> Option<PathBuf> {
    let program = format!("espeak-ng{}", std::env::consts::EXE_SUFFIX);
    known
        .iter()
        .map(PathBuf::from)
        .chain(
            path_var
                .iter()
                .flat_map(std::env::split_paths)
                .map(|dir| dir.join(&program)),
        )
        .find(|path| is_executable(path))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// A misaki G2P that asks `program` for the words its lexicon lacks.
pub(super) fn g2p(dialect: Dialect, program: PathBuf) -> G2P {
    g2p_with_timeout(dialect, program, TIMEOUT)
}

fn g2p_with_timeout(dialect: Dialect, program: PathBuf, timeout: Duration) -> G2P {
    let spellings = Arc::new(OnceLock::new());
    let fallback = Espeak {
        program,
        dialect,
        timeout,
        cache: Mutex::new(HashMap::new()),
        spellings: spellings.clone(),
    };
    let g2p = G2P::with_fallback(dialect.language(), Some(Box::new(fallback)));
    // Single characters never reach the fallback, so this cannot recurse.
    let _ = spellings.set(Spellings::of(&g2p));
    g2p
}

struct Espeak {
    program: PathBuf,
    dialect: Dialect,
    timeout: Duration,
    /// Words espeak-ng read, in Kokoro's symbols. Kept for as long as the G2P.
    cache: Mutex<HashMap<String, String>>,
    /// Filled in once the G2P exists; read when espeak-ng fails.
    spellings: Arc<OnceLock<Spellings>>,
}

impl Fallback for Espeak {
    /// Never an error: misaki-rs drops the whole sentence on one.
    fn phonemize(&self, word: &str) -> Result<String, FallbackError> {
        if !is_plain_word(word) {
            return Ok(self.spell(word));
        }
        if let Some(cached) = self.lock_cache().get(word) {
            return Ok(cached.clone());
        }
        let started = Instant::now();
        // The word itself stays out of the log: it is selected text.
        match run(&self.program, &args(self.dialect, word), self.timeout) {
            Ok(ipa) => {
                let (phonemes, dropped) = to_kokoro(&ipa, self.dialect);
                debug!(
                    "espeak-ng read an unknown word in {} ms ({dropped} symbols dropped)",
                    started.elapsed().as_millis()
                );
                if phonemes.is_empty() {
                    return Ok(self.spell(word));
                }
                let mut cache = self.lock_cache();
                if cache.len() >= CACHE_LIMIT {
                    cache.clear();
                }
                cache.insert(word.to_string(), phonemes.clone());
                Ok(phonemes)
            }
            Err(e) => {
                warn!("espeak-ng failed, so an unknown word is spelled out: {e}");
                Ok(self.spell(word))
            }
        }
    }
}

impl Espeak {
    fn lock_cache(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn spell(&self, word: &str) -> String {
        match self.spellings.get() {
            Some(spellings) => spellings.spell(word),
            None => String::new(),
        }
    }
}

/// ASCII letters and apostrophes only: espeak-ng's English voices read other
/// scripts as the names of their letters.
fn is_plain_word(word: &str) -> bool {
    word.len() <= LONGEST_WORD && word.chars().all(|c| c.is_ascii_alphabetic() || c == '\'')
}

/// What misaki-rs says without a fallback: each character read alone, joined
/// by spaces. Letters are looked up once, since the fallback cannot call G2P.
struct Spellings {
    letters: HashMap<char, String>,
    unknown: String,
}

impl Spellings {
    fn of(g2p: &G2P) -> Self {
        let read = |c: char| {
            g2p.g2p(&c.to_string())
                .map(|(phonemes, _)| phonemes)
                .unwrap_or_else(|_| g2p.unk.clone())
        };
        Self {
            letters: ('a'..='z')
                .chain('A'..='Z')
                .chain(['\''])
                .map(|c| (c, read(c)))
                .collect(),
            // Any character misaki-rs cannot read, as it reads one.
            unknown: read('\u{fffd}'),
        }
    }

    fn spell(&self, word: &str) -> String {
        word.chars()
            .map(|c| self.letters.get(&c).unwrap_or(&self.unknown).as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The word as one argument for the dialect's English voice.
fn args(dialect: Dialect, word: &str) -> Vec<String> {
    let voice = match dialect {
        Dialect::British => "en-gb",
        Dialect::American => "en-us",
    };
    command(voice, word)
}

/// `espeak-ng -q --ipa=3 -v <voice> -- <text>`: no audio, IPA with U+200D
/// ties (the form misaki-rs's lexicons use), the text as one argument that
/// cannot be read as an option ("-5 grados").
fn command(voice: &str, text: &str) -> Vec<String> {
    ["-q", "--ipa=3", "-v", voice, "--", text]
        .map(str::to_string)
        .to_vec()
}

/// espeak-ng's voice for a language Kokoro's own pipeline reads through it.
pub(super) fn voice(language: VoiceLanguage) -> Option<&'static str> {
    match language {
        VoiceLanguage::Spanish => Some("es"),
        VoiceLanguage::French => Some("fr-fr"),
        VoiceLanguage::Italian => Some("it"),
        VoiceLanguage::BrazilianPortuguese => Some("pt-br"),
        VoiceLanguage::Hindi => Some("hi"),
        _ => None,
    }
}

/// Reads whole sentences in one language, one espeak-ng process each and no
/// cache: espeak-ng reads numbers, times and the words of other languages
/// itself, which it can only do with the sentence whole.
pub(super) struct Sentences {
    program: PathBuf,
    voice: &'static str,
}

impl Sentences {
    pub(super) fn new(program: PathBuf, voice: &'static str) -> Self {
        Self { program, voice }
    }

    /// Kokoro's symbols for `sentence`, and how many of espeak-ng's had no
    /// token.
    pub(super) fn read(&self, sentence: &str) -> Result<(String, usize), String> {
        let text = without_controls(sentence);
        if text.trim().is_empty() {
            return Ok((String::new(), 0));
        }
        let started = Instant::now();
        let ipa = run(&self.program, &command(self.voice, &text), SENTENCE_TIMEOUT)
            .map_err(|e| format!("espeak-ng could not read a sentence: {e}"))?;
        // The sentence itself stays out of the log: it is selected text.
        debug!(
            "espeak-ng read a sentence in {} ms",
            started.elapsed().as_millis()
        );
        Ok(sentence_to_kokoro(&ipa, &clause_marks(&text)))
    }
}

/// `text` without NUL, which no argument can hold, or other control
/// characters; whitespace among them becomes a space.
fn without_controls(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            c if c.is_whitespace() => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// The marks that end a sentence's clauses, in order.
#[derive(Debug, PartialEq, Eq)]
struct ClauseMarks {
    /// Each as Kokoro reads it.
    marks: Vec<String>,
    /// Whether the last one ends the text.
    ends_text: bool,
}

/// The clause marks in `text`. A mark with nothing to say before it starts
/// no clause, as espeak-ng prints no line for one.
fn clause_marks(text: &str) -> ClauseMarks {
    let mut marks = Vec::new();
    let mut ends_text = false;
    let mut spoken = false;
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if !CLAUSE_MARKS.contains(&c) {
            spoken |= c.is_alphanumeric();
            continue;
        }
        let mut end = start + c.len_utf8();
        while let Some(&(i, next)) = chars.peek() {
            if !CLAUSE_MARKS.contains(&next) {
                break;
            }
            end = i + next.len_utf8();
            chars.next();
        }
        let after = text[end..].trim_start_matches(CLOSERS);
        if spoken && after.chars().next().is_none_or(char::is_whitespace) {
            marks.push(text[start..end].chars().map(kokoro_mark).collect());
            ends_text = after.trim().is_empty();
            spoken = false;
        }
    }
    ClauseMarks { marks, ends_text }
}

/// Kokoro has no token for the danda or the en dash.
fn kokoro_mark(mark: char) -> char {
    match mark {
        '।' | '॥' => '.',
        '–' => '—',
        mark => mark,
    }
}

/// espeak-ng's lines for one sentence as Kokoro's symbols, each clause given
/// back the mark that ended it, and how many symbols had no token. When the
/// lines do not match the marks, the clauses are joined by commas instead.
fn sentence_to_kokoro(ipa: &str, marks: &ClauseMarks) -> (String, usize) {
    let mut dropped = 0;
    let clauses: Vec<String> = ipa
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| clause_to_kokoro(line, &mut dropped))
        .collect();
    let Some(last) = clauses.len().checked_sub(1) else {
        return (String::new(), 0);
    };
    let expected = marks.marks.len() + usize::from(!marks.ends_text);
    let mut out = String::new();
    if clauses.len() == expected {
        for (i, clause) in clauses.iter().enumerate() {
            out.push_str(clause);
            if let Some(mark) = marks.marks.get(i) {
                out.push_str(mark);
            }
            if i < last {
                out.push(' ');
            }
        }
    } else {
        out = clauses.join(", ");
        if let Some(mark) = marks.marks.last().filter(|_| marks.ends_text) {
            out.push_str(mark);
        }
    }
    (out, dropped)
}

/// One of espeak-ng's lines as Kokoro's symbols, as misaki's `EspeakG2P`
/// maps them, counting the symbols with no token into `dropped`.
fn clause_to_kokoro(line: &str, dropped: &mut usize) -> String {
    let mapped = SENTENCE_TIES
        .iter()
        .fold(without_language_flags(line), |s, (from, to)| {
            s.replace(from, to)
        })
        // Ties left over, and French liaison's "le-z".
        .replace([TIE, '-'], "");
    let mut out = String::with_capacity(mapped.len());
    for c in mapped.chars() {
        match c {
            c if c.is_whitespace() => {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            c if in_vocab(c) => out.push(c),
            _ => *dropped += 1,
        }
    }
    out.trim_end().to_string()
}

/// `line` without the flags espeak-ng puts around words it reads in another
/// language, such as `(en)` and `(hi)`, which it writes with a U+200D before
/// each letter.
fn without_language_flags(line: &str) -> String {
    let is_flag = |inner: &str| {
        let mut letters = inner.chars().filter(|&c| c != TIE).peekable();
        letters.peek().is_some() && letters.all(|c| c.is_ascii_lowercase() || c == '-')
    };
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('(') {
        let flag = rest[open + 1..]
            .find(')')
            .filter(|&close| is_flag(&rest[open + 1..open + 1 + close]));
        match flag {
            Some(close) => {
                out.push_str(&rest[..open]);
                rest = &rest[open + close + 2..];
            }
            None => {
                out.push_str(&rest[..=open]);
                rest = &rest[open + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Runs `program` without a shell or stdin and returns its output, killing it
/// after `timeout`. The output is read while it runs, so a long one cannot
/// fill the pipe and stall it.
fn run(program: &Path, args: &[String], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", program.display()))?;
    let deadline = Instant::now() + timeout;
    let output = match read_output(&mut child) {
        Ok(output) => output,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("no answer within {} ms", timeout.as_millis()));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("cannot wait for it: {e}"));
            }
        }
    };
    if !status.success() {
        return Err(format!("it exited with {status}"));
    }
    let wait = deadline
        .saturating_duration_since(Instant::now())
        .max(OUTPUT_GRACE);
    let bytes = output
        .recv_timeout(wait)
        .map_err(|_| "its output did not end".to_string())?
        .map_err(|e| format!("cannot read its output: {e}"))?;
    String::from_utf8(bytes).map_err(|e| format!("cannot read its output: {e}"))
}

/// Reads `child`'s output to the end on a thread of its own.
fn read_output(child: &mut Child) -> Result<Receiver<std::io::Result<Vec<u8>>>, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "its output was not captured".to_string())?;
    let (send, receive) = mpsc::channel();
    thread::Builder::new()
        .name("espeak-output".into())
        .spawn(move || {
            let mut output = Vec::new();
            let read = stdout.take(OUTPUT_LIMIT).read_to_end(&mut output);
            let _ = send.send(read.map(|_| output));
        })
        .map_err(|e| format!("cannot read its output: {e}"))?;
    Ok(receive)
}

/// Kokoro symbols for espeak-ng's IPA, and how many symbols Kokoro has no
/// token for (dropped).
fn to_kokoro(ipa: &str, dialect: Dialect) -> (String, usize) {
    let dialect_rules = match dialect {
        Dialect::British => GB,
        Dialect::American => US,
    };
    let words = ipa.split_whitespace().collect::<Vec<_>>().join(" ");
    let mapped = GLOTTAL_N
        .iter()
        .chain(TIES_COMMON)
        .chain(dialect_rules)
        .chain(SYMBOLS)
        .fold(words, |s, (from, to)| s.replace(from, to));
    let mut out = String::with_capacity(mapped.len());
    let mut dropped = 0;
    for c in mapped.chars() {
        match c {
            TIE => {}
            // A syllabic consonant is read with a schwa before it.
            SYLLABIC => match out.pop() {
                Some(consonant) if consonant != ' ' => {
                    out.push('ᵊ');
                    out.push(consonant);
                }
                Some(space) => out.push(space),
                None => {}
            },
            c if in_vocab(c) => out.push(c),
            _ => dropped += 1,
        }
    }
    (out, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn british_espeak_output_maps_to_kokoro_symbols() {
        for (ipa, kokoro) in [
            (" kˌuːbənˈɛtiːz\n", "kˌuːbənˈɛtiːz"),
            ("kˈə\u{200d}ʊməndˌɛli", "kˈQməndˌɛli"),
            ("kˈiːθli", "kˈiːθli"),
            ("ʃɪvˈɔːn", "ʃɪvˈɔːn"),
            ("nˈa\u{200d}ɪəm", "nˈIəm"),
            ("hˈe\u{200d}ə", "hˈɛː"),
            ("nˌi\u{200d}ə", "nˌɪə"),
            ("bˈɒtə\u{200d}l", "bˈɒtᵊl"),
            ("lˈɒx", "lˈɒk"),
            ("ɐlnmˈa\u{200d}ʊθ", "əlnmˈWθ"),
            ("ɬanˈɛlɪ", "lanˈɛlɪ"),
            ("kwˈɑːsɑ̃", "kwˈɑːsɑ"),
            ("ɹˈe\u{200d}ɪkd\u{200d}ʒɐvˌɪk", "ɹˈAkʤəvˌɪk"),
            ("tˈiːt\u{200d}ʃe\u{200d}ɪkˈɒvski", "tˈiːʧAkˈɒvski"),
            ("fˈa\u{200d}ɪ\u{200d}əɹ", "fˈIəɹ"),
            ("lˈɔ\u{200d}ɪə", "lˈYə"),
            ("pˈə\u{200d}ʊstɡəɹ ˌɛskjˌuːˈɛl", "pˈQstɡəɹ ˌɛskjˌuːˈɛl"),
        ] {
            assert_eq!(
                to_kokoro(ipa, Dialect::British),
                (kokoro.into(), 0),
                "{ipa}"
            );
        }
    }

    #[test]
    fn american_espeak_output_maps_to_kokoro_symbols() {
        for (ipa, kokoro) in [
            ("kˌuːbɚnˈɛɾiːz", "kˌubəɹnˈɛTiz"),
            ("kˈo\u{200d}ʊməndˌɛli", "kˈOməndˌɛli"),
            ("bˈɜːd", "bˈɜɹd"),
            ("kˈɜːɹi", "kˈɜɹi"),
            ("sˈa\u{200d}ʊθwɔː\u{200d}ɹk", "sˈWθwɔɹk"),
            ("bˈʌʔn\u{329}", "bˈʌtn"),
            ("hˈɛ\u{200d}ɹ", "hˈɛɹ"),
            ("ɹˈæt\u{200d}ʃmɐnˌɪnɔf", "ɹˈæʧmənˌɪnɔf"),
            ("skˈɛd\u{200d}ʒuːl", "skˈɛʤul"),
            ("fˈa\u{200d}ɪ\u{200d}ɚɹ", "fˈIəɹɹ"),
        ] {
            assert_eq!(
                to_kokoro(ipa, Dialect::American),
                (kokoro.into(), 0),
                "{ipa}"
            );
        }
    }

    #[test]
    fn syllabic_consonants_take_a_schwa() {
        assert_eq!(to_kokoro("bˈʌtn\u{329}", Dialect::British).0, "bˈʌtᵊn");
        assert_eq!(to_kokoro("\u{329}n", Dialect::British).0, "n");
    }

    #[test]
    fn symbols_outside_the_vocab_are_dropped_and_counted() {
        // ʉ and ɵ: espeak vowels with no Kokoro token.
        assert_eq!(to_kokoro("ɡʉːsɵ", Dialect::British), ("ɡːs".to_string(), 2));
    }

    #[test]
    fn mapped_output_needs_no_further_tie_mapping() {
        // The G2P output goes through the lexicon's tie mapping again.
        for dialect in [Dialect::British, Dialect::American] {
            let (mapped, _) = to_kokoro("kˈə\u{200d}ʊ d\u{200d}ʒˈo\u{200d}ʊ", dialect);
            assert!(!mapped.contains(TIE), "{mapped}");
            assert!(mapped.chars().all(in_vocab), "{mapped}");
        }
    }

    #[test]
    fn the_word_is_one_argument_after_the_options() {
        assert_eq!(
            args(Dialect::British, "Cholmondeley"),
            ["-q", "--ipa=3", "-v", "en-gb", "--", "Cholmondeley"]
        );
        assert_eq!(
            args(Dialect::American, "O'Neill")[3..],
            ["en-us", "--", "O'Neill"]
        );
    }

    #[test]
    fn a_sentence_is_one_argument_after_the_options() {
        assert_eq!(
            command("es", "-5 grados"),
            ["-q", "--ipa=3", "-v", "es", "--", "-5 grados"]
        );
    }

    #[test]
    fn each_espeak_ng_language_has_kokoros_voice_for_it() {
        for (language, name) in [
            (VoiceLanguage::Spanish, "es"),
            (VoiceLanguage::French, "fr-fr"),
            (VoiceLanguage::Italian, "it"),
            (VoiceLanguage::BrazilianPortuguese, "pt-br"),
            (VoiceLanguage::Hindi, "hi"),
        ] {
            assert_eq!(voice(language), Some(name), "{language:?}");
        }
        for language in [
            VoiceLanguage::AmericanEnglish,
            VoiceLanguage::BritishEnglish,
            VoiceLanguage::Japanese,
            VoiceLanguage::Mandarin,
        ] {
            assert_eq!(voice(language), None, "{language:?}");
        }
    }

    #[test]
    fn control_characters_never_reach_espeak_ng() {
        assert_eq!(without_controls("a\u{0}b\u{7}c\td\ne\u{1b}"), "abc d e");
        assert_eq!(without_controls("¿Qué?"), "¿Qué?");
    }

    /// The sentence, espeak-ng 1.52's output for it as captured on macOS, and
    /// what Kokoro gets.
    const READINGS: &[(&str, &str, &str, &str)] = &[
        (
            "fr-fr",
            "Les enfants sont dans le jardin.",
            "le-z ɑ\u{303}fˈɑ\u{303} sˈɔ\u{303} dɑ\u{303} lə- ʒaʁdˈɛ\u{303}\n",
            "lez ɑ\u{303}fˈɑ\u{303} sˈɔ\u{303} dɑ\u{303} lə ʒaʁdˈɛ\u{303}.",
        ),
        (
            "pt-br",
            "Não, obrigado.",
            "nˈɐ\u{303}\u{200d}ʊ\u{303}\nˌobriɡˈadʊ\n",
            "nˈɐ\u{303}ʊ\u{303}, ˌobriɡˈadʊ.",
        ),
        (
            "hi",
            "मैं iPhone use करता हूँ।",
            "mɛ\u{303} (\u{200d}e\u{200d}n)ˈa\u{200d}ɪ fˈə\u{200d}ʊn jˈuːs(\u{200d}h\u{200d}i) kˈʌɾtˌaː hu\u{303}\n",
            "mɛ\u{303} ˈI fˈQn jˈuːs kˈʌɾtˌaː hu\u{303}.",
        ),
        (
            "hi",
            "आज मौसम बहुत अच्छा है।",
            "ˈaːɟ mˈɔːsəm bˈʌhʊt ˈʌcʰcʰaː hɛː\n",
            "ˈaːɟ mˈɔːsəm bˈʌhʊt ˈʌcʰcʰaː hɛː.",
        ),
        (
            "es",
            "El 3,5 % de 1.000 personas llegó a las 10:30.",
            "el tɾˈes koma θˈinko poɾθjˈɛnto ðe mˈil peɾsˈonas ʎeɣˈo a las ðjˈɛθ tɾˈe\u{200d}ɪnta\n",
            "el tɾˈes koma θˈinko poɾθjˈɛnto ðe mˈil peɾsˈonas ʎeɣˈo a las ðjˈɛθ tɾˈAnta.",
        ),
        (
            "es",
            "Hola, ¿cómo estás?",
            "ˈola\nkˈomo estˈas\n",
            "ˈola, kˈomo estˈas?",
        ),
        ("es", "¡Qué bien!", "kˈe βjˈen\n", "kˈe βjˈen!"),
        // espeak-ng reads no minus sign; the argument is not an option.
        ("es", "-5 grados", "θˈinko ɣɾˈaðos\n", "θˈinko ɣɾˈaðos"),
        (
            "it",
            "Ciao, come stai? Bene, grazie.",
            "t\u{200d}ʃˈao\nkˌome stˈaj\nbˈɛne\nɡrˈat\u{200d}sje\n",
            "ʧˈao, kˌome stˈaj? bˈɛne, ɡrˈaʦje.",
        ),
        (
            "it",
            "Lo zio mangia la pizza.",
            "lo d\u{200d}zˈio mˈand\u{200d}ʒa la pˈit\u{200d}sːa\n",
            "lo ʣˈio mˈanʤa la pˈiʦːa.",
        ),
        (
            "fr-fr",
            "Ce weekend, j'ai regardé Netflix.",
            "sə- (\u{200d}e\u{200d}n)wiːkˈɛnd(\u{200d}f\u{200d}r)\nʒe ʁəɡaʁdˈe nɛtflˈiks\n",
            "sə wiːkˈɛnd, ʒe ʁəɡaʁdˈe nɛtflˈiks.",
        ),
        (
            "es",
            "«¿Vienes?» preguntó.",
            "bjˈenes\npɾˌeɣuntˈo\n",
            "bjˈenes? pɾˌeɣuntˈo.",
        ),
        // espeak-ng does not end a clause at "UU.", before a lower-case word.
        (
            "es",
            "EE. UU. es grande.",
            "ˈee\nˌuˈu ˈes ɣɾˈande\n",
            "ˈee, ˌuˈu ˈes ɣɾˈande.",
        ),
        // It starts one at "¿", which ends nothing.
        (
            "es",
            "Dime ¿qué quieres?",
            "dˈime\nkˈe kjˈeɾes\n",
            "dˈime, kˈe kjˈeɾes?",
        ),
    ];

    #[test]
    fn espeak_ng_output_maps_to_what_kokoro_was_trained_on() {
        for &(_, text, ipa, kokoro) in READINGS {
            let read = sentence_to_kokoro(ipa, &clause_marks(text));
            assert_eq!(read, (kokoro.to_string(), 0), "{text}");
            assert!(kokoro.chars().all(in_vocab), "{text}: {kokoro}");
        }
    }

    #[test]
    fn nasal_vowels_keep_their_tilde() {
        let nasals = "ɑ\u{303} ɔ\u{303} ɛ\u{303} œ\u{303} ɐ\u{303} u\u{303}";
        let read = sentence_to_kokoro(&format!("{nasals}\n"), &clause_marks("x"));
        assert_eq!(read, (nasals.to_string(), 0));
    }

    #[test]
    fn language_flags_go_and_their_words_stay() {
        for (line, without) in [
            (
                "mɛ̃ (\u{200d}e\u{200d}n)ˈa\u{200d}ɪ jˈuːs(\u{200d}h\u{200d}i) hũ",
                "mɛ̃ ˈa\u{200d}ɪ jˈuːs hũ",
            ),
            ("(en)wiːkˈɛnd(fr)", "wiːkˈɛnd"),
            ("(\u{200d}p\u{200d}t\u{200d}-\u{200d}p\u{200d}t)a", "a"),
            // Not flags.
            ("a (b c) d", "a (b c) d"),
            ("a () b (", "a () b ("),
        ] {
            assert_eq!(without_language_flags(line), without, "{line}");
        }
    }

    #[test]
    fn symbols_with_no_token_are_dropped_and_counted() {
        // ʉ and ɵ: vowels Kokoro has no token for.
        let read = sentence_to_kokoro("ʉa ɵ\n", &clause_marks("x."));
        assert_eq!(read, ("a.".to_string(), 2));
        assert_eq!(
            sentence_to_kokoro("\n", &clause_marks("…")),
            (String::new(), 0)
        );
    }

    #[test]
    fn clause_marks_are_read_where_espeak_ng_ends_a_clause() {
        let marks = |text| {
            let found = clause_marks(text);
            (found.marks, found.ends_text)
        };
        let ending = |marks: &[&str]| (marks.iter().map(|m| m.to_string()).collect(), true);
        let inside = |marks: &[&str]| (marks.iter().map(|m| m.to_string()).collect(), false);
        assert_eq!(marks("Hola, ¿cómo estás?"), ending(&[",", "?"]));
        assert_eq!(
            marks("El 3,5 % de 1.000 personas llegó a las 10:30."),
            ending(&["."])
        );
        assert_eq!(marks("«¿Vienes?» preguntó."), ending(&["?", "."]));
        assert_eq!(marks("Él dijo: \"No lo sé.\""), ending(&[":", "."]));
        assert_eq!(marks("dijo: hola"), inside(&[":"]));
        assert_eq!(marks("¿Qué?! Espera…"), ending(&["?!", "…"]));
        assert_eq!(marks("नमस्ते। दुनिया॥"), ending(&[".", "."]));
        assert_eq!(marks("de 1914 – 1918"), inside(&["—"]));
        assert_eq!(marks("well—maybe, 3, 5"), inside(&[",", ","]));
        // Nothing to say before it: no clause.
        assert_eq!(marks(", , hola"), inside(&[]));
        assert_eq!(marks("hola, , adiós"), inside(&[","]));
        assert_eq!(marks("les enfants"), inside(&[]));
    }

    /// The installed espeak-ng; a test that needs it passes without it.
    fn installed() -> Option<PathBuf> {
        let program = find();
        if program.is_none() {
            eprintln!("espeak-ng is not installed, so this test checks nothing");
        }
        program
    }

    #[test]
    fn real_espeak_ng_reads_each_sentence_as_captured() {
        let Some(program) = installed() else {
            return;
        };
        let mismatches: Vec<String> = READINGS
            .iter()
            .filter_map(|&(voice, text, _, kokoro)| {
                let read = Sentences::new(program.clone(), voice).read(text).unwrap();
                (read != (kokoro.to_string(), 0))
                    .then(|| format!("{voice} {text}\n  expected {kokoro}\n  read     {read:?}"))
            })
            .collect();
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }

    #[test]
    fn only_plain_words_are_sent() {
        for word in ["Kubernetes", "O'Neill", "npm", "'tis"] {
            assert!(is_plain_word(word), "{word}");
        }
        for word in [
            "日本",
            "Ελλάδα",
            "-rf",
            "a b",
            &"a".repeat(LONGEST_WORD + 1),
        ] {
            assert!(!is_plain_word(word), "{word}");
        }
    }

    #[test]
    fn found_in_known_locations_then_path() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let on_path = dir.path().join("bin");
        std::fs::create_dir_all(&on_path).unwrap();
        let path_var = Some(std::env::join_paths([&on_path]).unwrap());
        let known = [first.to_str().unwrap(), second.to_str().unwrap()];

        assert_eq!(find_in(&known, path_var.clone()), None);

        let program = on_path.join("espeak-ng");
        make_executable(&program);
        assert_eq!(find_in(&known, path_var.clone()), Some(program));

        make_executable(&second);
        assert_eq!(find_in(&known, path_var.clone()), Some(second.clone()));

        make_executable(&first);
        assert_eq!(find_in(&known, path_var), Some(first));
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_run_is_not_found() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("espeak-ng");
        std::fs::write(&program, b"").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(find_in(&[program.to_str().unwrap()], None), None);
    }

    fn make_executable(path: &Path) {
        std::fs::write(path, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_hangs_is_killed_at_the_timeout() {
        let started = Instant::now();
        let error = run(
            Path::new("/bin/sleep"),
            &["5".to_string()],
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(error.contains("no answer"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_program_is_an_error() {
        let error = run(Path::new("/usr/bin/false"), &[], TIMEOUT).unwrap_err();
        assert!(error.contains("exited"), "{error}");
        assert!(run(Path::new("/nonexistent/espeak-ng"), &[], TIMEOUT).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn output_far_larger_than_a_pipe_holds_is_read_whole() {
        let args = ["-c", "yes | head -c 204800"].map(str::to_string);
        let output = run(Path::new("/bin/sh"), &args, SENTENCE_TIMEOUT).unwrap();
        assert_eq!(output.len(), 204_800);
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_writes_without_end_is_stopped() {
        let started = Instant::now();
        let error = run(Path::new("/usr/bin/yes"), &[], Duration::from_millis(200)).unwrap_err();
        assert!(
            error.contains("no answer") || error.contains("exited"),
            "{error}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_or_slow_call_spells_the_word_as_misaki_does() {
        let text = "Siobhan ran npm on Zxq'vb in 日本.";
        let plain = G2P::new(Dialect::British.language());
        let (expected, _) = plain.g2p(text).unwrap();
        for (program, timeout) in [
            ("/nonexistent/espeak-ng", TIMEOUT),
            ("/usr/bin/false", TIMEOUT),
            // Never exits.
            ("/usr/bin/yes", Duration::from_millis(50)),
        ] {
            let g2p = g2p_with_timeout(Dialect::British, PathBuf::from(program), timeout);
            assert_eq!(g2p.g2p(text).unwrap().0, expected, "{program}");
        }
    }

    /// `cargo test -- --ignored espeak` with espeak-ng installed.
    #[test]
    #[ignore = "needs espeak-ng"]
    fn real_espeak_reads_unknown_names() {
        let program = find().expect("espeak-ng is installed");
        for dialect in [Dialect::British, Dialect::American] {
            let plain = G2P::new(dialect.language());
            let g2p = g2p(dialect, program.clone());
            for word in ["Kubernetes", "Cholmondeley", "Keighley", "Siobhan"] {
                let started = Instant::now();
                let (read, _) = g2p.g2p(word).unwrap();
                let elapsed = started.elapsed();
                let (spelled, _) = plain.g2p(word).unwrap();
                println!("{dialect:?} {word}: {read} in {elapsed:?} (spelled: {spelled})");
                assert!(!read.contains('❓'), "{read}");
                assert_ne!(read.trim(), spelled.trim(), "{word} was spelled out");
                assert!(read.trim().chars().all(in_vocab), "{read}");
            }
        }
    }
}
