//! Mandarin as Kokoro's Chinese voices were trained to read it: misaki's
//! `ZHG2P` as Kokoro v1.0 runs it (`misaki/zh.py`, the legacy path). Numbers
//! become Chinese numerals as cn2an writes them, jieba cuts the text into
//! words, pypinyin reads each word (phrase readings first, then each
//! character's first reading), and misaki's tables, adapted from pinyin-to-ipa,
//! turn each syllable into Kokoro's symbols.
//!
//! Text in another script is read by the English voice, where misaki passes it
//! through as written.

use jieba_rs::Jieba;
use log::info;
use pinyin::ToPinyin;
use std::sync::OnceLock;
use std::time::Instant;

/// Longer sentences are cut before this. Mandarin prose runs about four ids a
/// character, so a chunk fits one inference; `split_tokens` cuts the rare
/// longer one.
pub(super) const MAX_CHUNK_CHARS: usize = 100;

/// mozillazg/phrase-pinyin-data 0.19.0, the version pypinyin 0.55 builds its
/// phrase readings from. Every phrase is kept: dropping those that read as
/// their characters would move pypinyin's word matching.
const PHRASE_DATA: &str = include_str!("data/phrase-pinyin.txt");

/// What `punctuation` leaves that Kokoro has a token for, kept as written
/// around a run of English.
const KOKORO_PUNCTUATION: &str = ";:,.!?—…\"()“”";

static DICTIONARIES: OnceLock<Dictionaries> = OnceLock::new();

struct Dictionaries {
    jieba: Jieba,
    phrases: Phrases,
}

/// Loads the dictionaries, once, so the first sentence does not wait for them.
pub(super) fn load() {
    dictionaries();
}

fn dictionaries() -> &'static Dictionaries {
    DICTIONARIES.get_or_init(|| {
        let started = Instant::now();
        let dictionaries = Dictionaries {
            jieba: Jieba::new(),
            phrases: Phrases::parse(PHRASE_DATA),
        };
        info!(
            "Loaded the Mandarin dictionaries in {} ms",
            started.elapsed().as_millis()
        );
        dictionaries
    })
}

/// Whether `text` has any Chinese writing in it: Han characters or CJK
/// punctuation. Full-width forms alone, such as the ￥ in "It costs ￥500", do
/// not count.
pub(super) fn has_chinese(text: &str) -> bool {
    text.chars()
        .any(|c| is_han(c) || ('\u{3000}'..='\u{303f}').contains(&c))
}

/// misaki reads only U+4E00–U+9FFF; the extensions, compatibility forms and
/// `〇` have readings too, so they are read rather than dropped.
fn is_han(c: char) -> bool {
    matches!(c,
        '\u{4e00}'..='\u{9fff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{f900}'..='\u{faff}'
        | '\u{20000}'..='\u{2ffff}'
        | '〇')
}

/// Kokoro's symbols for `text`, with its other scripts read by `english`, and
/// how many characters had no reading.
pub(super) fn phonemes(
    text: &str,
    mut english: impl FnMut(&str) -> Result<String, String>,
) -> Result<(String, usize), String> {
    let text = punctuation(&numerals(&halfwidth(&without_space_after_marks(text))));
    let dictionaries = dictionaries();
    let mut out = String::with_capacity(text.len() * 3);
    let mut skipped = 0;
    let mut space_before_han = false;
    for (han, run) in runs(&text) {
        if han {
            if space_before_han {
                out.push(' ');
                space_before_han = false;
            }
            let mut words: Vec<String> = Vec::new();
            let mut takes_er = false;
            for word in dictionaries.jieba.cut(run, true) {
                // jieba splits off the 儿 of a word its dictionary lacks (画画儿),
                // which misaki then says as a syllable of its own.
                if word == "儿" && takes_er {
                    if let Some(last) = words.last_mut() {
                        last.push('ɚ');
                    }
                    takes_er = false;
                    continue;
                }
                let reading = dictionaries.word(word, &mut skipped);
                takes_er = can_take_er(word, &reading);
                if !reading.is_empty() {
                    words.push(reading);
                }
            }
            out.push_str(&words.join(" "));
            continue;
        }
        // misaki passes other text through: Kokoro has tokens for its
        // punctuation and drops the rest. Words go to the English voice.
        if !run.contains(char::is_alphabetic) {
            out.push_str(run);
            continue;
        }
        let edge = |c: char| c.is_whitespace() || KOKORO_PUNCTUATION.contains(c);
        let start = run.len() - run.trim_start_matches(edge).len();
        let end = run.trim_end_matches(edge).len();
        out.push_str(&run[..start]);
        let words = english(&run[start..end])?;
        if !words.is_empty() {
            if start == 0 && !out.is_empty() && !out.ends_with(char::is_whitespace) {
                out.push(' ');
            }
            out.push_str(&words);
            space_before_han = end == run.len();
        }
        out.push_str(&run[end..]);
    }
    Ok((out.replace('\u{32f}', ""), skipped))
}

/// Whether a lone 儿 after `word` is its erhua. After a toneless particle
/// (他的儿, 抱着儿), a measure word (三个儿) or literary 与 and 吾, 儿 is
/// "son" or "child", a syllable of its own.
fn can_take_er(word: &str, reading: &str) -> bool {
    reading.contains(['→', '↗', '↘', '↓']) && !word.ends_with('个') && !matches!(word, "与" | "吾")
}

/// `text` cut into runs of Han characters and runs of anything else, in order,
/// each with whether it is Han.
fn runs(text: &str) -> Vec<(bool, &str)> {
    let mut runs = Vec::new();
    let mut start = 0;
    let mut han = None;
    for (i, c) in text.char_indices() {
        let is = is_han(c);
        if han.is_some_and(|han| han != is) {
            runs.push((!is, &text[start..i]));
            start = i;
        }
        han = Some(is);
    }
    if let Some(han) = han {
        runs.push((han, &text[start..]));
    }
    runs
}

