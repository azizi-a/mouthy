//! Japanese as Kokoro's Japanese voices were trained to read it: misaki's port
//! of cutlet's kana table (`misaki/cutlet.py`, from polm/cutlet, MIT), applied
//! to the kana as written.
//!
//! misaki gets each word and its kana reading from a dictionary; this build has
//! none, so `words` stands in for it: a word is a run of one script, kanji are
//! skipped, and は, へ and long vowels read as they are spelled.

use unicode_normalization::UnicodeNormalization;

/// Longer sentences are cut before this. Kana prose runs about 1.8 ids a
/// character, so its chunks fit one inference; numbers, runs of `き`-like kana
/// and alternating scripts can pass 510 ids, and `split_tokens` then cuts the
/// chunk, mid-word where it has no pause or space.
pub(super) const MAX_CHUNK_CHARS: usize = 180;

/// Above this share of the text's letters in kanji, so much would be skipped
/// that it is refused instead.
const MAX_KANJI_PERCENT: usize = 20;

const KANJI_REFUSAL: &str = "Japanese text with kanji is not supported yet";

/// Kana as cutlet reads it, hiragana only: katakana is folded first. Pairs are
/// a kana and the small kana after it.
#[rustfmt::skip]
const KANA: &[(&str, &str)] = &[
    ("ぁ", "a"), ("あ", "a"), ("ぃ", "i"), ("い", "i"), ("ぅ", "ɯ"), ("う", "ɯ"),
    ("ぇ", "e"), ("え", "e"), ("ぉ", "o"), ("お", "o"), ("か", "ka"), ("が", "ɡa"),
    ("き", "kʲi"), ("ぎ", "ɡʲi"), ("く", "kɯ"), ("ぐ", "ɡɯ"), ("け", "ke"), ("げ", "ɡe"),
    ("こ", "ko"), ("ご", "ɡo"), ("さ", "sa"), ("ざ", "ʣa"), ("し", "ɕi"), ("じ", "ʥi"),
    ("す", "sɨ"), ("ず", "zɨ"), ("せ", "se"), ("ぜ", "ʣe"), ("そ", "so"), ("ぞ", "ʣo"),
    ("た", "ta"), ("だ", "da"), ("ち", "ʨi"), ("ぢ", "ʥi"), ("つ", "ʦɨ"), ("づ", "zɨ"),
    ("て", "te"), ("で", "de"), ("と", "to"), ("ど", "do"), ("な", "na"), ("に", "ɲi"),
    ("ぬ", "nɯ"), ("ね", "ne"), ("の", "no"), ("は", "ha"), ("ば", "ba"), ("ぱ", "pa"),
    ("ひ", "çi"), ("び", "bʲi"), ("ぴ", "pʲi"), ("ふ", "ɸɯ"), ("ぶ", "bɯ"), ("ぷ", "pɯ"),
    ("へ", "he"), ("べ", "be"), ("ぺ", "pe"), ("ほ", "ho"), ("ぼ", "bo"), ("ぽ", "po"),
    ("ま", "ma"), ("み", "mʲi"), ("む", "mɯ"), ("め", "me"), ("も", "mo"), ("ゃ", "ja"),
    ("や", "ja"), ("ゅ", "jɯ"), ("ゆ", "jɯ"), ("ょ", "jo"), ("よ", "jo"), ("ら", "ɾa"),
    ("り", "ɾʲi"), ("る", "ɾɯ"), ("れ", "ɾe"), ("ろ", "ɾo"), ("ゎ", "βa"), ("わ", "βa"),
    ("ゐ", "i"), ("ゑ", "e"), ("を", "o"), ("ゔ", "vɯ"), ("ゕ", "ka"), ("ゖ", "ke"),
    ("ヷ", "va"), ("ヸ", "vʲi"), ("ヹ", "ve"), ("ヺ", "vo"), ("いぇ", "je"), ("うぃ", "βi"),
    ("うぇ", "βe"), ("うぉ", "βo"), ("きぇ", "kʲe"), ("きゃ", "kʲa"), ("きゅ", "kʲɨ"), ("きょ", "kʲo"),
    ("ぎゃ", "ɡʲa"), ("ぎゅ", "ɡʲɨ"), ("ぎょ", "ɡʲo"), ("くぁ", "kᵝa"), ("くぃ", "kᵝi"), ("くぇ", "kᵝe"),
    ("くぉ", "kᵝo"), ("ぐぁ", "ɡᵝa"), ("ぐぃ", "ɡᵝi"), ("ぐぇ", "ɡᵝe"), ("ぐぉ", "ɡᵝo"), ("しぇ", "ɕe"),
    ("しゃ", "ɕa"), ("しゅ", "ɕɨ"), ("しょ", "ɕo"), ("じぇ", "ʥe"), ("じゃ", "ʥa"), ("じゅ", "ʥɨ"),
    ("じょ", "ʥo"), ("ちぇ", "ʨe"), ("ちゃ", "ʨa"), ("ちゅ", "ʨɨ"), ("ちょ", "ʨo"), ("ぢゃ", "ʥa"),
    ("ぢゅ", "ʥɨ"), ("ぢょ", "ʥo"), ("つぁ", "ʦa"), ("つぃ", "ʦʲi"), ("つぇ", "ʦe"), ("つぉ", "ʦo"),
    ("てぃ", "tʲi"), ("てゅ", "tʲɨ"), ("でぃ", "dʲi"), ("でゅ", "dʲɨ"), ("とぅ", "tɯ"), ("どぅ", "dɯ"),
    ("にぇ", "ɲe"), ("にゃ", "ɲa"), ("にゅ", "ɲɨ"), ("にょ", "ɲo"), ("ひぇ", "çe"), ("ひゃ", "ça"),
    ("ひゅ", "çɨ"), ("ひょ", "ço"), ("びゃ", "bʲa"), ("びゅ", "bʲɨ"), ("びょ", "bʲo"), ("ぴゃ", "pʲa"),
    ("ぴゅ", "pʲɨ"), ("ぴょ", "pʲo"), ("ふぁ", "ɸa"), ("ふぃ", "ɸʲi"), ("ふぇ", "ɸe"), ("ふぉ", "ɸo"),
    ("ふゅ", "ɸʲɨ"), ("ふょ", "ɸʲo"), ("みゃ", "mʲa"), ("みゅ", "mʲɨ"), ("みょ", "mʲo"), ("りゃ", "ɾʲa"),
    ("りゅ", "ɾʲɨ"), ("りょ", "ɾʲo"), ("ゔぁ", "va"), ("ゔぃ", "vʲi"), ("ゔぇ", "ve"), ("ゔぉ", "vo"),
    ("ゔゅ", "bʲɨ"), ("ゔょ", "bʲo"),
];

