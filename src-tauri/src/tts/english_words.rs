//! English words misaki-rs's lexicons lack, which it would spell letter by
//! letter. In technical text most are made of words it has: compounds
//! ("changelog"), a word with a prefix or suffix ("debounce", "tokenizer"),
//! letters and a word ("tsconfig") and identifiers ("setTimeout"), so they are
//! read from those.

use super::text::{camel_cut, said_as_word};
use misaki_rs::lexicon::{Lexicon, PhonemeEntry};
use misaki_rs::Language;

/// Who reads a capitalised word misaki-rs's lexicons lack: this module, or
/// misaki-rs's espeak-ng fallback when the user turned it on, since espeak-ng
/// reads names better than their parts do ("Patton", "Kingsley").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Names {
    Here,
    Espeak,
}

/// Developer words no rule below reads right, said as these lexicon words
/// run together. The word in capitals is stressed, else the first; a single
/// capital is a letter's name.
const RESPELLED: &[(&str, &str)] = &[
    ("argparse", "arg parse"),
    ("argv", "arg V"),
    ("chown", "C H OWN"),
    ("clippy", "clip E"),
    ("deno", "D no"),
    ("deque", "deck"),
    ("emacs", "E max"),
    ("enum", "E numb"),
    ("eslint", "E S LINT"),
    ("fastapi", "fast A P I"),
    ("foreach", "for each"),
    ("graphql", "graph Q L"),
    ("hashtable", "hash table"),
    ("impl", "imp L"),
    ("kubectl", "cube CONTROL"),
    ("kubernetes", "coo burr NET ease"),
    ("malloc", "mal lock"),
    ("matplotlib", "mat plot lib"),
    ("mkdir", "make deer"),
    ("multiline", "multi line"),
    ("mutex", "mute ex"),
    ("nginx", "engine X"),
    ("nullptr", "null P T R"),
    ("numpy", "numb pie"),
    ("oncall", "on call"),
    ("openapi", "open A P I"),
    ("postgresql", "post gres Q L"),
    ("println", "print line"),
    ("protobuf", "proto buff"),
    ("pytorch", "pie torch"),
    ("rustc", "rust C"),
    ("segfault", "seg fault"),
    ("sudo", "sue doe"),
    ("sysadmin", "sis admin"),
    ("upsert", "up sert"),
    ("userid", "user I D"),
    ("venv", "V env"),
    ("vscode", "V S code"),
    ("wifi", "why fie"),
    ("xcode", "X code"),
];
/// Words misaki-rs reads that may be part of a compound, though not in its
/// gold lexicon, whose other entries make too many false parts.
const DEVELOPER_PARTS: &[&str] = &["dev", "nav", "ops", "std", "sym"];
/// Two-letter words a compound may end in ("plugin", "popup", "sizeof").
/// Others split words that are no compounds ("inode").
const SHORT_ENDINGS: &[&str] = &["in", "of", "up"];
/// Lexicon words that end a word as a suffix far more often than as a word
/// ("act|ions", "red|ist"; British "-ley" names would read "lay").
const NOT_LAST: &[&str] = &[
    "ate", "ates", "ion", "ions", "ism", "isms", "ist", "ists", "ite", "ites", "ley",
];
/// Unstressed prefixes ("debounce", "rerender", "unregister").
const PREFIXES: &[(&str, &str)] = &[("de", "dˌiː"), ("re", "ɹˌiː"), ("un", "ˌʌn")];
/// Unstressed suffixes, American then British ("hashable", "tokenizer",
/// "Kingsley"), after a word that may have lost a final "e" ("parsable").
const SUFFIXES: &[(&str, &str, &str)] = &[
    ("able", "əbə\u{200d}l", "əbə\u{200d}l"),
    ("er", "ɚ", "ə"),
    ("ley", "li", "li"),
];
/// Consonants an English word can start with before its first vowel; any
/// other run is letters said by name ("tsconfig", "pkill").
const ONSETS: &[&str] = &[
    "bl", "br", "ch", "chr", "cl", "cr", "dr", "dw", "fl", "fr", "gh", "gl", "gn", "gr", "kl",
    "kn", "kr", "ph", "phr", "pl", "pr", "ps", "rh", "sc", "sch", "scr", "sh", "shr", "sk", "sl",
    "sm", "sn", "sp", "sph", "spl", "spr", "st", "str", "sw", "th", "thr", "tr", "ts", "tw", "wh",
    "wr",
];
/// misaki-rs's vowel symbols.
const VOWELS: &str = "AIOQWYaiuæɑɒɔəɛɜɪʊʌᵻ";
/// The shortest word a part or stem can be, short endings apart.
const MIN_PART: usize = 3;
/// The shortest word rewritten unless `RESPELLED` has it: shorter ones are
/// mostly abbreviations and units ("KiB", "PoC").
const SHORTEST_WORD: usize = 5;
/// A longer run of letters is no word (base64, a hash) and is left alone.
const LONGEST_WORD: usize = 32;
/// An identifier's part this short, or with no vowel, is an initialism
/// ("Url", "Http"), as `spell_out` reads capitals.
const INITIALISM_LETTERS: usize = 4;

