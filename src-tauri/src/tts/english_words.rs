//! English words misaki-rs's lexicons lack, which it would spell letter by
//! letter. In technical text most are made of words it has: compounds
//! ("changelog", "filesystem"), a word with a prefix or suffix ("debounce",
//! "tokenizer"), letters and a word ("tsconfig") and identifiers
//! ("setTimeout"), so they are read from those.

use super::text::camel_cut;
use misaki_rs::lexicon::Lexicon;
use misaki_rs::Language;

/// Developer words no rule below reads right, said as these lexicon words
/// run together. The word in capitals is stressed, else the first; a single
/// capital is a letter's name.
const SAID_AS: &[(&str, &str)] = &[
    ("argv", "arg V"),
    ("chown", "C H OWN"),
    ("clippy", "clip E"),
    ("deno", "D no"),
    ("deque", "deck"),
    ("enum", "E numb"),
    ("eslint", "E S LINT"),
    ("impl", "imp L"),
    ("kubectl", "cube CONTROL"),
    ("kubernetes", "coo burr NET ease"),
    ("malloc", "mal lock"),
    ("mkdir", "make deer"),
    ("mutex", "mute ex"),
    ("nginx", "engine X"),
    ("nullptr", "null P T R"),
    ("numpy", "numb pie"),
    ("postgresql", "post gres Q L"),
    ("pytorch", "pie torch"),
    ("rustc", "rust C"),
    ("sudo", "sue doe"),
    ("sysadmin", "sis admin"),
    ("wifi", "why fie"),
];
/// Words misaki-rs reads that may be part of a compound, though not in its
/// gold lexicon, whose other entries make too many false parts.
const DEVELOPER_PARTS: &[&str] = &["dev", "nav", "ops", "std", "sym"];
/// Two-letter words a compound may end in ("plugin", "popup", "sizeof").
/// Others split words that are no compounds ("inode").
const SHORT_ENDINGS: &[&str] = &["in", "of", "up"];
/// Lexicon words that end a word as a suffix far more often than as a word
/// ("act|ions", "red|ist"), so no compound ends in one.
const NOT_LAST: &[&str] = &[
    "ate", "ates", "ion", "ions", "ism", "isms", "ist", "ists", "ite", "ites",
];
/// Unstressed prefixes ("debounce", "rerender", "unregister").
const PREFIXES: &[(&str, &str)] = &[("de", "dˌiː"), ("re", "ɹˌiː"), ("un", "ˌʌn")];
/// Unstressed suffixes, American then British ("hashable", "tokenizer"),
/// after a word that may have lost a final "e" ("parsable").
const SUFFIXES: &[(&str, &str, &str)] =
    &[("able", "əbə\u{200d}l", "əbə\u{200d}l"), ("er", "ɚ", "ə")];
/// Consonants an English word can start with before its first vowel; any
/// other run is letters said by name ("tsconfig", "pkill").
const ONSETS: &[&str] = &[
    "bl", "br", "ch", "chr", "cl", "cr", "dr", "dw", "fl", "fr", "gh", "gl", "gn", "gr", "kl",
    "kn", "kr", "ph", "phr", "pl", "pr", "ps", "rh", "sc", "sch", "scr", "sh", "shr", "sk", "sl",
    "sm", "sn", "sp", "sph", "spl", "spr", "st", "str", "sw", "th", "thr", "tr", "ts", "tw", "wh",
    "wr",
];
/// The shortest word a part or stem can be, short endings apart.
const MIN_PART: usize = 3;
/// The shortest word rewritten unless `SAID_AS` has it: shorter ones are
/// mostly abbreviations and units ("KiB", "PoC").
const SHORTEST_WORD: usize = 5;
/// A longer run of letters is no word (base64, a hash) and is left alone.
const LONGEST_WORD: usize = 32;