/// Full-width digits, letters and the ideographic space as ASCII, as misaki's
/// later frontend folds them (cn2an already reads full-width digits). Not
/// NFKC: full-width `，。！？：；` must stay for `punctuation` to space them.
fn halfwidth(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '０'..='９' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ' => {
                char::from_u32(c as u32 - 0xfee0).unwrap_or(c)
            }
            '\u{3000}' => ' ',
            c => c,
        })
        .collect()
}

/// Drops the space the splitter puts after a mark that `punctuation` spaces
/// itself, so "你好。 这" reads as "你好。这" does.
fn without_space_after_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut after_mark = false;
    for c in text.chars() {
        if after_mark && c.is_whitespace() {
            continue;
        }
        after_mark = "、，。．！：；？»》」】）".contains(c);
        out.push(c);
    }
    out
}

/// misaki's `map_punctuation`, then its strip.
fn punctuation(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    for c in text.chars() {
        match c {
            '、' | '，' => out.push_str(", "),
            '。' | '．' => out.push_str(". "),
            '！' => out.push_str("! "),
            '：' => out.push_str(": "),
            '；' => out.push_str("; "),
            '？' => out.push_str("? "),
            '«' | '《' | '「' | '【' => out.push_str(" “"),
            '»' | '》' | '」' | '】' => out.push_str("” "),
            '（' => out.push_str(" ("),
            '）' => out.push_str(") "),
            c => out.push(c),
        }
    }
    out.trim().to_string()
}

impl Dictionaries {
    /// misaki's `word2ipa`: the word's syllables, unspaced.
    fn word(&self, word: &str, skipped: &mut usize) -> String {
        let mut out = String::new();
        for part in self.phrases.cut(word) {
            match self.phrases.get(part) {
                Some(readings) => {
                    for reading in readings.split_whitespace() {
                        out.push_str(&syllable(reading).unwrap_or_default());
                    }
                }
                None => {
                    for c in part.chars() {
                        match c.to_pinyin().and_then(|p| syllable(p.with_tone())) {
                            Some(sound) => out.push_str(&sound),
                            None => *skipped += 1,
                        }
                    }
                }
            }
        }
        out
    }
}

/// Phrases and their readings, sorted, the first entry of a phrase kept.
struct Phrases(Vec<(&'static str, &'static str)>);

impl Phrases {
    /// Lines of `phrase: reading reading …`, with `#` comments.
    fn parse(data: &'static str) -> Self {
        let mut phrases: Vec<(&str, &str)> = data
            .lines()
            .filter_map(|line| {
                let line = line.split('#').next().unwrap_or_default().trim();
                let (phrase, readings) = line.split_once(':')?;
                Some((phrase.trim(), readings.trim()))
            })
            .collect();
        phrases.sort_by_key(|(phrase, _)| *phrase);
        phrases.dedup_by_key(|(phrase, _)| *phrase);
        Self(phrases)
    }

    fn get(&self, phrase: &str) -> Option<&'static str> {
        self.0
            .binary_search_by_key(&phrase, |(phrase, _)| phrase)
            .ok()
            .map(|at| self.0[at].1)
    }

    /// Whether some phrase starts with `prefix`.
    fn starts(&self, prefix: &str) -> bool {
        let at = self.0.partition_point(|(phrase, _)| *phrase < prefix);
        self.0
            .get(at)
            .is_some_and(|(phrase, _)| phrase.starts_with(prefix))
    }

    /// pypinyin's `mmseg.Seg.cut` with `no_non_phrases`: the longest phrase at
    /// each place, else one character. Where every prefix of the rest leads to
    /// a phrase but none is one, the rest goes character by character.
    fn cut<'a>(&self, text: &'a str) -> Vec<&'a str> {
        let mut parts = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            let mut phrase_end = None;
            let mut whole_rest_is_prefix = true;
            for (i, c) in rest.char_indices() {
                let end = i + c.len_utf8();
                let word = &rest[..end];
                if !self.starts(word) {
                    whole_rest_is_prefix = false;
                    break;
                }
                if self.get(word).is_some() {
                    phrase_end = Some(end);
                }
            }
            match phrase_end {
                Some(end) => {
                    parts.push(&rest[..end]);
                    rest = &rest[end..];
                }
                None if whole_rest_is_prefix => {
                    parts.extend(rest.char_indices().map(|(i, c)| &rest[i..i + c.len_utf8()]));
                    break;
                }
                None => {
                    let first = rest.chars().next().map_or(0, char::len_utf8);
                    parts.push(&rest[..first]);
                    rest = &rest[first..];
                }
            }
        }
        parts
    }
}

/// Chao tone letters for tones 1 to 5, as misaki's tables write them.
const TONES: [&str; 5] = ["˥", "˧˥", "˧˩˧", "˥˩", ""];

// misaki's `transcription.py` tables, adapted from pinyin-to-ipa: the first
// variant of each, its phonemes joined, `0` where the tone goes.

const INITIALS: &[(&str, &str)] = &[
    ("b", "p"),
    ("c", "ʦʰ"),
    ("ch", "\u{ab67}ʰ"),
    ("d", "t"),
    ("f", "f"),
    ("g", "k"),
    ("h", "x"),
    ("j", "ʨ"),
    ("k", "kʰ"),
    ("l", "l"),
    ("m", "m"),
    ("n", "n"),
    ("p", "pʰ"),
    ("q", "ʨʰ"),
    ("r", "ɻ"),
    ("s", "s"),
    ("sh", "ʂ"),
    ("t", "tʰ"),
    ("x", "ɕ"),
    ("z", "ʦ"),
    ("zh", "\u{ab67}"),
];

const SYLLABIC_CONSONANTS: &[(&str, &str)] = &[
    ("hm", "hm0"),
    ("hng", "hŋ0"),
    ("m", "m0"),
    ("n", "n0"),
    ("ng", "ŋ0"),
];

const INTERJECTIONS: &[(&str, &str)] = &[("io", "jɔ0"), ("ê", "ɛ0"), ("er", "ɚ0"), ("o", "ɔ0")];