/// `text` with each word misaki-rs would spell, and can read from words it
/// has, rewritten: as a `[word](/phonemes/)` override, or an identifier as
/// its words. `[…](…)` overrides already in `text` are kept.
pub(super) fn read_unknown_words(text: &str, lexicon: &Lexicon, names: Names) -> String {
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
            // A "](" nothing closes would take in the first override written after it.
            ']' if text[i + 1..].starts_with('(') && !text[i + 1..].contains(')') => break,
            ']' => {
                lone_bracket = lone_bracket && text[i + 1..].starts_with('(');
                1
            }
            c if c.is_alphabetic() => {
                let len = word_len(&text[i..]);
                let said = (!lone_bracket)
                    .then(|| said_at(text, i, len, lexicon, names))
                    .flatten();
                match said {
                    Some((end, said)) => {
                        out.push_str(&text[copied..i]);
                        out.push_str(&said);
                        copied = end;
                        end - i
                    }
                    None => len,
                }
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

/// What to write for the word `len` long at `i` of `text`, and where the text
/// it replaces ends.
fn said_at(
    text: &str,
    i: usize,
    len: usize,
    lexicon: &Lexicon,
    names: Names,
) -> Option<(usize, String)> {
    let word = &text[i..i + len];
    if let Some(end) = broken_word(text, i, len) {
        if let Some(said) = rejoin(word, &text[i + len + 1..end], lexicon, names) {
            return Some((end, said));
        }
    }
    // Cut into words, it would leave a new word beside the hyphen to rejoin.
    let hyphened = hyphen_before(&text[..i]) || hyphen_after(&text[i + len..]);
    if hyphened && camel_cut(word).len() > 1 {
        return None;
    }
    Some((i + len, rewrite(word, lexicon, names)?))
}

/// Whether `before` ends with a hyphen after a word: a letter, or the end of
/// an override.
fn hyphen_before(before: &str) -> bool {
    let mut chars = before.chars().rev();
    chars.next() == Some('-') && chars.next().is_some_and(|c| c.is_alphabetic() || c == ')')
}

/// Whether `after` starts with a hyphen before a word: a letter, or an
/// override.
fn hyphen_after(after: &str) -> bool {
    after
        .strip_prefix('-')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_alphabetic() || c == '['))
}

/// Where a word `len` long at `i` and the one after a hyphen end, when they
/// are two words joined by one hyphen ("imple-mentation", not "of" in
/// "state-of-the-art").
fn broken_word(text: &str, i: usize, len: usize) -> Option<usize> {
    if hyphen_before(&text[..i]) {
        return None;
    }
    let second = word_len(text[i + len..].strip_prefix('-')?);
    let end = i + len + 1 + second;
    (second > 0 && !hyphen_after(&text[end..])).then_some(end)
}