/// Punctuation as cutlet maps it. NFKC folds fullwidth `？！：；（），～` to
/// ASCII first, and those pass through as written.
const SYMBOLS: &[(char, &str)] = &[
    ('。', "."),
    ('、', ","),
    ('「', "“"),
    ('」', "”"),
    ('『', "“"),
    ('』', "”"),
    ('《', "("),
    ('》', ")"),
    ('【', "["),
    ('】', "]"),
    ('・', " "),
    ('〜', "—"),
    ('—', "—"),
    ('«', "“"),
    ('»', "”"),
    // パーセント, as misaki's dictionary reads it.
    ('%', "paːsento"),
    // A combining sound mark on its own.
    ('\u{3099}', ""),
    ('\u{309a}', ""),
];

/// Small kana that join the kana before them.
const SUTEGANA: &[char] = &['ゃ', 'ゅ', 'ょ', 'ぁ', 'ぃ', 'ぅ', 'ぇ', 'ぉ'];

/// Kana and the voiced kana `ゞ` makes of them.
#[rustfmt::skip]
const DAKUTEN: &[(char, char)] = &[
    ('か', 'が'), ('き', 'ぎ'), ('く', 'ぐ'), ('け', 'げ'), ('こ', 'ご'),
    ('さ', 'ざ'), ('し', 'じ'), ('す', 'ず'), ('せ', 'ぜ'), ('そ', 'ぞ'),
    ('た', 'だ'), ('ち', 'ぢ'), ('つ', 'づ'), ('て', 'で'), ('と', 'ど'),
    ('は', 'ば'), ('ひ', 'び'), ('ふ', 'ぶ'), ('へ', 'べ'), ('ほ', 'ぼ'),
];

/// Rare katakana for sounds Ainu writes, as the full-size kana.
const PHONETIC_EXTENSIONS: &str = "クシストヌハヒフヘホムラリルレロ";

/// One word as cutlet's tagger would give it.
struct Word {
    surface: String,
    kind: Kind,
}

enum Kind {
    /// Read through the kana table, from this hiragana reading.
    Kana(String),
    /// One punctuation mark or other symbol.
    Symbol,
    /// Text in another script, read by the English voice.
    English,
}