/// `text` with each word misaki-rs would spell, and can read from words it
/// has, rewritten: as a `[word](/phonemes/)` override, or an identifier as
/// its words. `[…](…)` overrides already in `text` are kept.
pub(super) fn read_unknown_words(text: &str, lexicon: &Lexicon) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    // After a lone "[", misaki-rs would take it and a new override as one.
    let mut lone_bracket = false;
    while let Some(c) = text[i..].chars().next() {
        let len = match c {
            '[' => override_len(&text[i..]).unwrap_or_else(|| {
                lone_bracket = true;
                1
            }),
            ']' => {
                lone_bracket = false;
                1
            }
            c if c.is_alphabetic() => {
                let len = word_len(&text[i..]);
                let said = (!lone_bracket)
                    .then(|| rewrite(&text[i..i + len], lexicon))
                    .flatten();
                if let Some(said) = said {
                    out.push_str(&text[copied..i]);
                    out.push_str(&said);
                    copied = i + len;
                }
                len
            }
            c => c.len_utf8(),
        };
        i += len;
    }
    out.push_str(&text[copied..]);
    out
}

/// The length of a `[text](feature)` override at the start of `text`, as
/// misaki-rs finds them.
fn override_len(text: &str) -> Option<usize> {
    let close = text.find(']').filter(|&at| at > 1)?;
    let feature = text[close + 1..].strip_prefix('(')?;
    Some(close + 2 + feature.find(')')? + 1)
}

/// The length of the word at the start of `text` as misaki-rs's tokeniser
/// cuts it: letters, with apostrophes between them ("changelog's").
fn word_len(text: &str) -> usize {
    let mut len = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if c.is_alphabetic() {
            len = at + c.len_utf8();
        } else if c != '\'' || !chars.peek().is_some_and(|&(_, next)| next.is_alphabetic()) {
            break;
        }
    }
    len
}

/// What to write for `word`, or `None` to leave it to misaki-rs. Capitals
/// are read only as a compound ("CHANGELOG"): shorter ones and the rest are
/// initialisms, which `spell_out` and misaki-rs spell.
fn rewrite(word: &str, lexicon: &Lexicon) -> Option<String> {
    let (letters, possessive) = match word.strip_suffix("'s") {
        Some(letters) => (letters, true),
        None => (word, false),
    };
    let short = letters.len() < SHORTEST_WORD
        && !SAID_AS
            .iter()
            .any(|(said, _)| said.eq_ignore_ascii_case(letters));
    if short
        || letters.len() > LONGEST_WORD
        || !letters.bytes().all(|b| b.is_ascii_alphabetic())
        || (is_capitals(letters) && letters.len() < SHORTEST_WORD)
        || knows(word, lexicon)
    {
        return None;
    }
    let lower = letters.to_ascii_lowercase();
    let parts = camel_cut(letters);
    let phonemes = if is_capitals(letters) {
        compound(&lower, lexicon)?
    } else if let Some(phonemes) = said(&lower, lexicon) {
        phonemes
    } else if parts.len() > 1 {
        return identifier(&parts, possessive, lexicon);
    } else {
        read(&lower, lexicon)?
    };
    Some(said_as(word, phonemes, possessive, lexicon))
}

fn is_capitals(letters: &str) -> bool {
    letters.len() > 1 && letters.bytes().all(|b| b.is_ascii_uppercase())
}

/// `[word](/phonemes/)`, with an "s" sound for "'s".
fn said_as(word: &str, phonemes: String, possessive: bool, lexicon: &Lexicon) -> String {
    let phonemes = if possessive {
        lexicon.append_s(&phonemes)
    } else {
        phonemes
    };
    format!("[{word}](/{phonemes}/)")
}

/// An identifier's words, or `None` to leave it whole: each one misaki-rs
/// reads, one read here ("getChangelog") or one with no vowel, which it
/// spells ("HttpServer"). Capitals are `spell_out`'s, which left this whole.
fn identifier(parts: &[&str], possessive: bool, lexicon: &Lexicon) -> Option<String> {
    let last = parts.len() - 1;
    let words = parts.iter().enumerate().map(|(n, &part)| {
        let possessive = possessive && n == last;
        let written = if possessive {
            format!("{part}'s")
        } else {
            part.to_string()
        };
        if is_capitals(part) {
            return None;
        }
        if part.len() == 1 || knows(&written, lexicon) {
            return Some(written);
        }
        if let Some(phonemes) = read(&part.to_ascii_lowercase(), lexicon) {
            return Some(said_as(&written, phonemes, possessive, lexicon));
        }
        let no_vowel = !part
            .to_ascii_lowercase()
            .contains(['a', 'e', 'i', 'o', 'u', 'y']);
        no_vowel.then_some(written)
    });
    Some(words.collect::<Option<Vec<_>>>()?.join(" "))
}