/// A word broken across lines at a hyphen ("imple-mentation", "change-log"),
/// read whole when a half is no word misaki-rs has; a compound of words it
/// has ("well-known") stays as written.
fn rejoin(first: &str, second: &str, lexicon: &Lexicon, names: Names) -> Option<String> {
    let whole = format!("{first}{second}");
    if !whole.bytes().all(|b| b.is_ascii_alphabetic())
        || !(SHORTEST_WORD..=LONGEST_WORD).contains(&whole.len())
        || (misaki_reads(first, lexicon) && misaki_reads(second, lexicon))
        || camel_cut(&whole).len() > 1
    {
        return None;
    }
    let phonemes = if misaki_reads(&whole, lexicon) {
        phonemes_of(&whole, lexicon)?
    } else {
        word_phonemes(&whole, lexicon, names)?
    };
    Some(format!("[{first}-{second}](/{phonemes}/)"))
}

/// What to write for `word`, or `None` to leave it to misaki-rs. "Mc" names
/// stay whole, as `spell_out` keeps them.
fn rewrite(word: &str, lexicon: &Lexicon, names: Names) -> Option<String> {
    let (letters, possessive) = match word.strip_suffix("'s") {
        Some(letters) => (letters, true),
        None => (word, false),
    };
    let respelled = RESPELLED
        .iter()
        .any(|(written, _)| written.eq_ignore_ascii_case(letters));
    if (letters.len() < SHORTEST_WORD && !respelled)
        || letters.len() > LONGEST_WORD
        || !letters.bytes().all(|b| b.is_ascii_alphabetic())
        || (all_capitals(letters) && letters.len() < SHORTEST_WORD)
        || letters.starts_with("Mc")
        || misaki_reads(word, lexicon)
    {
        return None;
    }
    let parts = camel_cut(letters);
    if parts.len() > 1 && !respelled && !all_capitals(letters) {
        return Some(identifier(&parts, possessive, lexicon));
    }
    let phonemes = word_phonemes(letters, lexicon, names)?;
    Some(said_as(word, phonemes, possessive, lexicon))
}

/// Phonemes for one word misaki-rs lacks. Capitals are read only as a
/// compound ("CHANGELOG"): the rest are initialisms, which `spell_out` and
/// misaki-rs spell.
fn word_phonemes(letters: &str, lexicon: &Lexicon, names: Names) -> Option<String> {
    let lower = letters.to_ascii_lowercase();
    let capitalised = letters.starts_with(|c: char| c.is_ascii_uppercase());
    if all_capitals(letters) {
        compound(&lower, lexicon)
    } else if let Some(phonemes) = respell(&lower, lexicon) {
        Some(phonemes)
    } else if capitalised && names == Names::Espeak {
        None
    } else {
        // Letters said by name read names wrongly ("Z hang", "P fister").
        read(&lower, lexicon, !capitalised)
    }
}

fn all_capitals(letters: &str) -> bool {
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

/// An identifier's words, each one misaki-rs reads as written, the others
/// read by `identifier_part`, so one unknown part never spells the whole.
fn identifier(parts: &[&str], possessive: bool, lexicon: &Lexicon) -> String {
    let last = parts.len() - 1;
    let words: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(n, &part)| {
            let possessive = possessive && n == last;
            let written = if possessive {
                format!("{part}'s")
            } else {
                part.to_string()
            };
            if part.len() == 1 || misaki_reads(&written, lexicon) {
                return written;
            }
            match identifier_part(part, lexicon) {
                Some(phonemes) => said_as(&written, phonemes, possessive, lexicon),
                None => written,
            }
        })
        .collect();
    words.join(" ")
}