/// The notice for `text` when too much of it is kanji to read.
pub(super) fn refusal(text: &str) -> Option<&'static str> {
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let kanji = text.chars().filter(|&c| is_kanji(c)).count();
    (kanji * 100 > letters * MAX_KANJI_PERCENT).then_some(KANJI_REFUSAL)
}

/// Kokoro's symbols for `text`, with its English read by `english`, and how many
/// kanji were skipped.
pub(super) fn phonemes(
    text: &str,
    mut english: impl FnMut(&str) -> Result<String, String>,
) -> Result<(String, usize), String> {
    let (words, skipped) = words(&normalize(text));
    // cutlet's `_romaji_tokens`: each word followed by a space, except around
    // punctuation. Python's `in` on strings is a substring test, so an empty
    // reading counts as an opening bracket.
    let mut tokens: Vec<(String, bool)> = Vec::with_capacity(words.len());
    for word in &words {
        let roma = match &word.kind {
            Kind::Kana(reading) => read_kana(reading),
            Kind::English => english(&word.surface)?,
            Kind::Symbol => word.surface.chars().map(symbol).collect(),
        };
        let space = if "「『«".contains(word.surface.as_str()) || "([".contains(roma.as_str()) {
            if let Some(last) = tokens.last_mut() {
                last.1 = true;
            }
            false
        } else if "」』»".contains(word.surface.as_str()) || "]).,?!:".contains(roma.as_str()) {
            if let Some(last) = tokens.last_mut() {
                last.1 = false;
            }
            true
        } else {
            roma != " "
        };
        // misaki's `«»`, kept apart from the English voice's own brackets.
        let roma = match (&word.kind, roma.as_str()) {
            (Kind::Symbol, "(") => "«".to_string(),
            (Kind::Symbol, ")") => "»".to_string(),
            _ => roma,
        };
        tokens.push((roma, space));
    }
    let mut joined = String::new();
    for (roma, space) in &tokens {
        joined.push_str(roma);
        if *space {
            joined.push(' ');
        }
    }
    let tidied = tidy_glottal_stops(&single_spaced(&joined));
    Ok((single_spaced(&tidied), skipped))
}

fn single_spaced(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// cutlet's last step: no space before or after a `ʔ`, except next to
/// punctuation. Brackets go too: Kokoro has no token for misaki's `«»` or
/// for `[]`.
fn tidy_glottal_stops(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        let before = i.checked_sub(1).map(|j| chars[j]);
        let after = chars.get(i + 1).copied();
        let drop = c == ' '
            && (after == Some('ʔ') && !before.is_some_and(|b| "!\",.:;?»—…”".contains(b))
                || before == Some('ʔ') && !after.is_some_and(|a| "\"«“".contains(a)));
        if !drop && !matches!(c, '«' | '»' | '[' | ']') {
            out.push(c);
        }
    }
    out
}

/// cutlet's `_normalize_text`: a wave dash before a number reads "から", the
/// Ainu small katakana become full size, then NFKC folds fullwidth ASCII,
/// half-width katakana and ellipses and joins a kana to a separate sound mark.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '〜' | '～' if chars.peek().is_some_and(|&next| is_digit(next)) => {
                out.push_str("から")
            }
            '\u{31f0}'..='\u{31ff}' => {
                out.extend(PHONETIC_EXTENSIONS.chars().nth(c as usize - 0x31f0))
            }
            c => out.push(c),
        }
    }
    out.nfkc().collect()
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || ('０'..='９').contains(&c)
}

/// The words of `text` and how many kanji were skipped: what cutlet's
/// dictionary tagger gives, as far as kana alone can tell.
fn words(text: &str) -> (Vec<Word>, usize) {
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut skipped = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if is_kanji(c) {
            skipped += 1;
            i += 1;
            continue;
        }
        let surface = |end: usize| chars[start..end].iter().collect::<String>();
        if let Some(end) = english_end(&chars, i) {
            i = end;
            words.push(Word {
                surface: surface(i),
                kind: Kind::English,
            });
        } else if c.is_ascii_digit() {
            i = number_end(&chars, i);
            let reading = number_kana(&surface(i));
            words.push(Word {
                kind: Kind::Kana(reading.chars().map(to_hiragana).collect()),
                surface: reading,
            });
        } else if let Some(mut run) = script(c) {
            i += 1;
            while let Some(next) = chars.get(i).and_then(|&c| script(c)) {
                match (run, next) {
                    (Script::Either, _) => run = next,
                    (_, Script::Either) => {}
                    _ if next != run => break,
                    _ => {}
                }
                i += 1;
            }
            let surface = surface(i);
            words.push(Word {
                kind: Kind::Kana(surface.chars().map(to_hiragana).collect()),
                surface,
            });
        } else {
            i += 1;
            words.push(Word {
                surface: surface(i),
                kind: Kind::Symbol,
            });
        }
    }
    (words, skipped)
}