/// Whether misaki-rs reads `word` from its lexicons: as written, in lower
/// case, or with a suffix it strips. Generous on purpose, so that no word it
/// reads is ever rewritten; but capitals it has no entry for it spells, as a
/// proper noun or letter by letter, so those count as unknown.
fn knows(word: &str, lexicon: &Lexicon) -> bool {
    let lower = word.to_ascii_lowercase();
    let in_table = |w: &str| lexicon.golds.contains_key(w) || lexicon.silvers.contains_key(w);
    if in_table(word)
        || in_table(&lower)
        || lexicon.stem_s(word, "NN", None, None).is_some()
        || lexicon.stem_ed(word, "NN", None, None).is_some()
        || lexicon.stem_ing(word, "NN", None, None).is_some()
    {
        return true;
    }
    if is_capitals(word.strip_suffix("'s").unwrap_or(word)) {
        return lexicon.get_word(word, "NN", None, None).is_some();
    }
    lexicon.is_known(word, "NN")
        || lexicon.is_known(&lower, "NN")
        || ["NN", "NNP"]
            .into_iter()
            .any(|tag| lexicon.get_word(word, tag, None, None).is_some())
}

/// Phonemes for `word` (lower case), the first way that reads it: as a word
/// `SAID_AS` or misaki-rs has (inside a longer one), a prefix or suffix with
/// a word, a compound, the plural of one of these, or letters said by name
/// before a word.
fn read(word: &str, lexicon: &Lexicon) -> Option<String> {
    if word.len() < MIN_PART {
        return None;
    }
    known(word, lexicon)
        .or_else(|| prefixed(word, lexicon))
        .or_else(|| suffixed(word, lexicon))
        .or_else(|| compound(word, lexicon))
        .or_else(|| plural(word, lexicon))
        .or_else(|| lettered(word, lexicon))
        .map(with_primary)
}

/// `word` as `SAID_AS` or misaki-rs's lexicons have it.
fn known(word: &str, lexicon: &Lexicon) -> Option<String> {
    if word.len() < MIN_PART {
        return None;
    }
    said(word, lexicon).or_else(|| phonemes_of(word, lexicon))
}

fn said(word: &str, lexicon: &Lexicon) -> Option<String> {
    let (_, respelt) = SAID_AS.iter().find(|(written, _)| *written == word)?;
    let parts: Vec<&str> = respelt.split(' ').collect();
    let stressed = parts.iter().position(|part| is_capitals(part)).unwrap_or(0);
    let parts = parts
        .iter()
        .map(|part| match is_capitals(part) {
            true => phonemes_of(&part.to_ascii_lowercase(), lexicon),
            false => phonemes_of(part, lexicon),
        })
        .collect::<Option<Vec<_>>>()?;
    Some(join(parts, stressed, lexicon))
}

fn phonemes_of(word: &str, lexicon: &Lexicon) -> Option<String> {
    lexicon
        .get_word(word, "NN", None, None)
        .map(|(phonemes, _)| phonemes)
}

fn prefixed(word: &str, lexicon: &Lexicon) -> Option<String> {
    PREFIXES.iter().find_map(|&(prefix, said)| {
        let rest = word.strip_prefix(prefix)?;
        let rest = known(rest, lexicon)
            .or_else(|| suffixed(rest, lexicon))
            .or_else(|| compound(rest, lexicon))?;
        Some(with_primary(format!("{said}{rest}")))
    })
}

fn suffixed(word: &str, lexicon: &Lexicon) -> Option<String> {
    let british = lexicon.lang == Language::EnglishGB;
    let stem = |stem: &str| {
        known(stem, lexicon)
            .or_else(|| prefixed(stem, lexicon))
            .or_else(|| compound(stem, lexicon))
    };
    SUFFIXES.iter().find_map(|&(suffix, american, english)| {
        let written = word.strip_suffix(suffix)?;
        let said = stem(written).or_else(|| stem(&format!("{written}e")))?;
        Some(format!(
            "{said}{}",
            if british { english } else { american }
        ))
    })
}

fn plural(word: &str, lexicon: &Lexicon) -> Option<String> {
    let stem = word.strip_suffix('s').filter(|stem| !stem.ends_with('s'))?;
    let phonemes = read(stem, lexicon).or_else(|| read(stem.strip_suffix('e')?, lexicon))?;
    Some(lexicon.append_s(&phonemes))
}