/// A part of an identifier misaki-rs cannot read: respelled, then as
/// `spell_out` reads it in capitals when it is short or has no vowel ("Json"
/// "jason", "Url" spelled), else read here, else spelled.
fn identifier_part(part: &str, lexicon: &Lexicon) -> Option<String> {
    let lower = part.to_ascii_lowercase();
    let capitals = part.to_ascii_uppercase();
    if let Some(phonemes) = respell(&lower, lexicon) {
        return Some(phonemes);
    }
    if let Some(said) = said_as_word(&capitals) {
        return respelling(said, lexicon);
    }
    let short = part.len() <= INITIALISM_LETTERS || !lower.contains(|c| "aeiouy".contains(c));
    if short && lexicon.golds.contains_key(&capitals) {
        return phonemes_of(&capitals, lexicon);
    }
    let lower_first = part.starts_with(|c: char| c.is_ascii_lowercase());
    (!short)
        .then(|| read(&lower, lexicon, lower_first))
        .flatten()
        .or_else(|| initialism(&capitals, lexicon))
}

/// `letters` (capitals) said as one initialism, as misaki-rs's `get_nnp` says
/// a proper noun it spells: every stress secondary, then the last primary.
pub(super) fn initialism(letters: &str, lexicon: &Lexicon) -> Option<String> {
    let names: String = letters
        .chars()
        .map(|c| match lexicon.golds.get(&c.to_string()) {
            Some(PhonemeEntry::Simple(name)) => Some(name.as_str()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let mut phonemes = lexicon.apply_stress(&names, Some(0.0));
    if let Some(at) = phonemes.rfind('ˌ') {
        phonemes.replace_range(at..at + 'ˌ'.len_utf8(), "ˈ");
    }
    Some(phonemes)
}

/// Whether misaki-rs reads `word` from its lexicons: as written, in lower
/// case, or with a suffix it strips. Generous on purpose, so that no word it
/// reads is ever rewritten; but capitals it has no entry for it spells, as a
/// proper noun or letter by letter, so those count as unknown.
fn misaki_reads(word: &str, lexicon: &Lexicon) -> bool {
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
    if all_capitals(word.strip_suffix("'s").unwrap_or(word)) {
        return lexicon.get_word(word, "NN", None, None).is_some();
    }
    lexicon.is_known(word, "NN")
        || lexicon.is_known(&lower, "NN")
        || ["NN", "NNP"]
            .into_iter()
            .any(|tag| lexicon.get_word(word, tag, None, None).is_some())
}

/// Phonemes for `word` (lower case), the first way that reads it: as a word
/// `RESPELLED` or misaki-rs has (inside a longer one), a prefix or suffix
/// with a word, a compound, the plural of one of these, or, with
/// `letters_first`, letters said by name before a word.
fn read(word: &str, lexicon: &Lexicon, letters_first: bool) -> Option<String> {
    if word.len() < MIN_PART {
        return None;
    }
    known(word, lexicon)
        .or_else(|| prefixed(word, lexicon, true))
        .or_else(|| suffixed(word, lexicon, true))
        .or_else(|| compound(word, lexicon))
        .or_else(|| plural(word, lexicon, letters_first))
        .or_else(|| letters_first.then(|| lettered(word, lexicon)).flatten())
        .map(with_primary)
}

/// `word` as `RESPELLED` or misaki-rs's lexicons have it.
fn known(word: &str, lexicon: &Lexicon) -> Option<String> {
    if word.len() < MIN_PART {
        return None;
    }
    respell(word, lexicon).or_else(|| phonemes_of(word, lexicon))
}

fn respell(word: &str, lexicon: &Lexicon) -> Option<String> {
    let (_, said) = RESPELLED.iter().find(|(written, _)| *written == word)?;
    respelling(said, lexicon)
}

/// Lexicon words run together as one word, the one in capitals stressed.
fn respelling(said: &str, lexicon: &Lexicon) -> Option<String> {
    let words: Vec<String> = said
        .split(' ')
        .map(|word| {
            if all_capitals(word) {
                word.to_ascii_lowercase()
            } else {
                word.to_string()
            }
        })
        .collect();
    let stressed = said.split(' ').position(all_capitals).unwrap_or(0);
    let parts = words
        .iter()
        .map(|word| Some((word.as_str(), phonemes_of(word, lexicon)?)))
        .collect::<Option<Vec<_>>>()?;
    Some(join(&parts, stressed, lexicon))
}

fn phonemes_of(word: &str, lexicon: &Lexicon) -> Option<String> {
    lexicon
        .get_word(word, "NN", None, None)
        .map(|(phonemes, _)| phonemes)
}

/// A prefix and a word; with `nested`, the word may take a suffix.
fn prefixed(word: &str, lexicon: &Lexicon, nested: bool) -> Option<String> {
    PREFIXES.iter().find_map(|&(prefix, said)| {
        let rest = word.strip_prefix(prefix)?;
        let rest = known(rest, lexicon)
            .or_else(|| nested.then(|| suffixed(rest, lexicon, false)).flatten())
            .or_else(|| compound(rest, lexicon))?;
        Some(format!("{said}{}", with_stress(&rest, lexicon)))
    })
}

/// A word and a suffix; with `nested`, the word may take a prefix.
fn suffixed(word: &str, lexicon: &Lexicon, nested: bool) -> Option<String> {
    let british = lexicon.lang == Language::EnglishGB;
    let stem = |stem: &str| {
        known(stem, lexicon)
            .or_else(|| nested.then(|| prefixed(stem, lexicon, false)).flatten())
            .or_else(|| compound(stem, lexicon))
    };
    SUFFIXES.iter().find_map(|&(suffix, american, english)| {
        let written = word.strip_suffix(suffix)?;
        let said = stem(written).or_else(|| stem(&format!("{written}e")))?;
        Some(format!(
            "{}{}",
            with_stress(&said, lexicon),
            if british { english } else { american }
        ))
    })
}

fn plural(word: &str, lexicon: &Lexicon, letters_first: bool) -> Option<String> {
    let stem = word.strip_suffix('s').filter(|stem| !stem.ends_with('s'))?;
    let phonemes = read(stem, lexicon, letters_first)
        .or_else(|| read(stem.strip_suffix('e')?, lexicon, letters_first))?;
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

/// Phonemes for `word` as two lexicon words run together, the longest first
/// word winning unless it is a plural ("time|stamp", not "times|tamp").
/// English compounds are stressed on their first word ("CHANGElog").
fn compound(word: &str, lexicon: &Lexicon) -> Option<String> {
    let mut plural_first = None;
    for at in (MIN_PART..word.len()).rev() {
        let (first, last) = word.split_at(at);
        let (Some(first_said), Some(last_said)) = (part(first, lexicon), last_part(last, lexicon))
        else {
            continue;
        };
        let parts = [(first, first_said), (last, last_said)];
        if !is_plural(first, lexicon) {
            return Some(join(&parts, 0, lexicon));
        }
        plural_first.get_or_insert(parts);
    }
    plural_first.map(|parts| join(&parts, 0, lexicon))
}

/// Parts, each as written and as said, run together: only the `stressed`
/// one keeps a primary stress, and a letter written twice across a join is
/// said once ("Pat|ton", "mal lock"), though "date|time" keeps both.
fn join(parts: &[(&str, String)], stressed: usize, lexicon: &Lexicon) -> String {
    let mut joined = String::new();
    let mut last_letter: Option<char> = None;
    for (n, (written, said)) in parts.iter().enumerate() {
        let said = if n == stressed {
            with_stress(said, lexicon)
        } else {
            lexicon.apply_stress(said, Some(-1.0))
        };
        let doubled = last_letter
            .zip(written.chars().next())
            .is_some_and(|(end, start)| end.eq_ignore_ascii_case(&start));
        let repeated = joined
            .chars()
            .last()
            .filter(|end| doubled && !VOWELS.contains(*end) && said.starts_with(*end));
        match repeated {
            Some(sound) => joined.push_str(&said[sound.len_utf8()..]),
            None => joined.push_str(&said),
        }
        last_letter = written.chars().last();
    }
    with_primary(joined)
}

/// `phonemes` with a primary stress: its first secondary promoted, or one on
/// its first vowel when it has none ("hath" in "Hathaway").
fn with_stress(phonemes: &str, lexicon: &Lexicon) -> String {
    if phonemes.contains(['ˈ', 'ˌ']) {
        with_primary(phonemes.to_string())
    } else {
        lexicon.apply_stress(phonemes, Some(2.0))
    }
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
    use misaki_rs::G2P;
    use std::path::Path;
    use std::sync::OnceLock;

    fn g2ps() -> [&'static G2P; 2] {
        static BRITISH: OnceLock<G2P> = OnceLock::new();
        static AMERICAN: OnceLock<G2P> = OnceLock::new();
        [
            BRITISH.get_or_init(|| G2P::new(Language::EnglishGB)),
            AMERICAN.get_or_init(|| G2P::new(Language::EnglishUS)),
        ]
    }

    fn lexicons() -> [&'static Lexicon; 2] {
        g2ps().map(|g2p| &g2p.lexicon)
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
            ("toolchain", "tˈuːlʧˌAn", "tˈulʧˌAn"),
            ("webview", "wˈɛbvjˌuː", "wˈɛbvjˌu"),
            ("changelogs", "ʧˈAnʤlˌɒɡz", "ʧˈAnʤlˌɔɡz"),
            ("typechecking", "tˈIpʧˌɛkɪŋ", "tˈIpʧˌɛkɪŋ"),
            (
                "the changelog's date",
                "ðə ʧˈAnʤlˌɒɡz dˈAt",
                "ðə ʧˈAnʤlˌɔɡz dˈAt",
            ),
            ("plugin", "plˈʌɡˌɪn", "plˈʌɡˌɪn"),
            ("plugins", "plˈʌɡˌɪnz", "plˈʌɡˌɪnz"),
            ("popup", "pˈɒpʌp", "pˈɑpʌp"),
            (
                "CHANGELOG.md",
                "ʧˈAnʤlˌɒɡ dˈɒt ˌɛmdˈiː",
                "ʧˈAnʤlˌɔɡ dˈɑt ˌɛmdˈi",
            ),
            // The first part stressed though its entry has no mark.
            ("Hathaway", "hˈaθɐwˌA", "hˈæθɐwˌA"),
            ("forloop", "fˈɔːlˌuːp", "fˈɔɹlˌup"),
            ("fromstring", "fɹˈɒmstɹˌɪŋ", "fɹˈʌmstɹˌɪŋ"),
            // A letter doubled across the join is one sound; "date|time" is two.
            ("Patton", "pˈatˌʌn", "pˈætˌʌn"),
            ("Reddit", "ɹˈɛdˌɪt", "ɹˈɛdˌɪt"),
            ("malloc", "mˈalˌɒk", "mˈælˌɑk"),
            ("datetime", "dˈAttˌIm", "dˈAttˌIm"),
            // Names whose parts read them.
            ("Peterson", "pˈiːtəsˌʌn", "pˈiɾɚsˌʌn"),
            ("Cloudflare", "klˈWdflˌɛː", "klˈWdflˌɛɹ"),
            ("Supabase", "sˈʌpɐbˌAs", "sˈʌpɐbˌAs"),
            ("Kingsley", "kˈɪŋzli", "kˈɪŋzli"),
            ("Ashley", "ˈaʃli", "ˈæʃli"),
            ("Hadley", "hˈadli", "hˈædli"),
            // Prefixes and suffixes, one nested.
            ("debounce", "dˌiːbˈWns", "dˌibˈWns"),
            ("unregister", "ˌʌnɹˈɛʤɪstə", "ˌʌnɹˈɛʤɪstɚ"),
            ("tokenizer", "tˈQkənˌIzə", "tˈOkənˌIzɚ"),
            ("hashable", "hˈaʃəbᵊl", "hˈæʃəbᵊl"),
            ("deserializer", "dˌiːsˈɪəɹiᵊlˌIzə", "dˌisˈɪɹiᵊlˌIzɚ"),
            // Letters said by name, in lower case only.
            ("tsconfig", "tˌiːˌɛskˈɒnfˌɪɡ", "tˌiˌɛskˈɑnfˌɪɡ"),
            ("ebook", "ˌiːbˈʊk", "ˌibˈʊk"),
            // Respelled.
            ("sudo", "sˈuːdˌQ", "sˈudˌO"),
            ("kubectl", "kjˌuːbkəntɹˈQl", "kjˌubkəntɹˈOl"),
            ("kubernetes", "kˌuːbˌɜːnˈɛtˌiːz", "kˌubˌɜnˈɛtˌiz"),
            ("hashtable", "hˈaʃtˌAbᵊl", "hˈæʃtˌAbᵊl"),
            ("userid", "jˈuːzəˌIdˌiː", "jˈuzɚˌIdˌi"),
            ("foreach", "fˈɔːˌiːʧ", "fˈɔɹˌiʧ"),
            ("Xcode", "ˈɛkskˌQd", "ˈɛkskˌOd"),
            // Identifiers: each part read, an initialism as one.
            ("getChangelog", "ɡɛt ʧˈAnʤlˌɒɡ", "ɡɛt ʧˈAnʤlˌɔɡ"),
            ("getUrl", "ɡɛt jˌuːˌɑːˈɛl", "ɡɛt jˌuˌɑɹˈɛl"),
            ("ApiClient", "ˌApˌiːˈI klˈIənt", "ˌApˌiˈI klˈIənt"),
            ("parseJson", "pˈɑːs ʤˈAsᵊn", "pˈɑɹs ʤˈAsᵊn"),
            ("HttpServer", "ˌAʧtˌiːtˌiːpˈiː sˈɜːvə", "ˌAʧtˌitˌipˈi sˈɜvɚ"),
            (
                "UrlSearchParams",
                "jˌuːˌɑːˈɛl sˈɜːʧ pˈaɹəmz",
                "jˌuˌɑɹˈɛl sˈɜʧ pˈæɹəmz",
            ),
            // As before.
            (
                "the API URL",
                "ði ˌApˌiːˈI jˌuːˌɑːˈɛl",
                "ði ˌApˌiˈI jˌuˌɑɹˈɛl",
            ),
            ("npm", "ˈɛn pˈiː ˈɛm", "ˈɛn pˈi ˈɛm"),
            ("user_name", "jˈuːzə nˈAm", "jˈuzɚ nˈAm"),
            ("codebase", "kˈQdbAs", "kˈOdbˌAs"),
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

    #[test]
    fn a_word_broken_at_a_hyphen_reads_whole() {
        // Text, then `bm_lewis` and `af_alloy`.
        let cases = [
            ("imple-mentation", "ˌɪmplɪməntˈAʃən", "ˌɪmplɪməntˈAʃən"),
            (
                "imple\u{2010}mentation",
                "ˌɪmplɪməntˈAʃən",
                "ˌɪmplɪməntˈAʃən",
            ),
            ("chan-gelog", "ʧˈAnʤlˌɒɡ", "ʧˈAnʤlˌɔɡ"),
        ];
        let texts: Vec<&str> = cases.iter().map(|&(text, _, _)| text).collect();
        let british = read_by("bm_lewis", &texts);
        let american = read_by("af_alloy", &texts);
        for (i, &(text, gb, us)) in cases.iter().enumerate() {
            assert_eq!(
                (british[i].as_str(), american[i].as_str()),
                (gb, us),
                "{text}"
            );
        }
        // Halves that are both words stay apart, as "re-sign" must.
        for lexicon in lexicons() {
            for text in [
                "well-known",
                "state-of-the-art",
                "end-of-life",
                "re-sign",
                "con-figuration",
                "change-log",
                "co-op",
            ] {
                assert_eq!(read_unknown_words(text, lexicon, Names::Here), text);
            }
        }
    }

    #[test]
    fn names_their_parts_misread_stay_spelled() {
        for lexicon in lexicons() {
            for name in [
                "Zhang",
                "Zheng",
                "Pfister",
                "Szabo",
                "Dlamini",
                "Gallagher",
                "Rockefeller",
                "Tottenham",
                "Mussorgsky",
            ] {
                assert_eq!(read_unknown_words(name, lexicon, Names::Here), name);
            }
        }
    }

    #[test]
    fn with_espeak_ng_on_capitalised_words_are_left_to_it() {
        let text = "Patton and Kingsley read the changelog in Xcode with getUrl.";
        for lexicon in lexicons() {
            let read = read_unknown_words(text, lexicon, Names::Espeak);
            assert!(
                read.starts_with("Patton and Kingsley read the [changelog]("),
                "{read}"
            );
            assert!(
                read.contains("[Xcode](") && read.contains("get [Url]("),
                "{read}"
            );
            let here = read_unknown_words(text, lexicon, Names::Here);
            assert!(here.starts_with("[Patton]("), "{here}");
        }
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
                    assert_eq!(read_unknown_words(&written, lexicon, Names::Here), written);
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
                "REDIST",
                "McChangelog",
                "[ changelog",
                "[x](]changelog",
                "sign[x](/zx]sdesCHANGE",
            ] {
                assert_eq!(read_unknown_words(text, lexicon, Names::Here), text);
            }
        }
    }

    #[test]
    fn overrides_already_in_the_text_are_kept() {
        let [british, _] = lexicons();
        let text = "[IDs](/ˌIdˈiːz/) and [changelog](/x/) in the changelog";
        assert_eq!(
            read_unknown_words(text, british, Names::Here),
            "[IDs](/ˌIdˈiːz/) and [changelog](/x/) in the [changelog](/t\u{200d}ʃˈe\u{200d}ɪnd\u{200d}ʒlˌɒɡ/)"
        );
    }

    #[test]
    fn every_respelling_reads_in_both_dialects_and_is_unknown_to_one() {
        for (word, _) in RESPELLED {
            assert!(
                lexicons()
                    .iter()
                    .any(|lexicon| !misaki_reads(word, lexicon)),
                "{word} is in both lexicons"
            );
            for lexicon in lexicons() {
                assert!(respell(word, lexicon).is_some(), "{word}");
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
            "ley",
            "s",
            "ed",
            "ing",
            "ts",
            "config",
            "e",
            "book",
            "sudo",
            "Http",
            "Url",
            "Json",
            "Server",
            "get",
            "Pat",
            "ton",
            "x",
            "q",
            "zz",
            "'s",
            "'",
            "[",
            "]",
            "(",
            ")",
            "](",
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
            for g2p in g2ps() {
                for names in [Names::Here, Names::Espeak] {
                    let read = read_unknown_words(&text, &g2p.lexicon, names);
                    // What misaki-rs reads once it has taken the overrides out.
                    let letters = |text: &str| g2p.preprocess_links(text).0.replace(' ', "");
                    assert_eq!(letters(&read), letters(&text), "{text:?} -> {read:?}");
                    assert_eq!(
                        read_unknown_words(&read, &g2p.lexicon, names),
                        read,
                        "{text:?}"
                    );
                }
            }
        }
    }
}