const FINALS: &[(&str, &str)] = &[
    ("a", "a0"),
    ("ai", "ai\u{32f}0"),
    ("an", "a0n"),
    ("ang", "a0ŋ"),
    ("ao", "au\u{32f}0"),
    ("e", "ɤ0"),
    ("ei", "ei\u{32f}0"),
    ("en", "ə0n"),
    ("eng", "ə0ŋ"),
    ("i", "i0"),
    ("ia", "ja0"),
    ("ian", "jɛ0n"),
    ("iang", "ja0ŋ"),
    ("iao", "jau\u{32f}0"),
    ("ie", "je0"),
    ("in", "i0n"),
    ("iou", "jou\u{32f}0"),
    ("ing", "i0ŋ"),
    ("iong", "jʊ0ŋ"),
    ("ong", "ʊ0ŋ"),
    ("ou", "ou\u{32f}0"),
    ("u", "u0"),
    ("uei", "wei\u{32f}0"),
    ("ua", "wa0"),
    ("uai", "wai\u{32f}0"),
    ("uan", "wa0n"),
    ("uen", "wə0n"),
    ("uang", "wa0ŋ"),
    ("ueng", "wə0ŋ"),
    ("uo", "wo0"),
    ("o", "wo0"),
    ("ü", "y0"),
    ("üe", "ɥe0"),
    ("üan", "ɥɛ0n"),
    ("ün", "y0n"),
];

/// `i` after zh, ch, sh and r.
const RETROFLEX_I: &str = "ɻ\u{329}0";
/// `i` after z, c and s.
const DENTAL_I: &str = "ɹ\u{329}0";

/// pypinyin's initials, in the order it tries them.
const PINYIN_INITIALS: &[&str] = &[
    "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "zh", "ch", "sh", "r",
    "z", "c", "s",
];

/// pypinyin's finals, as its strict mode spells them.
const PINYIN_FINALS: &[&str] = &[
    "i", "u", "ü", "a", "ia", "ua", "o", "uo", "e", "ie", "üe", "ai", "uai", "ei", "uei", "ao",
    "iao", "ou", "iou", "an", "ian", "uan", "üan", "en", "in", "uen", "ün", "ang", "iang", "uang",
    "eng", "ing", "ueng", "ong", "iong", "er", "ê",
];