/// Where the English starting at `start` ends: the whole run of text up to the
/// next Japanese character, when it has a letter or a currency sign, so the
/// English path sees "$4.99", "10:30" or "(1)" whole. A leading stop or comma
/// stays Japanese punctuation.
fn english_end(chars: &[char], start: usize) -> Option<usize> {
    if is_japanese(chars[start]) || ".,:;!?".contains(chars[start]) {
        return None;
    }
    let run = chars[start..]
        .iter()
        .take_while(|&&c| !is_japanese(c))
        .count();
    let end = start + run
        - chars[start..start + run]
            .iter()
            .rev()
            .take_while(|c| c.is_whitespace())
            .count();
    chars[start..end]
        .iter()
        .any(|&c| c.is_alphabetic() || matches!(c, '$' | '£' | '€'))
        .then_some(end)
}

/// Whether `text` has any Japanese writing in it: kana, kanji, Japanese
/// punctuation or fullwidth forms.
pub(super) fn has_japanese(text: &str) -> bool {
    text.chars().any(is_japanese)
}

fn is_japanese(c: char) -> bool {
    is_kanji(c)
        || matches!(c,
            '\u{3000}'..='\u{30ff}' | '\u{31f0}'..='\u{31ff}' | '\u{ff00}'..='\u{ffef}')
}

/// A number: digits, with thousands commas and a decimal part.
fn number_end(chars: &[char], start: usize) -> usize {
    let digits_from = |i: usize| chars[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut end = start + digits_from(start);
    if end - start <= 3 {
        while chars.get(end) == Some(&',') && digits_from(end + 1) == 3 {
            end += 4;
        }
    }
    if chars.get(end) == Some(&'.') && digits_from(end + 1) > 0 {
        end += 1 + digits_from(end + 1);
    }
    end
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Hiragana,
    Katakana,
    /// `ー` and `〃`, which go with either.
    Either,
}

fn script(c: char) -> Option<Script> {
    match c {
        '\u{3041}'..='\u{3096}' | 'ゝ' | 'ゞ' => Some(Script::Hiragana),
        '\u{30a1}'..='\u{30fa}' | 'ヽ' | 'ヾ' => Some(Script::Katakana),
        'ー' | '〃' => Some(Script::Either),
        _ => None,
    }
}

/// jaconv's `kata2hira`: `ヷヸヹヺ` stay, the table has them.
fn to_hiragana(c: char) -> char {
    match c {
        '\u{30a1}'..='\u{30f6}' | 'ヽ' | 'ヾ' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
        c => c,
    }
}

fn is_kanji(c: char) -> bool {
    matches!(c,
        '\u{4e00}'..='\u{9fff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{f900}'..='\u{faff}'
        | '\u{20000}'..='\u{2ffff}'
        | '々' | '〆' | '〇')
}

fn symbol(c: char) -> String {
    match SYMBOLS.iter().find(|(symbol, _)| *symbol == c) {
        Some((_, mapped)) => mapped.to_string(),
        None => c.to_string(),
    }
}

fn lookup(kana: &[char]) -> Option<&'static str> {
    KANA.iter()
        .find(|(key, _)| key.chars().eq(kana.iter().copied()))
        .map(|(_, sound)| *sound)
}

/// cutlet's `_romaji_word` for kana: each kana read with its neighbours.
fn read_kana(reading: &str) -> String {
    let kana: Vec<char> = reading.chars().collect();
    (0..kana.len())
        .map(|i| {
            let before = i.checked_sub(1).map(|j| kana[j]);
            sound(before, kana[i], kana.get(i + 1).copied())
        })
        .collect()
}