/// Letters said by name before a word: one vowel but "a" or "y" ("E book"),
/// or one or two consonants of a run no English word starts with, before a
/// word or compound ("T S config").
fn lettered(word: &str, lexicon: &Lexicon) -> Option<String> {
    let vowel = |c: &u8| b"aeiouy".contains(c);
    let cluster = word.bytes().take_while(|c| !vowel(c)).count();
    let (leading, compounds) = match cluster {
        0 if !word.starts_with(['a', 'y']) => (1..=1, false),
        2.. if cluster < word.len() && !ONSETS.contains(&&word[..cluster]) => {
            (1..=cluster.min(2), true)
        }
        _ => return None,
    };
    leading.into_iter().find_map(|at| {
        let rest = &word[at..];
        let rest = known(rest, lexicon)
            .or_else(|| compounds.then(|| compound(rest, lexicon)).flatten())?;
        Some(format!("{}{rest}", letters(&word[..at], lexicon)?))
    })
}

/// The names of `letters`, unstressed against the word they are said with.
fn letters(letters: &str, lexicon: &Lexicon) -> Option<String> {
    letters
        .chars()
        .map(|c| {
            let name = phonemes_of(&c.to_ascii_uppercase().to_string(), lexicon)?;
            Some(lexicon.apply_stress(&name, Some(-1.0)))
        })
        .collect()
}

/// Phonemes for `word` as the lexicon words it is made of, run together:
/// two if it can, else three, the longest first word winning unless it is a
/// plural ("time|stamp", not "times|tamp"). English compounds are stressed
/// on their first word ("CHANGElog"), so later words keep a secondary stress.
fn compound(word: &str, lexicon: &Lexicon) -> Option<String> {
    let parts = two_parts(word, lexicon).or_else(|| three_parts(word, lexicon))?;
    Some(join(parts, 0, lexicon))
}

/// `parts` run together, only the `stressed` one keeping its primary stress.
fn join(parts: Vec<String>, stressed: usize, lexicon: &Lexicon) -> String {
    let mut joined = String::new();
    for (n, part) in parts.into_iter().enumerate() {
        if n == stressed {
            joined.push_str(&part);
        } else {
            joined.push_str(&lexicon.apply_stress(&part, Some(-1.0)));
        }
    }
    with_primary(joined)
}

/// Promotes the first secondary stress when nothing has a primary one.
fn with_primary(mut phonemes: String) -> String {
    if !phonemes.contains('ˈ') {
        if let Some(at) = phonemes.find('ˌ') {
            phonemes.replace_range(at..at + 'ˌ'.len_utf8(), "ˈ");
        }
    }
    phonemes
}

fn two_parts(word: &str, lexicon: &Lexicon) -> Option<Vec<String>> {
    let mut plural_first = None;
    for at in (MIN_PART..word.len()).rev() {
        let (first, last) = word.split_at(at);
        let (Some(first_said), Some(last_said)) = (part(first, lexicon), last_part(last, lexicon))
        else {
            continue;
        };
        let parts = vec![first_said, last_said];
        if !is_plural(first, lexicon) {
            return Some(parts);
        }
        plural_first.get_or_insert(parts);
    }
    plural_first
}

fn three_parts(word: &str, lexicon: &Lexicon) -> Option<Vec<String>> {
    (MIN_PART..word.len()).rev().find_map(|at| {
        let first = part(&word[..at], lexicon)?;
        let mut rest = two_parts(&word[at..], lexicon)?;
        rest.insert(0, first);
        Some(rest)
    })
}

fn is_plural(word: &str, lexicon: &Lexicon) -> bool {
    word.strip_suffix('s')
        .is_some_and(|stem| !stem.ends_with('s') && lexicon.golds.contains_key(stem))
}

/// A word that can start a compound: one of the lexicon's own, as written,
/// so no proper name.
fn is_part(word: &str, lexicon: &Lexicon) -> bool {
    word.len() >= MIN_PART && (lexicon.golds.contains_key(word) || DEVELOPER_PARTS.contains(&word))
}

fn part(word: &str, lexicon: &Lexicon) -> Option<String> {
    is_part(word, lexicon).then(|| phonemes_of(word, lexicon))?
}