fn table(table: &[(&str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// One tone-marked syllable ("zhōng", "lǜ", "men") as misaki's `py2ipa` reads
/// it, with the U+032F misaki drops removed: `None` where misaki would fail.
fn syllable(reading: &str) -> Option<String> {
    let (plain, tone) = tone_number(reading);
    let tone_letters = TONES[usize::from(tone) - 1];
    let ipa = if let Some(sound) = table(INTERJECTIONS, &plain) {
        sound.replace('0', tone_letters)
    } else if let Some(sound) = table(SYLLABIC_CONSONANTS, &plain) {
        sound.replace('0', tone_letters)
    } else {
        let initial = initial(&plain, true);
        let fin = pinyin_final(&plain)?;
        let fin = match (initial, fin) {
            ("zh" | "ch" | "sh" | "r", "i") => RETROFLEX_I,
            ("z" | "c" | "s", "i") => DENTAL_I,
            _ => table(FINALS, fin)?,
        };
        let initial = if initial.is_empty() {
            ""
        } else {
            table(INITIALS, initial)?
        };
        format!("{initial}{}", fin.replace('0', tone_letters))
    };
    Some(retone(&ipa).replace('\u{32f}', ""))
}

/// misaki's `retone`: Chao letters to Kokoro's tone arrows, and the syllabic
/// `ɻ̩` and `ɹ̩` to `ɨ`.
fn retone(ipa: &str) -> String {
    ipa.replace("˧˩˧", "↓")
        .replace("˧˥", "↗")
        .replace("˥˩", "↘")
        .replace('˥', "→")
        .replace("ɻ\u{329}", "ɨ")
        .replace("ɹ\u{329}", "ɨ")
}

/// The syllable without its tone mark, and its tone: 5 when it has none.
fn tone_number(reading: &str) -> (String, u8) {
    let mut plain = String::with_capacity(reading.len());
    let mut tone = 5;
    for c in reading.chars() {
        let (base, mark) = match c {
            'ā' => ('a', 1),
            'á' => ('a', 2),
            'ǎ' => ('a', 3),
            'à' => ('a', 4),
            'ē' => ('e', 1),
            'é' => ('e', 2),
            'ě' => ('e', 3),
            'è' => ('e', 4),
            'ō' => ('o', 1),
            'ó' => ('o', 2),
            'ǒ' => ('o', 3),
            'ò' => ('o', 4),
            'ī' => ('i', 1),
            'í' => ('i', 2),
            'ǐ' => ('i', 3),
            'ì' => ('i', 4),
            'ū' => ('u', 1),
            'ú' => ('u', 2),
            'ǔ' => ('u', 3),
            'ù' => ('u', 4),
            'ǖ' => ('ü', 1),
            'ǘ' => ('ü', 2),
            'ǚ' => ('ü', 3),
            'ǜ' => ('ü', 4),
            'ń' => ('n', 2),
            'ň' => ('n', 3),
            'ǹ' => ('n', 4),
            'ḿ' => ('m', 2),
            'ế' => ('ê', 2),
            'ề' => ('ê', 4),
            // Combining marks, as in "m̄" and "ê̌".
            '\u{304}' => {
                tone = 1;
                continue;
            }
            '\u{301}' => {
                tone = 2;
                continue;
            }
            '\u{30c}' => {
                tone = 3;
                continue;
            }
            '\u{300}' => {
                tone = 4;
                continue;
            }
            c => (c, 0),
        };
        plain.push(base);
        if mark > 0 {
            tone = mark;
        }
    }
    (plain, tone)
}

/// pypinyin's `get_initials`; without `strict`, `y` and `w` count too.
fn initial(syllable: &str, strict: bool) -> &'static str {
    let loose: &[&'static str] = if strict { &[] } else { &["y", "w"] };
    PINYIN_INITIALS
        .iter()
        .chain(loose)
        .find(|initial| syllable.starts_with(**initial))
        .copied()
        .unwrap_or_default()
}

/// pypinyin's strict `to_finals`: the final as the scheme spells it in full,
/// so "yu" is "ü", "jun" is "ün" and "gui" is "uei".
fn pinyin_final(syllable: &str) -> Option<&'static str> {
    let full = full_final_spelling(syllable);
    let known = |rest: &str| PINYIN_FINALS.iter().find(|f| **f == rest).copied();
    let strict = &full[initial(&full, true).len()..];
    known(strict).or_else(|| known(&full[initial(&full, false).len()..]))
}

/// pypinyin's `convert_finals`: y and w restored to the vowels they stand
/// for, ü after j, q and x, and the iou, uei and uen that iu, ui and un
/// abbreviate.
fn full_final_spelling(syllable: &str) -> String {
    let mut out = syllable.to_string();
    if let Some(rest) = syllable.strip_prefix('y') {
        let restored = if let Some(rest) = rest.strip_prefix('u') {
            format!("ü{rest}")
        } else if rest.starts_with('i') {
            rest.to_string()
        } else {
            format!("i{rest}")
        };
        if PINYIN_FINALS.contains(&restored.as_str()) {
            out = restored;
        }
    } else if let Some(rest) = syllable.strip_prefix('w') {
        let restored = if rest.starts_with('u') {
            rest.to_string()
        } else {
            format!("u{rest}")
        };
        if PINYIN_FINALS.contains(&restored.as_str()) {
            out = restored;
        }
    }
    if let Some(rest) = ["j", "q", "x"]
        .iter()
        .find_map(|initial| out.strip_prefix(initial).map(|rest| (initial, rest)))
        .and_then(|(initial, rest)| {
            rest.strip_prefix('u')
                .map(|rest| format!("{initial}ü{rest}"))
        })
    {
        out = rest;
    }
    // pypinyin's patterns: `^([a-z]+)iu$`, `([a-z]+)ui$`, `([a-z]+)un$`.
    let lowercase = |stem: &str| !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_lowercase());
    if let Some(stem) = out.strip_suffix("iu").filter(|stem| lowercase(stem)) {
        out = format!("{stem}iou");
    }
    for (short, full) in [("ui", "uei"), ("un", "uen")] {
        if let Some(stem) = out
            .strip_suffix(short)
            .filter(|stem| stem.ends_with(|c: char| c.is_ascii_lowercase()))
        {
            out = format!("{stem}{full}");
        }
    }
    out
}

/// cn2an's `transform(text, "an2cn")`, which misaki runs first: ranges before
/// a measure word, dates, fractions, percentages, temperatures, then every
/// other number, each pass over what the last one left.
///
/// Departures, where cn2an would leave the digits for Kokoro to drop or read
/// them wrongly: phone numbers and numbers past 16 digits are read digit by
/// digit; "1,500" is one number; a hyphen between amounts with units is a range
/// and one after a letter or digit ("COVID-19") is not a minus; and a
/// temperature keeps its sign and decimals.
fn numerals(text: &str) -> String {
    let text = unit_ranges(&ungrouped(&phone_numbers(text)));
    let text = each_match(&text, range, |m| {
        let (start, end) = m.split_once('-').unwrap_or((m, ""));
        format!("{}到{}", low(start), low(end))
    });
    let text = each_match(&text, date, |m| {
        let mut out = String::new();
        let mut rest = m;
        while let Some(at) = rest.find(|c: char| c.is_ascii_digit()) {
            out.push_str(&rest[..at]);
            let digits = &rest[at..];
            let len = digits
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(digits.len());
            let number = &digits[..len];
            rest = &digits[len..];
            if rest.starts_with('年') {
                out.push_str(&direct(number));
            } else {
                out.push_str(&low(number));
            }
        }
        out.push_str(rest);
        out
    });
    let text = each_match(&text, fraction, |m| {
        let (numerator, denominator) = m.split_once('/').unwrap_or((m, ""));
        format!("{}分之{}", low(denominator), low(numerator))
    });
    let text = each_match(&text, percent, |m| {
        format!("百分之{}", low(m.trim_end_matches('%')))
    });
    let text = each_match(&text, celsius, |m| {
        let degrees = m.trim_end_matches('℃');
        match degrees.strip_prefix('-') {
            Some(below) => format!("零下{}摄氏度", low(below)),
            None => format!("{}摄氏度", low(degrees)),
        }
    });
    each_match(&text, number, low)
}

/// Phone numbers, read digit by digit with 一 for 1: an 11-digit mobile number
/// (1, then 3 to 9) and a landline written `0xx-xxxxxxxx`, its hyphen a pause.
fn phone_numbers(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    let mut rest = text;
    let mut prev: Option<char> = None;
    while let Some(c) = rest.chars().next() {
        let starts_here = !prev.is_some_and(|p| p.is_ascii_digit() || matches!(p, '.' | ','));
        if let Some(len) = phone_number(rest).filter(|_| starts_here) {
            let (number, tail) = rest.split_at(len);
            let spoken: Vec<String> = number.split('-').map(direct).collect();
            out.push_str(&spoken.join(" "));
            prev = number.chars().next_back();
            rest = tail;
            continue;
        }
        out.push(c);
        prev = Some(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// How long the phone number `text` starts with is, if it starts with one.
fn phone_number(text: &str) -> Option<usize> {
    let ends_at = |len: usize| !text[len..].starts_with(|c: char| c.is_ascii_digit() || c == '.');
    let bytes = text.as_bytes();
    let run = digits(text);
    if run == 11 && bytes[0] == b'1' && (b'3'..=b'9').contains(&bytes[1]) && ends_at(run) {
        return Some(run);
    }
    if !(3..=4).contains(&run) || bytes[0] != b'0' {
        return None;
    }
    let local = digits(text[run..].strip_prefix('-')?);
    let len = run + 1 + local;
    ((7..=8).contains(&local) && ends_at(len)).then_some(len)
}

/// A hyphen between two amounts is a range, read 到: after a number ending in
/// `%`, `℃` or a measure word ("10%-20%", "5元-10元"), or before one ending in
/// `%` or `℃` ("5-10℃"). cn2an's `range` already reads "3-5个".
fn unit_ranges(text: &str) -> String {
    let ends_in_digit = |text: &str| text.ends_with(|c: char| c.is_ascii_digit());
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        let (before, after) = (&text[..i], &text[i + c.len_utf8()..]);
        let is_range = c == '-'
            && decimal(after).is_some_and(|len| {
                let unit_before = ["%", "℃"]
                    .iter()
                    .chain(MEASURE_WORDS)
                    .any(|unit| before.strip_suffix(unit).is_some_and(ends_in_digit));
                let unit_after = after[len..].starts_with(['%', '℃']);
                unit_before || (unit_after && ends_in_digit(before))
            });
        out.push(if is_range { '到' } else { c });
    }
    out
}

/// Rewrites every leftmost match: `matcher` gives the length of a match
/// starting at a char, if any, given the text before it.
fn each_match(
    text: &str,
    matcher: fn(&str, &str) -> Option<usize>,
    mut rewrite: impl FnMut(&str) -> String,
) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        match matcher(&text[..at], rest).filter(|&len| len > 0) {
            Some(len) => {
                out.push_str(&rewrite(&rest[..len]));
                at += len;
            }
            None => {
                let c = rest.chars().next().map_or(1, char::len_utf8);
                out.push_str(&rest[..c]);
                at += c;
            }
        }
    }
    out
}

fn digits(text: &str) -> usize {
    text.bytes().take_while(u8::is_ascii_digit).count()
}

/// `\d+` with an optional `.\d+` after it.
fn decimal(text: &str) -> Option<usize> {
    let whole = digits(text);
    if whole == 0 {
        return None;
    }
    let fraction = text[whole..]
        .strip_prefix('.')
        .map_or(0, |rest| match digits(rest) {
            0 => 0,
            n => n + 1,
        });
    Some(whole + fraction)
}

/// A leading minus sign, unless a letter or digit comes before it.
fn minus(before: &str, text: &str) -> usize {
    let signed = text.starts_with('-')
        && !before
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    usize::from(signed)
}

/// cn2an's measure words: a range reads "到" only before one of these.
const MEASURE_WORDS: &[&str] = &[
    "斤", "克", "千克", "公斤", "吨", "米", "厘米", "毫米", "公里", "升", "毫升", "元", "角", "分",
    "个", "只", "条", "张", "块", "瓶", "杯", "份", "本", "辆", "台", "匹", "头", "位", "亩",
    "小时", "分钟", "秒", "天", "半",
];

/// `\d+(\.\d+)?-\d+(\.\d+)?` before a measure word.
fn range(_: &str, text: &str) -> Option<usize> {
    let start = decimal(text)?;
    let rest = text[start..].strip_prefix('-')?;
    let end = decimal(rest)?;
    MEASURE_WORDS
        .iter()
        .any(|word| rest[end..].starts_with(word))
        .then_some(start + 1 + end)
}

/// `(\d{2,4}年)?(\d{1,2}月)?(\d{1,2}日)?`, each part only where its digits
/// run up to its character.
fn date(_: &str, text: &str) -> Option<usize> {
    let mut len = 0;
    for (unit, fewest, most) in [('年', 2, 4), ('月', 1, 2), ('日', 1, 2)] {
        let rest = &text[len..];
        let n = digits(rest);
        if (fewest..=most).contains(&n) && rest[n..].starts_with(unit) {
            len += n + unit.len_utf8();
        }
    }
    Some(len)
}

/// `\d+/\d+`.
fn fraction(_: &str, text: &str) -> Option<usize> {
    let numerator = digits(text);
    let rest = text[numerator..].strip_prefix('/')?;
    let denominator = digits(rest);
    (numerator > 0 && denominator > 0).then_some(numerator + 1 + denominator)
}

/// `-?(\d+\.)?\d+%`.
fn percent(before: &str, text: &str) -> Option<usize> {
    let sign = minus(before, text);
    let len = decimal(&text[sign..])?;
    text[sign + len..]
        .starts_with('%')
        .then_some(sign + len + 1)
}

/// `\d+℃`, and its sign and decimals: cn2an reads "-5℃" as -五摄氏度, the sign
/// dropped by Kokoro, and "36.5℃" as 三十六.五摄氏度.
fn celsius(before: &str, text: &str) -> Option<usize> {
    let sign = minus(before, text);
    let len = sign + decimal(&text[sign..])?;
    text[len..].starts_with('℃').then_some(len + '℃'.len_utf8())
}

/// `-?(\d+\.)?\d+`.
fn number(before: &str, text: &str) -> Option<usize> {
    let sign = minus(before, text);
    Some(sign + decimal(&text[sign..])?)
}

/// Digits grouped by commas ("1,500", "10,000,000") without them: the first
/// group one to three digits, not after "digit," nor starting with 0, each
/// group after it exactly three. A grouped number is never a year, so one
/// before 年, 月 or 日 is read here ("1,000年" is 一千年).
fn ungrouped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let first = digits(rest);
        if first == 0 {
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let after_group = out
            .strip_suffix(',')
            .is_some_and(|before| before.ends_with(|c: char| c.is_ascii_digit()));
        let mut number = rest[..first].to_string();
        let mut len = first;
        if first <= 3 && !rest.starts_with('0') && !after_group {
            while rest[len..].starts_with(',') && digits(&rest[len + 1..]) == 3 {
                number.push_str(&rest[len + 1..len + 4]);
                len += 4;
            }
        }
        rest = &rest[len..];
        if len > first && rest.starts_with(['年', '月', '日']) {
            out.push_str(&low(&number));
        } else {
            out.push_str(&number);
        }
    }
    out
}