/// cutlet's `_get_single_mapping`.
fn sound(before: Option<char>, kana: char, after: Option<char>) -> String {
    match kana {
        // cutlet repeats the kana itself, which Kokoro cannot read.
        'ゝ' => return before.and_then(|b| lookup(&[b])).unwrap_or_default().into(),
        'ゞ' => {
            return before
                .and_then(voiced)
                .and_then(|v| lookup(&[v]))
                .unwrap_or_default()
                .into()
        }
        '〃' => return String::new(),
        _ => {}
    }
    if let Some(pair) = before.and_then(|b| lookup(&[b, kana])) {
        return pair.into();
    }
    if after.is_some_and(|a| lookup(&[kana, a]).is_some()) {
        return String::new();
    }
    if let Some(small) = after.filter(|a| SUTEGANA.contains(a)) {
        if kana == 'っ' {
            return String::new();
        }
        // cutlet fails on a kana outside its table here, such as "んゃ".
        if let (Some(base), Some(small)) = (lookup(&[kana]), lookup(&[small])) {
            let mut joined = base.to_string();
            joined.pop();
            return joined + small;
        }
    }
    if SUTEGANA.contains(&kana) {
        return String::new();
    }
    match kana {
        'ー' => "ː".into(),
        'っ' => "ʔ".into(),
        'ん' => nasal(after).into(),
        _ => lookup(&[kana]).unwrap_or_default().into(),
    }
}

/// `ん` takes the place of the sound after it.
fn nasal(after: Option<char>) -> &'static str {
    let next = after.and_then(|a| lookup(&[a])).unwrap_or_default();
    match next.chars().next() {
        Some('m' | 'p' | 'b') => "m",
        Some('k' | 'ɡ') => "ŋ",
        Some('ɲ' | 'ʨ' | 'ʥ') => "ɲ",
        Some('n' | 't' | 'd' | 'ɾ' | 'z') => "n",
        _ => "ɴ",
    }
}

fn voiced(kana: char) -> Option<char> {
    DAKUTEN
        .iter()
        .find(|(plain, _)| *plain == kana)
        .map(|(_, voiced)| *voiced)
}

/// A number in kana, as misaki's `num2kana.Convert` reads it. Past nine
/// digits, where `Convert` gives up, each digit is read.
fn number_kana(number: &str) -> String {
    let digits = number.replace(',', "");
    let (whole, fraction) = digits.split_once('.').unwrap_or((&digits, ""));
    let whole = match whole.trim_start_matches('0') {
        "" => "0",
        whole => whole,
    };
    let long = whole.len() > 9;
    let mut out = if long {
        whole.chars().filter_map(digit_kana).collect()
    } else {
        whole_kana(whole)
    };
    if fraction.is_empty() {
        return out;
    }
    // "10.5" is じゅってん, not じゅうてん.
    if !long && whole.len() > 1 && whole.ends_with('0') && !whole[..whole.len() - 1].ends_with('0')
    {
        out.pop();
        out.push('っ');
    }
    out.push_str("てん");
    out.extend(fraction.chars().filter_map(digit_kana));
    out
}

fn digit_kana(digit: char) -> Option<&'static str> {
    let at = digit.to_digit(10)? as usize;
    Some(
        [
            "ゼロ",
            "いち",
            "に",
            "さん",
            "よん",
            "ご",
            "ろく",
            "なな",
            "はち",
            "きゅう",
        ][at],
    )
}

fn digit(number: &str, at: usize) -> &'static str {
    number[at..]
        .chars()
        .next()
        .and_then(digit_kana)
        .unwrap_or_default()
}

/// `Convert` for a whole number of at most nine digits, no leading zero.
fn whole_kana(number: &str) -> String {
    match number.len() {
        1 => digit(number, 0).to_string(),
        2 => tens(number),
        3 => hundreds(number),
        4 => thousands(number, true),
        _ => millions(number),
    }
}

/// `len_two`.
fn tens(number: &str) -> String {
    match (&number[..1], &number[1..]) {
        ("0", _) => digit(number, 1).to_string(),
        ("1", "0") => "じゅう".to_string(),
        ("1", _) => format!("じゅう{}", digit(number, 1)),
        (_, "0") => format!("{}じゅう", digit(number, 0)),
        _ => format!("{}じゅう{}", digit(number, 0), digit(number, 1)),
    }
}

/// `len_three`.
fn hundreds(number: &str) -> String {
    let mut out = match &number[..1] {
        "1" => "ひゃく".to_string(),
        "3" => "さんびゃく".to_string(),
        "6" => "ろっぴゃく".to_string(),
        "8" => "はっぴゃく".to_string(),
        _ => format!("{}ひゃく", digit(number, 0)),
    };
    match &number[1..] {
        "00" => {}
        rest if rest.starts_with('0') => out.push_str(digit(number, 2)),
        rest => out.push_str(&tens(rest)),
    }
    out
}

