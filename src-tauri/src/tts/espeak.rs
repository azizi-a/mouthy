//! Pronunciations from a separately installed espeak-ng for words misaki-rs
//! has no entry for. espeak-ng is GPL-3, so it only ever runs as its own
//! program, given one word on its command line and read back as text; Mouthy
//! never links, downloads or ships it.

use super::local::{in_vocab, Dialect, TIES_COMMON};
use log::{debug, info, warn};
use misaki_rs::fallback::FallbackError;
use misaki_rs::{Fallback, G2P};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// Homebrew's prefixes, checked before `PATH`: an app opened from Finder gets
/// a `PATH` without them.
const KNOWN_LOCATIONS: &[&str] = &["/opt/homebrew/bin/espeak-ng", "/usr/local/bin/espeak-ng"];
/// The longest wait for one word. Speech waits for it, so it stays short.
const TIMEOUT: Duration = Duration::from_millis(500);
const POLL: Duration = Duration::from_millis(1);
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

/// `espeak-ng -q --ipa=3 -v <voice> -- <word>`: no audio, IPA with U+200D
/// ties (the form misaki-rs's lexicons use), the word as one argument.
fn args(dialect: Dialect, word: &str) -> Vec<String> {
    let voice = match dialect {
        Dialect::British => "en-gb",
        Dialect::American => "en-us",
    };
    ["-q", "--ipa=3", "-v", voice, "--", word]
        .map(str::to_string)
        .to_vec()
}

/// Runs `program` without a shell or stdin and returns its output, killing it
/// after `timeout`. Output for one word is far smaller than a pipe's buffer,
/// so the program never blocks on it before exiting.
fn run(program: &Path, args: &[String], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", program.display()))?;
    let deadline = Instant::now() + timeout;
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
    let mut output = String::new();
    child
        .stdout
        .take()
        .ok_or_else(|| "its output was not captured".to_string())?
        .read_to_string(&mut output)
        .map_err(|e| format!("cannot read its output: {e}"))?;
    Ok(output)
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
        let known = dir.path().join("known");
        let on_path = dir.path().join("bin");
        std::fs::create_dir_all(&on_path).unwrap();
        let path_var = Some(std::env::join_paths([&on_path]).unwrap());
        let known_program = known.to_str().unwrap();

        assert_eq!(find_in(&[known_program], path_var.clone()), None);

        let program = on_path.join("espeak-ng");
        make_executable(&program);
        assert_eq!(find_in(&[known_program], path_var.clone()), Some(program));

        make_executable(&known);
        assert_eq!(find_in(&[known_program], path_var), Some(known));
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