const NUMERALS: [char; 10] = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
/// The unit of each place, ones first.
const PLACES: [&str; 16] = [
    "", "十", "百", "千", "万", "十", "百", "千", "亿", "十", "百", "千", "万", "十", "百", "千",
];

fn numeral(digit: char) -> char {
    digit.to_digit(10).map_or(digit, |d| NUMERALS[d as usize])
}

/// cn2an's `an2cn` direct mode: each digit, a point as 点.
fn direct(number: &str) -> String {
    let (sign, number) = match number.strip_prefix('-') {
        Some(number) => ("负", number),
        None => ("", number),
    };
    let digits: String = number
        .chars()
        .map(|c| if c == '.' { '点' } else { numeral(c) })
        .collect();
    format!("{sign}{digits}")
}

/// cn2an's `an2cn` low mode: "2024" is 二千零二十四, "3.14" 三点一四.
fn low(number: &str) -> String {
    let (sign, number) = match number.strip_prefix('-') {
        Some(number) => ("负", number),
        None => ("", number),
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let whole = match whole.trim_start_matches('0') {
        "" => "0",
        whole => whole,
    };
    if whole.len() > PLACES.len() {
        return format!("{sign}{}", direct(number));
    }
    let mut out = format!("{sign}{}", whole_number(whole));
    if !fraction.is_empty() {
        out.push('点');
        out.extend(fraction.chars().take(16).map(numeral));
    }
    out
}

/// cn2an's `__integer_convert` for at most 16 digits with no leading zero.
fn whole_number(digits: &str) -> String {
    let len = digits.len();
    let mut out = String::new();
    for (i, d) in digits.chars().enumerate() {
        let place = len - i - 1;
        if d != '0' {
            out.push(numeral(d));
            out.push_str(PLACES[place]);
            continue;
        }
        if place.is_multiple_of(4) {
            out.push('零');
            out.push_str(PLACES[place]);
        }
        if i > 0 && !out.ends_with('零') {
            out.push('零');
        }
    }
    let out = out
        .replace("零零", "零")
        .replace("零万", "万")
        .replace("零亿", "亿")
        .replace("亿万", "亿");
    let chars: Vec<char> = out.trim_matches('零').chars().collect();
    // "万零一千" is said "万一千".
    let mut tidied = String::with_capacity(out.len());
    let mut i = 0;
    while i < chars.len() {
        let skip_zero = i + 3 < chars.len()
            && matches!(chars[i], '万' | '亿')
            && chars[i + 1] == '零'
            && "一二三四五六七八九壹贰叁肆伍陆柒捌玖".contains(chars[i + 2])
            && matches!(chars[i + 3], '千' | '仟');
        tidied.push(chars[i]);
        i += if skip_zero { 2 } else { 1 };
    }
    let tidied = match tidied.strip_prefix("一十") {
        Some(rest) => format!("十{rest}"),
        None => tidied,
    };
    if tidied.is_empty() {
        "零".to_string()
    } else {
        tidied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::local::in_vocab;

    /// Each `input<TAB>expected` line of a fixture, past its `#` header.
    fn fixture(text: &'static str) -> Vec<(&'static str, &'static str)> {
        text.lines()
            .filter(|line| !line.starts_with('#'))
            .map(|line| line.split_once('\t').expect("a tab"))
            .collect()
    }

    fn no_english(text: &str) -> Result<String, String> {
        panic!("no English expected in {text:?}")
    }

    fn reads(text: &str) -> String {
        let (phonemes, skipped) = phonemes(text, no_english).unwrap();
        assert_eq!(skipped, 0, "{text}");
        phonemes
    }

    #[test]
    fn every_syllable_reads_as_misaki_reads_it() {
        let syllables = fixture(include_str!("fixtures/mandarin-syllables.txt"));
        assert_eq!(syllables.len(), 1558);
        let wrong: Vec<_> = syllables
            .iter()
            .filter(|(reading, ipa)| syllable(reading).as_deref() != Some(*ipa))
            .map(|(reading, ipa)| (reading, ipa, syllable(reading)))
            .collect();
        assert!(wrong.is_empty(), "{} wrong: {wrong:?}", wrong.len());
        let outside: Vec<char> = syllables
            .iter()
            .flat_map(|(_, ipa)| ipa.chars())
            .filter(|&c| !in_vocab(c))
            .collect();
        assert!(outside.is_empty(), "{outside:?}");
    }

    #[test]
    fn numbers_read_as_cn2an_writes_them() {
        let wrong: Vec<_> = fixture(include_str!("fixtures/mandarin-numbers.txt"))
            .into_iter()
            .filter(|(text, chinese)| numerals(&halfwidth(text)) != *chinese)
            .map(|(text, chinese)| (text, chinese, numerals(&halfwidth(text))))
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn sentences_read_as_misaki_reads_them() {
        let goldens = fixture(include_str!("fixtures/mandarin-goldens.txt"));
        assert!(goldens.len() >= 60);
        let wrong: Vec<_> = goldens
            .into_iter()
            .filter(|(text, ipa)| reads(text) != *ipa)
            .map(|(text, ipa)| format!("{text}\n  misaki {ipa}\n  mouthy {}", reads(text)))
            .collect();
        assert!(
            wrong.is_empty(),
            "{} differ:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }

    #[test]
    fn the_phrase_data_reads_as_pypinyin_reads_it() {
        let phrases = Phrases::parse(PHRASE_DATA);
        assert_eq!(phrases.0.len(), 47_111);
        for (phrase, readings) in &phrases.0 {
            assert_eq!(
                phrase.chars().count(),
                readings.split_whitespace().count(),
                "{phrase}"
            );
        }
        // Listed twice; pypinyin reads the first.
        assert_eq!(phrases.get("朝阳"), Some("zhāo yáng"));
        assert_eq!(phrases.get("那些"), Some("nà xiē"));
        assert_eq!(phrases.get("银行"), Some("yín háng"));
        // The comment after a reading is not part of it.
        assert_eq!(phrases.get("地藏"), Some("dì zàng"));
    }

    #[test]
    fn a_word_is_cut_into_its_longest_phrases() {
        let phrases =
            Phrases::parse("银行: yín háng\n银行家: yín háng jiā\n行家里手: háng jiā lǐ shǒu\n");
        assert_eq!(phrases.cut("银行家里"), ["银行家", "里"]);
        assert_eq!(phrases.cut("去银行"), ["去", "银行"]);
        // Every prefix of the rest starts a phrase but none is one, so it
        // goes character by character, as pypinyin does.
        assert_eq!(phrases.cut("行家里"), ["行", "家", "里"]);
        assert_eq!(phrases.cut(""), [] as [&str; 0]);
    }

    #[test]
    fn other_scripts_go_to_the_english_voice_spaced_from_the_chinese() {
        // misaki gives each with its English as written and unspaced:
        // "wo↓jʊ↘ŋiPhonepʰai→ꭧau↘." for the first.
        for (text, expected) in [
            ("我用iPhone拍照。", "wo↓jʊ↘ŋ <iPhone> pʰai→ꭧau↘."),
            ("我用 iPhone 拍照", "wo↓jʊ↘ŋ <iPhone> pʰai→ꭧau↘"),
            ("你好，Tom！", "ni↓xau↓, <Tom>!"),
            ("他说“OK”。", "tʰa→ ʂwo→“<OK>”."),
            ("你好，Hello, world！", "ni↓xau↓, <Hello, world>!"),
            ("这是naïve café。", "ꭧɤ↘ʂɨ↘ <naïve café>."),
            (
                "人工智能，AI，正在改变世界。",
                "ɻə↗nkʊ→ŋꭧɨ↘nə↗ŋ, <AI>, ꭧə↘ŋʦai↘ kai↓pjɛ↘n ʂɨ↘ʨje↘.",
            ),
            // Numbers are Chinese first, as misaki has them.
            ("我买了iPhone 15。", "wo↓ mai↓ lɤ <iPhone> ʂɨ↗u↓."),
            // A hyphen Kokoro has no token for stays with the English word.
            ("这是COVID-19。", "ꭧɤ↘ʂɨ↘ <COVID-> ʂɨ↗ʨjou↓."),
        ] {
            assert_eq!(marked(text), expected, "{text}");
        }
    }

    /// Stands in for the English voice.
    fn marked(text: &str) -> String {
        let (phonemes, skipped) = phonemes(text, |english| Ok(format!("<{english}>"))).unwrap();
        assert_eq!(skipped, 0, "{text}");
        phonemes
    }

    #[test]
    fn full_width_letters_and_digits_read_as_ascii_and_full_width_marks_stay() {
        // misaki's v1.0 path passes the letters through, for Kokoro to drop.
        assert_eq!(
            marked("我用ｉＰｈｏｎｅ１５拍照。"),
            marked("我用iPhone15拍照。")
        );
        assert_eq!(marked("你好，ＯＫ！"), "ni↓xau↓, <OK>!");
        assert_eq!(
            halfwidth("ＡＢＣ　１２３，。！？：；"),
            "ABC 123，。！？：；"
        );
    }

    #[test]
    fn a_space_after_a_fullwidth_mark_is_not_doubled() {
        // Sentences joined by the splitter get a space; misaki has one there.
        assert_eq!(reads("你好。 我很好。"), "ni↓xau↓. wo↓ xə↓n xau↓.");
        assert_eq!(reads("「你好」 ，她说。"), reads("「你好」，她说。"));
    }

    #[test]
    fn numbers_misaki_would_leave_as_digits_or_misread() {
        // cn2an gives up past 16 digits and Kokoro drops the digits.
        assert_eq!(
            numerals("有12345678901234567个"),
            "有一二三四五六七八九零一二三四五六七个"
        );
        assert_eq!(
            numerals("-123456789012345678"),
            "负一二三四五六七八九零一二三四五六七八"
        );
        // cn2an: 一,五百 and 十,零,零.
        assert_eq!(numerals("共1,500元"), "共一千五百元");
        assert_eq!(numerals("10,000,000"), "一千万");
        assert_eq!(numerals("1,234.5"), "一千二百三十四点五");
        // A comma groups only exactly three digits, after a first group that
        // neither starts with 0 nor follows "digit,".
        assert_eq!(numerals("1,5000"), "一,五千");
        assert_eq!(numerals("0,500"), "零,五百");
        assert_eq!(numerals("1,23,456"), "一,二十三,四百五十六");
        // cn2an: 一,零零零年, as if a year.
        assert_eq!(numerals("1,000年"), "一千年");
        // cn2an: COVID负十九, 二千零二十四负一负一, 五负三.
        assert_eq!(numerals("COVID-19"), "COVID-十九");
        assert_eq!(numerals("2024-01-01"), "二千零二十四-一-一");
        assert_eq!(numerals("5-3"), "五-三");
        // A minus sign everywhere else.
        assert_eq!(numerals("-5"), "负五");
        assert_eq!(numerals("温度-5度"), "温度负五度");
        assert_eq!(numerals("下降-5%"), "下降百分之负五");
        assert_eq!(numerals("(-3.5)"), "(负三点五)");
    }

    #[test]
    fn ranges_temperatures_and_phone_numbers_read_as_a_newsreader_would() {
        for (text, ours, cn2an) in [
            (
                "增长10%-20%",
                "增长百分之十到百分之二十",
                "增长百分之十百分之负二十",
            ),
            ("5元-10元", "五元到十元", "五元负十元"),
            ("3小时-5小时", "三小时到五小时", "三小时负五小时"),
            ("5-10℃", "五到十摄氏度", "五-十摄氏度"),
            ("气温-5℃", "气温零下五摄氏度", "气温-五摄氏度"),
            ("-0.5℃", "零下零点五摄氏度", "负零.五摄氏度"),
            ("36.5℃", "三十六点五摄氏度", "三十六.五摄氏度"),
            (
                "电话13812345678。",
                "电话一三八一二三四五六七八。",
                "电话一百三十八亿一千二百三十四万五千六百七十八。",
            ),
            (
                "010-12345678",
                "零一零 一二三四五六七八",
                "十负一千二百三十四万五千六百七十八",
            ),
            (
                "0755-1234567",
                "零七五五 一二三四五六七",
                "七百五十五负一百二十三万四千五百六十七",
            ),
        ] {
            assert_eq!(numerals(text), ours, "{text}, cn2an: {cn2an}");
        }
        // Unchanged: cn2an's own range, and numbers that are not phone-shaped.
        assert_eq!(numerals("3-5个"), "三到五个");
        assert_eq!(numerals("10%-"), "百分之十-");
        assert_eq!(
            numerals("12345678901"),
            "一百二十三亿四千五百六十七万八千九百零一"
        );
        assert_eq!(
            numerals("138123456789"),
            "一千三百八十一亿二千三百四十五万六千七百八十九"
        );
        assert_eq!(
            numerals("15812345678.5"),
            "一百五十八亿一千二百三十四万五千六百七十八点五"
        );
        // misaki reads the number whole: "i↘pai↓sa→nʂɨ↗pa→ i↘li↗ŋ …".
        assert_eq!(
            reads("电话号码是13800138000。"),
            "tjɛ↘nxwa↘xau↘ma↓ ʂɨ↘ i→ sa→npa→ li↗ŋli↗ŋ i→ sa→npa→ li↗ŋli↗ŋ li↗ŋ."
        );
    }

    #[test]
    fn a_lone_er_joins_the_word_before_it() {
        for (text, ours, misaki) in [
            (
                "他学画画儿了。",
                "tʰa→ɕɥe↗ xwa↘xwa↘ɚ lɤ.",
                "tʰa→ɕɥe↗ xwa↘xwa↘ ɚ↗ lɤ.",
            ),
            ("等一下儿。", "tə↓ŋ i→ɕja↘ɚ.", "tə↓ŋ i→ɕja↘ ɚ↗."),
            ("那边儿", "na↘pjɛ→nɚ", "na↘pjɛ→n ɚ↗"),
            ("唱歌儿", "ꭧʰa↘ŋkɤ→ɚ", "ꭧʰa↘ŋkɤ→ ɚ↗"),
            ("画画儿，儿", "xwa↘xwa↘ɚ, ɚ↗", "xwa↘xwa↘ ɚ↗, ɚ↗"),
        ] {
            assert_eq!(reads(text), ours, "{text}, misaki: {misaki}");
        }
        // Words jieba knows, a 儿 meaning "son" or "child", and a 儿 that
        // starts a word or follows no Chinese word, read as misaki reads them.
        for (text, misaki) in [
            ("他的儿", "tʰa→ tɤ ɚ↗"),
            ("母亲抱着儿", "mu↓ʨʰi→n pau↘ ꭧɤ ɚ↗"),
            ("与儿", "y↓ ɚ↗"),
            ("吾儿", "u↗ ɚ↗"),
            ("生了三个儿", "ʂə→ŋ lɤ sa→nkɤ↘ ɚ↗"),
            ("我们聊了一会儿，", "wo↓mən ljau↗ lɤ i→xwei↘ɚ,"),
            ("那儿", "na↘ɚ↗"),
            ("女儿", "ny↓ɚ↗"),
            ("儿子来了。", "ɚ↗ʦɨ lai↗ lɤ."),
            ("儿童", "ɚ↗tʰʊ↗ŋ"),
            ("儿", "ɚ↗"),
            ("好，儿", "xau↓, ɚ↗"),
        ] {
            assert_eq!(reads(text), misaki, "{text}");
        }
        assert_eq!(marked("Hi儿"), "<Hi> ɚ↗");
    }

    #[test]
    fn han_misaki_skips_is_read() {
        // misaki reads U+4E00–U+9FFF only and passes these through, for
        // Kokoro to drop: "ɚ↘〇ɚ↘ sɨ↘njɛ↗n" for the first.
        assert_eq!(reads("二〇二四年"), "ɚ↘ li↗ŋ ɚ↘ sɨ↘njɛ↗n");
        assert_eq!(reads("㐀"), "ʨʰjou→");
    }

    #[test]
    fn a_character_with_no_reading_is_skipped_and_counted() {
        // pypinyin reads U+9FFF as xìng; the pinyin crate has no reading.
        assert_eq!(phonemes("鿿", no_english).unwrap(), (String::new(), 1));
        assert_eq!(
            phonemes("你好鿿", no_english).unwrap(),
            ("ni↓xau↓".to_string(), 1)
        );
    }

    #[test]
    fn only_chinese_writing_counts_as_chinese() {
        for text in ["你好", "日本語 and emoji", "。", "二〇二四", "１２３，你好"] {
            assert!(has_chinese(text), "{text}");
        }
        for text in [
            "Hello, world.",
            "こんにちは",
            "123",
            "café",
            "",
            "It costs ￥500",
            "，",
            "１２３",
        ] {
            assert!(!has_chinese(text), "{text}");
        }
    }

    #[test]
    fn empty_and_blank_text_read_as_nothing() {
        for text in ["", " ", "。", "，。"] {
            let (phonemes, skipped) = phonemes(text, no_english).unwrap();
            assert_eq!(skipped, 0);
            assert!(
                phonemes.chars().all(|c| ",. ".contains(c)),
                "{text}: {phonemes:?}"
            );
        }
    }

    /// Chinese, numbers in every form the converter reads, marks of both
    /// widths, Latin and other scripts, at every UTF-8 width.
    const PIECES: &[&str] = &[
        "你", "好", "银行", "重新", "一个", "不是", "这儿", "了", "吗", "〇", "㐀", "𠀀", "鿿",
        "豈", "0", "1", "25", "3.14", "-", "5", "%", "/", "℃", "年", "月", "日", "个", "1,500",
        "１２", "。", "，", "、", "！", "？", "：", "；", "「", "」", "《", "》", "（", "）", "【",
        "】", "“", "”", "…", "—", ".", ",", "!", "?", " ", "\u{3000}", "iPhone", "OK", "naïve",
        "ж", "こ", "🎉", "\u{32f}", "#", "~",
    ];

    #[test]
    fn random_mixed_text_never_panics_and_emits_only_kokoro_symbols_or_its_own_text() {
        let mut rng = crate::tts::text_fuzz_tests::Rng(0x5eed_00c4);
        for _ in 0..3000 {
            let len = rng.below(40);
            let text: String = (0..len).map(|_| PIECES[rng.below(PIECES.len())]).collect();
            let (phonemes, _) = phonemes(&text, |_| Ok("ˈOkA".to_string())).unwrap();
            let foreign: Vec<char> = phonemes
                .chars()
                .filter(|&c| !in_vocab(c) && !text.contains(c))
                .collect();
            assert!(foreign.is_empty(), "{text:?} → {phonemes:?}: {foreign:?}");
        }
    }
}