/// `len_four`; `alone` is a number under ten thousand, where 1000 is せん
/// rather than いっせん.
fn thousands(number: &str, alone: bool) -> String {
    let number = number.trim_start_matches('0');
    match number.len() {
        0 => return String::new(),
        1 => return digit(number, 0).to_string(),
        2 => return tens(number),
        3 => return hundreds(number),
        _ => {}
    }
    let mut out = match &number[..1] {
        "1" if alone => "せん".to_string(),
        "1" => "いっせん".to_string(),
        "3" => "さんぜん".to_string(),
        "8" => "はっせん".to_string(),
        _ => format!("{}せん", digit(number, 0)),
    };
    match &number[1..] {
        "000" => {}
        rest if rest.starts_with('0') => out.push_str(&tens(&rest[1..])),
        rest => out.push_str(&hundreds(rest)),
    }
    out
}

/// `len_x`: five to nine digits.
fn millions(number: &str) -> String {
    let (high, low) = number.split_at(number.len() - 4);
    let mut out = match high.len() {
        1 => digit(high, 0).to_string(),
        2 => tens(high),
        3 => hundreds(high),
        4 => thousands(high, false),
        _ => {
            let man = &high[1..];
            let mut out = format!("{}おく{}", digit(high, 0), thousands(man, false));
            if man != "0000" {
                out.push_str("まん");
            }
            out.push_str(&thousands(low, false));
            return out;
        }
    };
    out.push_str("まん");
    out.push_str(&thousands(low, false));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::local::in_vocab;

    /// Stands in for the English voice, as the expected strings were made.
    fn read(text: &str) -> (String, usize) {
        phonemes(text, |latin| Ok(format!("<{latin}>"))).unwrap()
    }

    fn reads(text: &str) -> String {
        let (phonemes, skipped) = read(text);
        assert_eq!(skipped, 0, "{text}");
        phonemes
    }

    #[test]
    fn every_symbol_the_table_gives_is_one_kokoro_has() {
        let brackets = ['(', ')', '[', ']'];
        let symbols = KANA
            .iter()
            .flat_map(|(_, sound)| sound.chars())
            .chain(SYMBOLS.iter().flat_map(|(_, mapped)| mapped.chars()))
            .chain(['ː', 'ʔ', 'm', 'ŋ', 'ɲ', 'n', 'ɴ']);
        for symbol in symbols {
            assert!(
                in_vocab(symbol) || brackets.contains(&symbol),
                "{symbol:?} is not in Kokoro's vocabulary"
            );
        }
        // Every kana and pair before every single kana, and ん before each.
        let single = || KANA.iter().filter(|(kana, _)| kana.chars().count() == 1);
        let mut text = String::new();
        for (kana, _) in KANA {
            for (after, _) in single() {
                text.push_str(kana);
                text.push_str(after);
                text.push(' ');
            }
        }
        for (after, _) in single() {
            text.push('ん');
            text.push_str(after);
            text.push(' ');
        }
        text.extend(SYMBOLS.iter().map(|(symbol, _)| symbol));
        text.push_str("ーっゝゞ〃ヽヾ「（あ）」【い】");
        let (phonemes, _) = phonemes(&text, |_| Ok(String::new())).unwrap();
        let outside: Vec<char> = phonemes.chars().filter(|&c| !in_vocab(c)).collect();
        assert!(outside.is_empty(), "{outside:?}");
    }

    // Expected strings come from misaki's own `Cutlet._romaji_tokens` and the
    // last steps of `Cutlet.__call__`, fed the words `words` makes.

    #[test]
    fn plain_hiragana() {
        assert_eq!(reads("ありがとう"), "aɾʲiɡatoɯ");
        assert_eq!(reads("しじすちつ"), "ɕiʥisɨʨiʦɨ");
        assert_eq!(reads("あいうえお　かきくけこ"), "aiɯeo kakʲikɯkeko");
    }

    #[test]
    fn small_kana_join_the_kana_before() {
        assert_eq!(reads("きゃきゅきょ"), "kʲakʲɨkʲo");
        assert_eq!(reads("しゃしゅしょ"), "ɕaɕɨɕo");
        assert_eq!(reads("ちゃちゅちょ"), "ʨaʨɨʨo");
        assert_eq!(reads("りゃりゅりょ"), "ɾʲaɾʲɨɾʲo");
        assert_eq!(reads("ぴゃぴゅぴょ"), "pʲapʲɨpʲo");
        // Not a pair in the table: the kana's consonant and the small vowel.
        assert_eq!(reads("ちぃ"), "ʨi");
        // cutlet fails on this one.
        assert_eq!(reads("んゃ"), "ɴ");
    }

    #[test]
    fn katakana_reads_as_hiragana_and_long_marks_lengthen() {
        assert_eq!(reads("ラーメン"), "ɾaːmeɴ");
        assert_eq!(reads("コーヒー"), "koːçiː");
        assert_eq!(reads("スーパー"), "sɨːpaː");
        assert_eq!(reads("ティーシャツ"), "tʲiːɕaʦɨ");
        assert_eq!(reads("ファイル"), "ɸaiɾɯ");
        assert_eq!(reads("ウィキ"), "βikʲi");
        assert_eq!(reads("クォーター"), "kᵝoːtaː");
        assert_eq!(reads("ヷヸヹヺ"), "vavʲivevo");
        assert_eq!(reads("ゕゖ"), "kake");
        assert_eq!(reads("ㇰㇱ"), "kɯɕi");
    }

    #[test]
    fn vu_reads_v() {
        assert_eq!(reads("ゔぁいおりん"), "vaioɾʲiɴ");
        assert_eq!(reads("ヴァイオリン"), "vaioɾʲiɴ");
        assert_eq!(reads("ヴィ"), "vʲi");
    }

    #[test]
    fn small_tsu_is_a_glottal_stop_before_any_consonant() {
        assert_eq!(reads("がっこう"), "ɡaʔkoɯ");
        assert_eq!(reads("きって"), "kʲiʔte");
        assert_eq!(reads("いっぱい"), "iʔpai");
        assert_eq!(reads("ざっし"), "ʣaʔɕi");
        assert_eq!(reads("まって"), "maʔte");
    }

    #[test]
    fn n_takes_the_place_of_the_next_sound() {
        assert_eq!(reads("さんぽ"), "sampo");
        assert_eq!(reads("しんぶん"), "ɕimbɯɴ");
        assert_eq!(reads("さんま"), "samma");
        assert_eq!(reads("げんき"), "ɡeŋkʲi");
        assert_eq!(reads("まんが"), "maŋɡa");
        assert_eq!(reads("こんにゃく"), "koɲɲakɯ");
        assert_eq!(reads("かんじ"), "kaɲʥi");
        assert_eq!(reads("みんな"), "mʲinna");
        assert_eq!(reads("ほんと"), "honto");
        assert_eq!(reads("てんいん"), "teɴiɴ");
        assert_eq!(reads("ほん"), "hoɴ");
    }

    #[test]
    fn repeat_marks_repeat_the_kana_before() {
        // cutlet gives the kana itself here; Kokoro would drop it.
        assert_eq!(reads("こゝろ"), "kokoɾo");
        assert_eq!(reads("いすゞ"), "isɨzɨ");
        assert_eq!(reads("バナヽ"), "banana");
    }

    #[test]
    fn punctuation_is_kept_and_spaced_as_cutlet_spaces_it() {
        assert_eq!(reads("あ、い。う！え？"), "a, i. ɯ! e?");
        assert_eq!(reads("「はい」と、いった。"), "“hai” to, iʔta.");
        assert_eq!(reads("ねこ・いぬ"), "neko inɯ");
        // misaki's «» and [] have no Kokoro token.
        assert_eq!(reads("『ほん』（うえ）【した】"), "“hoɴ” ɯe ɕita");
    }

    #[test]
    fn text_in_another_script_goes_to_the_english_voice_whole() {
        assert_eq!(reads("きょうはOKです。"), "kʲoɯha <OK> desɨ.");
        assert_eq!(reads("これはiPhone 15です。"), "koɾeha <iPhone 15> desɨ.");
        assert_eq!(reads("ＡＢＣ１２３"), "<ABC123>");
        assert_eq!(reads("3 tests passed."), "<3 tests passed.>");
        assert_eq!(reads("3.5GBです"), "<3.5GB> desɨ");
        assert_eq!(reads("$5です"), "<$5> desɨ");
        assert_eq!(reads("ねこ (cat) です"), "neko <(cat)> desɨ");
        assert_eq!(reads("じかんは10:30 a.m.です"), "ʥikaɴha <10:30 a.m.> desɨ");
        assert_eq!(
            reads("「Hello, world!」といった。"),
            "“<Hello, world!>” toiʔta."
        );
        assert_eq!(reads("naïve caféです"), "<naïve café> desɨ");
        // A leading stop stays Japanese punctuation.
        assert_eq!(reads("メモ: OK"), "memo: <OK>");
    }

    #[test]
    fn numbers_and_percent_with_no_english_read_in_japanese() {
        assert_eq!(reads("3こ"), "saɴ ko");
        assert_eq!(reads("50%です"), "ɡoʥɨɯ paːsento desɨ");
        assert_eq!(reads("(1)りんご"), "iʨi ɾʲiŋɡo");
    }

    #[test]
    fn ellipses_read_as_stops() {
        assert_eq!(reads("そうですね…"), "soɯdesɨne...");
        assert_eq!(reads("ええ‥"), "ee..");
    }

    #[test]
    fn half_width_katakana_reads_as_full_width() {
        assert_eq!(reads("ｶﾀｶﾅ"), reads("カタカナ"));
        assert_eq!(reads("ｶﾀｶﾅ"), "katakana");
        assert_eq!(reads("ｶﾞｲﾄﾞ"), "ɡaido");
        assert_eq!(reads("ﾊﾟｰﾃｨｰ"), "paːtʲiː");
        assert_eq!(reads("ｺｰﾋｰ｡"), "koːçiː.");
    }

    #[test]
    fn a_kana_and_a_separate_sound_mark_read_as_one_kana() {
        assert_eq!(reads("か\u{3099}っこう"), reads("がっこう"));
        assert_eq!(reads("か\u{3099}っこう"), "ɡaʔkoɯ");
        assert_eq!(reads("さんほ\u{309a}"), "sampo");
        assert_eq!(reads("ハ\u{309a}ン"), "paɴ");
        assert_eq!(reads("ふ\u{3099}た"), "bɯta");
    }

    #[test]
    fn numbers_read_in_japanese() {
        for (number, expected) in [
            ("0", "ʣeɾo"),
            ("10", "ʥɨɯ"),
            ("15", "ʥɨɯɡo"),
            ("100", "çakɯ"),
            ("300", "sambʲakɯ"),
            ("1000", "seɴ"),
            ("2024", "ɲiseɲɲiʥɨɯjoɴ"),
            ("10000", "iʨimaɴ"),
            ("3.5", "santeŋɡo"),
            ("1,500", "seŋɡoçakɯ"),
            ("〜5", "kaɾa ɡo"),
            ("5〜10", "ɡo kaɾa ʥɨɯ"),
        ] {
            assert_eq!(reads(number), expected, "{number}");
        }
    }

    #[test]
    fn numbers_read_as_num2kana_reads_them() {
        // From misaki's `num2kana.Convert`.
        for (number, expected) in [
            ("7", "なな"),
            ("11", "じゅういち"),
            ("40", "よんじゅう"),
            ("99", "きゅうじゅうきゅう"),
            ("101", "ひゃくいち"),
            ("600", "ろっぴゃく"),
            ("800", "はっぴゃく"),
            ("1001", "せんいち"),
            ("3000", "さんぜん"),
            ("8000", "はっせん"),
            ("11000", "いちまんいっせん"),
            (
                "123456789",
                "いちおくにせんさんびゃくよんじゅうごまんろくせんななひゃくはちじゅうきゅう",
            ),
            ("100000000", "いちおく"),
            ("10.5", "じゅってんご"),
            ("100.25", "ひゃくてんにご"),
            ("007", "なな"),
        ] {
            assert_eq!(number_kana(number), expected, "{number}");
        }
        assert_eq!(number_kana("0.5"), "ゼロてんご");
        assert_eq!(
            number_kana("1234567890"),
            "いちにさんよんごろくななはちきゅうゼロ"
        );
        assert_eq!(
            number_kana("1234567890.5"),
            "いちにさんよんごろくななはちきゅうゼロてんご"
        );
        // A comma groups only exactly three digits after it.
        assert_eq!(reads("1,5000"), "iʨi, ɡoseɴ");
    }

    #[test]
    fn kanji_are_skipped_and_counted() {
        assert_eq!(read("わたしの本です。"), ("βataɕino desɨ.".into(), 1));
        assert_eq!(read("東京にいきます。"), ("ɲiikʲimasɨ.".into(), 2));
    }

    #[test]
    fn text_mostly_in_kanji_is_refused() {
        assert_eq!(refusal("東京に行きます。"), Some(KANJI_REFUSAL));
        assert_eq!(refusal("わたしの本です。"), None);
        assert_eq!(refusal("これはペンです"), None);
        // 1 of 5 letters is 20%, which is read; 2 of 6 is not.
        assert_eq!(refusal("本あいうえ"), None);
        assert_eq!(refusal("本日あいうえ"), Some(KANJI_REFUSAL));
        assert_eq!(refusal("123。"), None);
    }
}