/// The last word, which may carry an ending ("change|logs", "plug|ins").
fn last_part(word: &str, lexicon: &Lexicon) -> Option<String> {
    let base = |word: &str| SHORT_ENDINGS.contains(&word) || is_part(word, lexicon);
    if NOT_LAST.contains(&word) {
        return None;
    }
    if base(word) || inflection_stems(word).iter().any(|stem| base(stem)) {
        phonemes_of(word, lexicon)
    } else {
        None
    }
}

/// The words `word` may be an inflection of, as misaki-rs strips endings.
fn inflection_stems(word: &str) -> Vec<String> {
    let mut stems = Vec::new();
    let cut = |suffix: &str| word.strip_suffix(suffix).map(str::to_string);
    if !word.ends_with("ss") {
        stems.extend(cut("s"));
    }
    stems.extend(cut("es"));
    stems.extend(cut("ies").map(|stem| stem + "y"));
    if !word.ends_with("dd") {
        stems.extend(cut("d"));
    }
    if !word.ends_with("eed") {
        stems.extend(cut("ed"));
    }
    if let Some(stem) = cut("ing") {
        stems.push(format!("{stem}e"));
        let b = stem.as_bytes();
        if b.len() >= 2 && b[b.len() - 1] == b[b.len() - 2] {
            stems.push(stem[..stem.len() - 1].to_string());
        }
        stems.push(stem);
    }
    stems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ModelVariant;
    use crate::tts::local::{phonemes_without_espeak, ModelFiles};
    use std::path::Path;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    fn lexicons() -> [&'static Lexicon; 2] {
        static BRITISH: OnceLock<Lexicon> = OnceLock::new();
        static AMERICAN: OnceLock<Lexicon> = OnceLock::new();
        [
            BRITISH.get_or_init(|| Lexicon::new(Language::EnglishGB)),
            AMERICAN.get_or_init(|| Lexicon::new(Language::EnglishUS)),
        ]
    }

    /// Kokoro's symbols for each of `texts` as `voice` reads it.
    fn read_by(voice: &str, texts: &[&str]) -> Vec<String> {
        let files = ModelFiles::new(Path::new("models"), ModelVariant::Int8, voice);
        let texts: Vec<String> = texts.iter().map(|text| text.to_string()).collect();
        phonemes_without_espeak(&files, &texts)
    }

    #[test]
    fn unknown_words_read_as_the_words_they_are_made_of() {
        // Text, then `bm_lewis` and `af_alloy`.
        let cases = [
            ("changelog", "ʧˈAnʤlˌɒɡ", "ʧˈAnʤlˌɔɡ"),
            ("filesystem", "fˈIlsˌɪstəm", "fˈIlsˌɪstəm"),
            ("frontend", "fɹˈʌntˌɛnd", "fɹˈʌntˌɛnd"),
            ("dropdown", "dɹˈɒpdˌWn", "dɹˈɑpdˌWn"),
            ("lockfile", "lˈɒkfˌIl", "lˈɑkfˌIl"),
            ("toolchain", "tˈuːlʧˌAn", "tˈulʧˌAn"),
            ("codegen", "kˈQdʤˌɛn", "kˈOdʤˌɛn"),
            ("webview", "wˈɛbvjˌuː", "wˈɛbvjˌu"),
            // Endings.
            ("changelogs", "ʧˈAnʤlˌɒɡz", "ʧˈAnʤlˌɔɡz"),
            ("typechecking", "tˈIpʧˌɛkɪŋ", "tˈIpʧˌɛkɪŋ"),
            (
                "the changelog's date",
                "ðə ʧˈAnʤlˌɒɡz dˈAt",
                "ðə ʧˈAnʤlˌɔɡz dˈAt",
            ),
            ("plugins", "plˈʌɡˌɪnz", "plˈʌɡˌɪnz"),
            // Two-letter last words.
            ("plugin", "plˈʌɡˌɪn", "plˈʌɡˌɪn"),
            ("popup", "pˈɒpʌp", "pˈɑpʌp"),
            ("sizeof", "sˈIzɒv", "sˈIzʌv"),
            // Prefixes and suffixes.
            ("debounce", "dˌiːbˈWns", "dˌibˈWns"),
            ("unregister", "ˌʌnɹˈɛʤɪstə", "ˌʌnɹˈɛʤɪstɚ"),
            ("tokenizer", "tˈQkənˌIzə", "tˈOkənˌIzɚ"),
            ("hashable", "hˈaʃəbᵊl", "hˈæʃəbᵊl"),
            // Letters said by name.
            ("tsconfig", "tˌiːˌɛskˈɒnfˌɪɡ", "tˌiˌɛskˈɑnfˌɪɡ"),
            ("ebook", "ˌiːbˈʊk", "ˌibˈʊk"),
            // Said as `SAID_AS` has them.
            ("sudo", "sˈuːdˌQ", "sˈudˌO"),
            ("nginx", "ˈɛnʤɪnˌɛks", "ˈɛnʤɪnˌɛks"),
            ("kubectl", "kjˌuːbkəntɹˈQl", "kjˌubkəntɹˈOl"),
            ("kubernetes", "kˌuːbˌɜːnˈɛtˌiːz", "kˌubˌɜnˈɛtˌiz"),
            // Identifiers, capitals.
            ("getChangelog", "ɡɛt ʧˈAnʤlˌɒɡ", "ɡɛt ʧˈAnʤlˌɔɡ"),
            ("setTimeout", "sˈɛt tˈImWt", "sˈɛt tˈImWt"),
            (
                "HttpServer",
                "ˈAʧ tˈiː tˈiː pˈiː sˈɜːvə",
                "ˈAʧ tˈi tˈi pˈi sˈɜvɚ",
            ),
            (
                "CHANGELOG.md",
                "ʧˈAnʤlˌɒɡ dˈɒt ˌɛmdˈiː",
                "ʧˈAnʤlˌɔɡ dˈɑt ˌɛmdˈi",
            ),
            // As before: words the lexicons have, initialisms, and words
            // misaki-rs's tokeniser already splits.
            (
                "the API URL",
                "ði ˌApˌiːˈI jˌuːˌɑːˈɛl",
                "ði ˌApˌiˈI jˌuˌɑɹˈɛl",
            ),
            ("npm", "ˈɛn pˈiː ˈɛm", "ˈɛn pˈi ˈɛm"),
            ("user_name", "jˈuːzə nˈAm", "jˈuzɚ nˈAm"),
            ("dry-run", "dɹˈI ɹˈʌn", "dɹˈI ɹˈʌn"),
            ("codebase", "kˈQdbAs", "kˈOdbˌAs"),
            ("keyboard", "kˈiːbɔːd", "kˈibɔɹd"),
            ("timestamp", "tˈImstamp", "tˈImstæmp"),
            ("readme", "ɹˈiːd mˌiː", "ɹˈid mˌi"),
        ];
        let texts: Vec<&str> = cases.iter().map(|&(text, _, _)| text).collect();
        for (voice, read) in [
            ("bm_lewis", read_by("bm_lewis", &texts)),
            ("af_alloy", read_by("af_alloy", &texts)),
        ] {
            for (&(text, british, american), read) in cases.iter().zip(read) {
                let expected = if voice == "bm_lewis" {
                    british
                } else {
                    american
                };
                assert_eq!(read, expected, "{voice}: {text}");
            }
        }
    }

    #[test]
    fn english_in_japanese_and_chinese_reads_unknown_words_too() {
        let read = read_by("jm_kumo", &["changelogをよむ"]);
        assert!(read[0].starts_with("ʧˈAnʤlˌɔɡ"), "{read:?}");
        let read = read_by("zf_xiaobei", &["看changelog"]);
        assert!(read[0].ends_with("ʧˈAnʤlˌɔɡ"), "{read:?}");
    }

    /// Words misaki-rs looks up whole: letters, with apostrophes between.
    fn one_token(word: &str) -> bool {
        word.starts_with(|c: char| c.is_ascii_alphabetic())
            && word.ends_with(|c: char| c.is_ascii_alphabetic())
            && word.chars().all(|c| c.is_ascii_alphabetic() || c == '\'')
    }

    /// Every entry of both lexicons, with the casings misaki-rs reads it in:
    /// a lower-case or capitalised entry also capitalised and in capitals.
    #[test]
    fn no_word_the_lexicons_have_is_rewritten() {
        for lexicon in lexicons() {
            let words = lexicon.golds.keys().chain(lexicon.silvers.keys());
            for word in words.filter(|word| one_token(word)) {
                let lower = word.to_ascii_lowercase();
                let capitalised = lower[..1].to_ascii_uppercase() + &lower[1..];
                let mut written = vec![word.clone()];
                if *word == lower || *word == capitalised {
                    written.extend([capitalised, word.to_ascii_uppercase()]);
                }
                for written in written {
                    assert_eq!(read_unknown_words(&written, lexicon), written);
                }
            }
        }
    }

    #[test]
    fn initialisms_short_words_and_other_scripts_stay_as_written() {
        for lexicon in lexicons() {
            for text in [
                "API",
                "HTTPS",
                "KiB",
                "PoC",
                "npm",
                "utf",
                "日本changelog",
                "changelog'd",
                "XMLHttpRequest",
                "REDIST",
                "[ changelog",
            ] {
                assert_eq!(read_unknown_words(text, lexicon), text);
            }
        }
    }

    #[test]
    fn overrides_already_in_the_text_are_kept() {
        let [british, _] = lexicons();
        let text = "[IDs](/ˌIdˈiːz/) and [changelog](/x/) in the changelog";
        assert_eq!(
            read_unknown_words(text, british),
            "[IDs](/ˌIdˈiːz/) and [changelog](/x/) in the [changelog](/t\u{200d}ʃˈe\u{200d}ɪnd\u{200d}ʒlˌɒɡ/)"
        );
    }

    #[test]
    fn every_said_as_word_reads_in_both_dialects_and_is_unknown_to_one() {
        for (word, _) in SAID_AS {
            assert!(
                lexicons().iter().any(|lexicon| !knows(word, lexicon)),
                "{word} is in both lexicons"
            );
            for lexicon in lexicons() {
                assert!(said(word, lexicon).is_some(), "{word}");
            }
        }
        for part in DEVELOPER_PARTS {
            for lexicon in lexicons() {
                assert!(phonemes_of(part, lexicon).is_some(), "{part}");
            }
        }
    }

    /// xorshift64*, so a failure repeats from its seed.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % n as u64) as usize
        }
    }

    /// `text` as misaki-rs reads it after taking its overrides out.
    fn without_overrides(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find('[') {
            out.push_str(&rest[..at]);
            match override_len(&rest[at..]) {
                Some(len) => {
                    let close = rest[at..].find(']').unwrap();
                    out.push_str(&rest[at + 1..at + close]);
                    rest = &rest[at + len..];
                }
                None => {
                    out.push('[');
                    rest = &rest[at + 1..];
                }
            }
        }
        out + rest
    }

    #[test]
    fn random_text_keeps_its_letters_and_a_second_pass_changes_nothing() {
        const PIECES: &[&str] = &[
            "change",
            "log",
            "CHANGE",
            "LOG",
            "Log",
            "file",
            "system",
            "plug",
            "in",
            "up",
            "de",
            "re",
            "un",
            "bounce",
            "able",
            "er",
            "s",
            "ed",
            "ing",
            "ts",
            "config",
            "e",
            "book",
            "sudo",
            "Http",
            "Server",
            "get",
            "x",
            "q",
            "zz",
            "'s",
            "'",
            "[",
            "]",
            "(",
            ")",
            "/",
            "[ID](/ˌIdˈiː/)",
            "1",
            "8",
            "-",
            "_",
            ".",
            " ",
            " ",
            " ",
            "日本",
            "é",
            "\u{301}",
        ];
        let mut rng = Rng(0x5eed_0e1d);
        for _ in 0..5_000 {
            let text: String = (0..rng.below(24))
                .map(|_| PIECES[rng.below(PIECES.len())])
                .collect();
            for lexicon in lexicons() {
                let read = read_unknown_words(&text, lexicon);
                let letters = |text: &str| without_overrides(text).replace(' ', "");
                assert_eq!(letters(&read), letters(&text), "{text:?} -> {read:?}");
                assert_eq!(read_unknown_words(&read, lexicon), read, "{text:?}");
            }
        }
    }

    #[test]
    fn long_runs_of_letters_and_affixes_end_quickly() {
        let [british, _] = lexicons();
        let started = Instant::now();
        for word in [
            "dedededededededededededededeer",
            "unreunreunreunreunreunreunreable",
            "abcdefghijklmnopqrstuvwxyzabcdef",
            "logloglogloglogloglogloglogloglo",
        ] {
            read_unknown_words(word, british);
        }
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "took {:?}",
            started.elapsed()
        );
    }
}
