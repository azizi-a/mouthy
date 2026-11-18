//! Japanese as Kokoro's Japanese voices were trained to read it: misaki's port
//! of cutlet's kana table (`misaki/cutlet.py`, from polm/cutlet, MIT), applied
//! to each word's kana reading.
//!
//! misaki takes each word and its pronunciation from a dictionary tagger, and
//! so does this with the Japanese dictionary when it is installed. Without it,
//! a word is a run of one script, kanji are skipped, and は, へ and long vowels
//! read as they are spelled.

use super::ja_dictionary::Tagger;
use unicode_normalization::UnicodeNormalization;

/// Longer sentences are cut before this. Prose mostly in kanji runs about
/// three ids a character with the dictionary, so its chunks fit one
/// inference; numbers, runs of `き`-like kana and alternating scripts can pass
/// 510 ids, and `split_tokens` then cuts the chunk, mid-word where it has no
/// pause or space.
pub(super) const MAX_CHUNK_CHARS: usize = 150;

/// Without the dictionary, above this share of the text's letters in kanji so
/// much would be skipped that it is refused instead.
const MAX_KANJI_PERCENT: usize = 20;

/// Short enough for the overlay's two lines after "Speech failed:".
const KANJI_REFUSAL: &str = "Kanji need the dictionary in Settings → Models";

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

/// The notice for `text` when too much of it is kanji to read without the
/// dictionary.
pub(super) fn refusal(text: &str) -> Option<&'static str> {
    // As it will be read: NFKC turns "㍿" into four kanji.
    let text = normalize(text);
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let kanji = text.chars().filter(|&c| is_kanji(c)).count();
    (kanji * 100 > letters * MAX_KANJI_PERCENT).then_some(KANJI_REFUSAL)
}

/// Kokoro's symbols for `text`, with its words found by `tagger` when there is
/// one and its English read by `english`, and how many kanji were skipped.
pub(super) fn phonemes(
    text: &str,
    tagger: Option<&Tagger>,
    mut english: impl FnMut(&str) -> Result<String, String>,
) -> Result<(String, usize), String> {
    let text = normalize(text);
    let (words, skipped) = match tagger {
        Some(tagger) => tagged_words(&text, tagger)?,
        None => words(&text),
    };
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

/// The words of `text` as the dictionary splits and reads them, with the
/// English and numbers in it found as `words` finds them, and how many kanji
/// had no reading.
fn tagged_words(text: &str, tagger: &Tagger) -> Result<(Vec<Word>, usize), String> {
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut skipped = 0;
    let mut run = 0;
    let mut i = 0;
    while i < chars.len() {
        let english = (!chars[i].is_whitespace())
            .then(|| english_end(&chars, i))
            .flatten();
        match english {
            Some(end) => {
                skipped += tag(&chars[run..i], tagger, &mut words)?;
                words.push(Word {
                    surface: chars[i..end].iter().collect(),
                    kind: Kind::English,
                });
                run = end;
                i = end;
            }
            None => i += 1,
        }
    }
    skipped += tag(&chars[run..], tagger, &mut words)?;
    Ok((words, skipped))
}

/// Reads `run`, text with no English in it, word by word through the
/// dictionary. A number reads as misaki's `num2kana` reads it, while the
/// dictionary still sees its digits, so the counter after it reads as one:
/// "3人" is さんにん, where the kana "さん人" would read さんじん.
fn tag(run: &[char], tagger: &Tagger, words: &mut Vec<Word>) -> Result<usize, String> {
    if run.iter().all(|c| c.is_whitespace()) {
        return Ok(0);
    }
    // The dictionary drops tabs and line breaks, which would shift every word
    // after them.
    let spaced: String = run
        .iter()
        .map(|&c| {
            if c.is_whitespace() || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let nodes = tagger
        .text_to_njd(&spaced)
        .map_err(|e| format!("Cannot read the Japanese text: {e}"))?
        .nodes;
    // The dictionary sees one character for each of `run`'s, but a sound mark
    // joins the kana before it.
    let starts: Vec<usize> = (0..run.len()).filter(|&i| !is_sound_mark(run[i])).collect();
    let mut seen = 0;
    let spans: Vec<(usize, usize)> = nodes
        .iter()
        .map(|node| {
            let from = starts.get(seen).copied().unwrap_or(run.len());
            seen += node.get_string().chars().count();
            (from, starts.get(seen).copied().unwrap_or(run.len()))
        })
        .collect();
    let readings: Vec<Option<String>> = nodes
        .iter()
        .map(|node| {
            kana_reading(
                &node.get_pron().to_string(),
                node.get_read(),
                node.get_string(),
            )
        })
        .collect();
    // The dictionary tags numbers 名詞,数: digits, and kanji such as 一, 十 or 百.
    let numerals: Vec<bool> = nodes
        .iter()
        .map(|node| node.get_pos().to_string().starts_with("名詞,数"))
        .collect();
    let numbers = number_spans(run);
    let mut skipped = 0;
    let mut done = 0;
    // Where the last word this call read ends: a split-off う joins only a word
    // ending where it starts. Words a nested `tag` reads leave it behind them,
    // so a う after those never joins.
    let mut last_end = None;
    // Where the last month named ends: 1日 right after it is ついたち.
    let mut month_end = None;
    let after_month = |month_end: Option<usize>, at: usize| {
        month_end.is_some_and(|month| {
            run.get(month..at)
                .is_some_and(|gap| gap.iter().all(|c| c.is_whitespace()))
        })
    };
    // What comes before the number `value` starting at `at`, in node `k`, with
    // its counter at `end`.
    let context = |k: usize, at: usize, end: usize, value: &str, month_end: Option<usize>| {
        // 1、2人目 and 一、二人とも list ordinals or say "both".
        let ordinal = run
            .get(end + 1..)
            .is_some_and(|rest| rest.starts_with(&['目']) || rest.starts_with(&['と', 'も']));
        let range = (spans[k].0 == at && !ordinal)
            .then(|| range_start(run, &spans, &numerals, &numbers, k, value.parse().ok()?))
            .flatten();
        match range {
            // Days of the month named before the first number: 5月3、4日.
            Some(first) if after_month(month_end, first) => Before::Other,
            Some(_) => Before::Number,
            None if after_month(month_end, at) => Before::Month,
            None => Before::Other,
        }
    };
    for (k, &(from, to)) in spans.iter().enumerate() {
        if to <= done || from == to {
            continue;
        }
        let start = from.max(done);
        let number = numbers
            .iter()
            .find(|&&(number, end)| number < to && end > start);
        match number {
            None if start == from => {
                let counted = kanji_count(run, &spans, &numerals, k).and_then(|(end, value)| {
                    let before = context(k, from, end, &value, month_end);
                    // A count starts here only after a word that is not a
                    // numeral, or after a comma the dictionary tags 名詞,数 in
                    // an approximate count: 二，三日.
                    (k == 0 || !numerals[k - 1] || before == Before::Number).then_some(())?;
                    let word = counted_word(run, &spans, &readings, end, &value, before);
                    Some((end, word?))
                });
                if let Some((end, (counted_end, reading))) = counted {
                    if run[end] == '月' {
                        month_end = Some(counted_end);
                    }
                    words.push(Word {
                        surface: run[from..counted_end].iter().collect(),
                        kind: Kind::Kana(reading),
                    });
                    last_end = Some(counted_end);
                    done = counted_end;
                    continue;
                }
                if names_month(&run[from..to], readings[k].as_deref()) {
                    month_end = Some(to);
                }
                // The dictionary makes the end of ましょう, 行こう, でしょう or
                // だろう a word う of its own; it lengthens the word read just
                // before it.
                let joined = run[from..to] == ['う']
                    && readings[k].is_some()
                    && last_end == Some(from)
                    && lengthen(words.last_mut());
                let count = words.len();
                if !joined {
                    skipped += read(&run[from..to], readings[k].clone(), words);
                }
                if joined || words.len() > count {
                    last_end = Some(to);
                }
            }
            // What is left of a word that ran on from a number.
            None => skipped += tag(&run[start..to], tagger, words)?,
            Some(&(number, end)) => {
                if start < number {
                    skipped += tag(&run[start..number], tagger, words)?;
                }
                // A dictionary word holding the number and more reads as one:
                // ４月, ２つ, １人暮らし.
                let whole = readings[k]
                    .as_ref()
                    .filter(|_| start == number && from == number && to > end);
                if let Some(reading) = whole {
                    if names_month(&run[from..to], Some(reading)) {
                        month_end = Some(to);
                    }
                    words.push(Word {
                        surface: run[from..to].iter().collect(),
                        kind: Kind::Kana(reading.clone()),
                    });
                    last_end = Some(to);
                    done = to;
                    continue;
                }
                let digits: String = run[number..end].iter().collect();
                let before = context(k, number, end, &digits, month_end);
                if let Some((counted_end, reading)) =
                    counted_word(run, &spans, &readings, end, &digits, before)
                {
                    if run[end] == '月' {
                        month_end = Some(counted_end);
                    }
                    words.push(Word {
                        surface: run[number..counted_end].iter().collect(),
                        kind: Kind::Kana(reading),
                    });
                    last_end = Some(counted_end);
                    done = counted_end;
                    continue;
                }
                // The dictionary says じゅう and きゅう long, as misaki's
                // tagger reads its numbers.
                let reading = number_kana(&digits).replace("ゅう", "ゅー");
                words.push(Word {
                    kind: Kind::Kana(reading.chars().map(to_hiragana).collect()),
                    surface: reading,
                });
                last_end = Some(end);
                if end < to {
                    skipped += tag(&run[end..to], tagger, words)?;
                }
            }
        }
        done = to.max(number.map_or(to, |&(_, end)| end));
    }
    Ok(skipped)
}

/// Where the kanji numerals from node `k` end and the number they write, when
/// it is 一 to 九十九: 三, 十四, 二十.
fn kanji_count(
    run: &[char],
    spans: &[(usize, usize)],
    numerals: &[bool],
    k: usize,
) -> Option<(usize, String)> {
    let last = (k..spans.len()).take_while(|&j| numerals[j]).last()?;
    let end = spans[last].1;
    Some((end, kanji_value(&run[spans[k].0..end])?.to_string()))
}

/// The number kanji numerals write, when it is 一 to 九十九.
fn kanji_value(kanji: &[char]) -> Option<u32> {
    let digit = |c: &char| {
        "一二三四五六七八九"
            .chars()
            .position(|d| d == *c)
            .map(|i| i as u32 + 1)
    };
    Some(match kanji {
        ['十'] => 10,
        ['十', one] => 10 + digit(one)?,
        [ten, '十'] => digit(ten)? * 10,
        [ten, '十', one] => digit(ten)? * 10 + digit(one)?,
        [one] => digit(one)?,
        _ => return None,
    })
}

/// Where an approximate count such as 二、三 or 十四、五 starts, when node `k`
/// starts its last number, `n`, and each number is the one after the number
/// and comma before it.
fn range_start(
    run: &[char],
    spans: &[(usize, usize)],
    numerals: &[bool],
    numbers: &[(usize, usize)],
    k: usize,
    n: u32,
) -> Option<usize> {
    let (mut first, mut n) = (k, n);
    while let Some((from, before)) = number_before(run, spans, numerals, numbers, first) {
        if !follows(before, n) {
            break;
        }
        (first, n) = (from, before);
    }
    (first < k).then(|| spans[first].0)
}

/// The number written before node `k` and a comma: its first node and value.
/// Digits count only as the whole number `numbers` found, so the 001 of
/// 1,001 is not one.
fn number_before(
    run: &[char],
    spans: &[(usize, usize)],
    numerals: &[bool],
    numbers: &[(usize, usize)],
    k: usize,
) -> Option<(usize, u32)> {
    let comma = k.checked_sub(1).filter(|&j| is_comma(run, spans[j]))?;
    let last = comma.checked_sub(1).filter(|&j| numerals[j])?;
    let mut first = last;
    // The dictionary tags a comma between digits 名詞,数 too.
    while first > 0 && numerals[first - 1] && !is_comma(run, spans[first - 1]) {
        first -= 1;
    }
    let (from, to) = (spans[first].0, spans[last].1);
    let number = &run[from..to];
    let value = if number.iter().all(char::is_ascii_digit) {
        numbers.contains(&(from, to)).then_some(())?;
        number.iter().collect::<String>().parse().ok()?
    } else {
        kanji_value(number)?
    };
    Some((first, value))
}

/// Whether the word at `span` is 、 or a comma, which can join two numbers
/// into an approximate count. 〜 and から are said, and ・ can be a decimal
/// point.
fn is_comma(run: &[char], (from, to): (usize, usize)) -> bool {
    let mark: String = run[from..to].iter().collect();
    matches!(mark.trim(), "、" | ",")
}

/// Whether `n` follows `before` in an approximate count: 二、三, 9、10, or
/// 十四、五 for 14 or 15. 19、20 is two numbers.
fn follows(before: u32, n: u32) -> bool {
    match before {
        0..=9 => n == before + 1,
        _ => (1..=8).contains(&(before % 10)) && n == before % 10 + 1,
    }
}

/// Words that name a month, so 1日 after them is a day of it: ４月, 十二月,
/// 毎月. 1ヶ月, a length of time, is not one.
fn names_month(surface: &[char], reading: Option<&str>) -> bool {
    let surface: String = surface.iter().collect();
    ["毎月", "今月", "来月", "先月", "翌月", "同月", "前月"].contains(&surface.as_str())
        || surface.ends_with('月') && reading.is_some_and(|reading| reading.ends_with("がつ"))
}

/// The number before `end` of `run`, whose value is written `value`, and the
/// counter at `end` as one word, when the counter changes how the number
/// sounds: where the word ends and its reading. The rest of the dictionary
/// word holding the counter keeps its reading (日間 is ...かん); after ひとり
/// and ふたり only a few words do (二人組, but 一人前 is いちにんまえ).
fn counted_word(
    run: &[char],
    spans: &[(usize, usize)],
    readings: &[Option<String>],
    end: usize,
    value: &str,
    before: Before,
) -> Option<(usize, String)> {
    let counter = *run.get(end)?;
    let mut reading: String = counted(value, counter, before)?
        .chars()
        .map(to_hiragana)
        .collect::<String>()
        .replace("ゅう", "ゅー");
    let held = spans
        .iter()
        .position(|&(from, to)| from <= end && end < to)?;
    let (from, mut to) = spans[held];
    if to > end + 1 {
        if from != end {
            return None;
        }
        let rest: String = run[end + 1..to].iter().collect();
        if counter == '人' && matches!(value.parse(), Ok(1 | 2)) && rest != "組" {
            return None;
        }
        let rest = readings[held]
            .as_deref()?
            .strip_prefix(counter_kana(counter))?;
        reading.push_str(rest);
    }
    // 一日中: 中 after a count of days is じゅう.
    if counter == '日' && run.get(to) == Some(&'中') && spans.contains(&(to, to + 1)) {
        reading.push_str("じゅー");
        to += 1;
    }
    Some((to, reading))
}

/// What comes right before a count, which can change how it reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Before {
    /// A word naming a month: 1日 is ついたち.
    Month,
    /// The number before it and a comma, in an approximate count: 二、三日 is
    /// にさんにち.
    Number,
    Other,
}

/// How a whole number reads before 月, 日, 人 or つ, or `None` when the
/// counter leaves it as it is.
fn counted(number: &str, counter: char, before: Before) -> Option<String> {
    let n: u32 = number.parse().ok()?;
    let reading = match (counter, n) {
        ('月', 4) => "しがつ",
        ('月', 7) => "しちがつ",
        ('月', 9) => "くがつ",
        ('月', 1..=12) => return Some(number_kana(number) + "がつ"),
        ('日', _) => return Some(days(n, &number_kana(number), before)),
        // An approximate count has いちにん and ににん: 一、二人.
        ('人', 1) if before != Before::Number => "ひとり",
        ('人', 2) if before != Before::Number => "ふたり",
        // よにん, and so じゅうよにん, but よんじゅうにん.
        ('人', _) => {
            let kana = number_kana(number);
            let kana = kana
                .strip_suffix("よん")
                .map_or(kana.clone(), |kana| kana.to_string() + "よ");
            return Some(kana + "にん");
        }
        ('つ', 1..=9) => [
            "ひとつ",
            "ふたつ",
            "みっつ",
            "よっつ",
            "いつつ",
            "むっつ",
            "ななつ",
            "やっつ",
            "ここのつ",
        ][n as usize - 1],
        _ => return None,
    };
    Some(reading.to_string())
}

/// `n` days, or day `n` of a month, where `n` reads `kana` on its own.
fn days(n: u32, kana: &str, before: Before) -> String {
    // よっか, じゅうよっか, ひゃくよっか, in an approximate count too: よんにち
    // is not said.
    if let Some(rest) = kana.strip_suffix("よん") {
        return format!("{rest}よっか");
    }
    let word = match n {
        1 if before == Before::Month => "ついたち",
        2 => "ふつか",
        3 => "みっか",
        5 => "いつか",
        6 => "むいか",
        7 => "なのか",
        // Long with ー, as the dictionary says them: the kana table reads ようか
        // with a separate う.
        8 => "よーか",
        9 => "ここのか",
        10 => "とーか",
        20 => "はつか",
        _ => "",
    };
    // The second number of an approximate count keeps the plain number:
    // 二、三日 is にさんにち.
    if !word.is_empty() && before != Before::Number {
        return word.to_string();
    }
    // A day of the month says 7 しち and 9 く, and so does an approximate
    // count: 六、七日.
    let tens = || match n / 10 {
        0 => String::new(),
        tens => number_kana(&(tens * 10).to_string()),
    };
    match n {
        7 | 17 | 27 => tens() + "しちにち",
        9 | 19 | 29 => tens() + "くにち",
        _ => format!("{kana}にち"),
    }
}

/// What the dictionary reads a counter as after a number.
fn counter_kana(counter: char) -> &'static str {
    match counter {
        '月' => "がつ",
        '日' => "にち",
        '人' => "にん",
        _ => "つ",
    }
}

/// Takes a split-off う into `word` as a long vowel, when it ends in one that
/// takes it.
fn lengthen(word: Option<&mut Word>) -> bool {
    let Some(Word {
        surface,
        kind: Kind::Kana(reading),
    }) = word
    else {
        return false;
    };
    let lengthens = reading.ends_with(|c| {
        "おこごそぞとどのほぼぽもよょろをぉうくぐすずつづぬふぶぷむゆゅるぅゔ".contains(c)
    });
    if lengthens {
        reading.push('ー');
        surface.push('う');
    }
    lengthens
}

/// One dictionary word written `surface`: read from `reading`; with none,
/// skipped when it has kanji, and as its symbols otherwise.
fn read(surface: &[char], reading: Option<String>, words: &mut Vec<Word>) -> usize {
    if let Some(reading) = reading {
        words.push(Word {
            surface: surface.iter().collect(),
            kind: Kind::Kana(reading),
        });
        return 0;
    }
    let kanji = surface.iter().filter(|&&c| is_kanji(c)).count();
    if kanji == 0 {
        words.extend(
            surface
                .iter()
                .filter(|c| !c.is_whitespace())
                .map(|&c| Word {
                    surface: c.to_string(),
                    kind: Kind::Symbol,
                }),
        );
    }
    kanji
}

/// cutlet reads a word from its `pron`, else its `kana`, else as written: here
/// the first of the dictionary's pronunciation, its reading and the written
/// word that is all kana, in hiragana.
fn kana_reading(pronunciation: &str, reading: Option<&str>, written: &str) -> Option<String> {
    // `’` marks a mora said without voice; Kokoro has no symbol for that.
    let pronunciation: String = pronunciation.chars().filter(|&c| c != '’').collect();
    let kana = [pronunciation.as_str(), reading.unwrap_or_default(), written]
        .into_iter()
        .find(|kana| !kana.is_empty() && kana.chars().all(|c| script(c).is_some()))?;
    Some(kana.chars().map(to_hiragana).collect())
}

/// Where each number in `chars` starts and ends.
fn number_spans(chars: &[char]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let end = number_end(chars, i);
            spans.push((i, end));
            i = end;
        } else {
            i += 1;
        }
    }
    spans
}

/// The voiced and semi-voiced sound marks, combining or not, full or half width.
fn is_sound_mark(c: char) -> bool {
    matches!(c, '\u{3099}'..='\u{309c}' | '\u{ff9e}' | '\u{ff9f}')
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

/// Ordinary prose, mostly kanji.
#[cfg(test)]
pub(super) const KANJI_PROSE: &str = "昨日の夜、駅前の小さな本屋で、昔から読みたかった小説を見つけました。店の主人は七十歳を過ぎていて、この町で五十年以上も店を続けているそうです。最近は大きな書店やインターネットで本を買う人が増えたので、客は少なくなったと話していました。それでも、毎朝早く店を開けて、新しく入った本を一冊ずつ丁寧に棚に並べるのが楽しみだと笑っていました。政府は今年度の経済見通しを発表し、物価の上昇が続く中で、個人消費の回復は緩やかなものにとどまるとの見方を示しました。";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::local::in_vocab;

    /// Reads without the dictionary, with a stand-in for the English voice, as
    /// the expected strings were made.
    fn read(text: &str) -> (String, usize) {
        phonemes(text, None, |latin| Ok(format!("<{latin}>"))).unwrap()
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
        let (phonemes, _) = phonemes(&text, None, |_| Ok(String::new())).unwrap();
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
        // A squared form counts as the kanji it is read as: ㍿ is 株式会社.
        assert_eq!(refusal("㍿あいうえおかきくけこさしすせそた"), None);
        assert_eq!(
            refusal("㍿あいうえおかきくけこさしすせそ"),
            Some(KANJI_REFUSAL)
        );
    }

    // With the dictionary, when the test run has it (`tagger_for_test`).
    // Readings are NAIST-jdic's pronunciations, checked against the standard
    // ones; the symbols follow from cutlet's table as the tests above check
    // it. misaki's own tagger uses UniDic, which is not installed here, so
    // these are not misaki's output.

    use crate::tts::ja_dictionary::tagger_for_test;

    /// Reads with the dictionary and a stand-in for the English voice.
    fn tagged_with(tagger: &Tagger, text: &str) -> (String, usize) {
        phonemes(text, Some(tagger), |latin| Ok(format!("<{latin}>"))).unwrap()
    }

    fn reads_with(tagger: &Tagger, text: &str) -> String {
        let (phonemes, skipped) = tagged_with(tagger, text);
        assert_eq!(skipped, 0, "{text}");
        phonemes
    }

    #[test]
    fn kanji_read_as_the_dictionary_says_them() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        // トーキョー ニ イキ マス
        assert_eq!(reads_tagged("東京に行きます。"), "toːkʲoː ɲi ikʲi masɨ.");
        // キョー ワ テンキ ガ イイ デス ネ
        assert_eq!(
            reads_tagged("今日は天気がいいですね。"),
            "kʲoː βa teŋkʲi ɡa ii desɨ ne."
        );
        // ワタシ ワ ガクセー デス
        assert_eq!(reads_tagged("私は学生です。"), "βataɕi βa ɡakɯseː desɨ.");
        // ニホンゴ ヲ ベンキョー シ テ イ マス, where ヲ reads `o`
        assert_eq!(
            reads_tagged("日本語を勉強しています。"),
            "ɲihoŋɡo o beŋkʲoː ɕi te i masɨ."
        );
        assert_eq!(
            reads_tagged("先生、ありがとうございました。"),
            "seɴseː, aɾʲiɡatoː ɡoʣaimaɕita."
        );
        assert_eq!(reads_tagged("人々"), "çitobʲito");
    }

    #[test]
    fn particles_read_as_they_are_said() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        assert_eq!(reads_tagged("学校へ行く。"), "ɡaʔkoː e ikɯ.");
        assert_eq!(reads_tagged("本を読む。"), "hoɴ o jomɯ.");
        assert_eq!(reads_tagged("これはペンです"), "koɾe βa peɴ desɨ");
        assert_eq!(reads_tagged("こんにちは"), "koɲɲiʨiβa");
        // Kana the dictionary knows read as it says them too.
        assert_eq!(
            reads_tagged("きょうもいいてんきですね。"),
            "kʲoː mo ii teŋkʲi desɨ ne."
        );
    }

    #[test]
    fn numbers_read_with_num2kana_and_the_counter_after_them_as_a_counter() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        assert_eq!(reads_tagged("3人"), "saɲɲiɴ");
        assert_eq!(reads_tagged("2024年"), "ɲiseɲɲiʥɨːjoɴ neɴ");
        assert_eq!(reads_tagged("10時30分"), "ʥɨː ʥi saɲʥɨː ɸɯɴ");
        assert_eq!(
            reads_tagged("3人で10時30分に会いました。"),
            "saɲɲiɴ de ʥɨː ʥi saɲʥɨː ɸɯɴ ɲi ai maɕi ta."
        );
        assert_eq!(reads_tagged("1,500円"), "seŋɡoçakɯ eɴ");
        assert_eq!(reads_tagged("3.5キロ"), "santeŋɡo kʲiɾo");
        assert_eq!(reads_tagged("50%です"), "ɡoʥɨː paːsento desɨ");
        assert_eq!(reads_tagged("5〜10"), "ɡo kaɾa ʥɨː");
    }

    #[test]
    fn months_days_people_and_things_change_how_a_number_sounds() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        // しがつ と くがつ と しちがつ
        assert_eq!(
            reads_tagged("4月と9月と7月"),
            "ɕiɡaʦɨ to kɯɡaʦɨ to ɕiʨiɡaʦɨ"
        );
        // にせんにじゅうよねん じゅうにがつ みっか, with num2kana's よん
        assert_eq!(
            reads_tagged("２０２４年１２月３日"),
            "ɲiseɲɲiʥɨːjoɴ neɴ ʥɨːɲiɡaʦɨ mʲiʔka"
        );
        // ついたち right after a month
        assert_eq!(
            reads_tagged("2024年4月1日から"),
            "ɲiseɲɲiʥɨːjoɴ neɴ ɕiɡaʦɨ ʦɨitaʨi kaɾa"
        );
        assert_eq!(reads_tagged("1人で"), "çitoɾʲi de");
        assert_eq!(reads_tagged("2人で"), "ɸɯtaɾʲi de");
        assert_eq!(reads_tagged("4人"), "joɲiɴ");
        assert_eq!(reads_tagged("10人"), "ʥɨːɲiɴ");
        assert_eq!(reads_tagged("りんごを2つ"), "ɾʲiŋɡo o ɸɯtaʦɨ");
        // みっかかん: the dictionary's 日間 keeps its かん
        assert_eq!(reads_tagged("3日間"), "mʲiʔkakaɴ");
        // いちにちじゅう: 1日 away from a month, and 中 after it
        assert_eq!(reads_tagged("1日中"), "iʨiɲiʨiʥɨː");
        assert_eq!(reads_tagged("14日"), "ʥɨːjoʔka");
        assert_eq!(reads_tagged("20日"), "haʦɨka");
        // じゅうしちにち, にじゅうくにち
        assert_eq!(reads_tagged("17日"), "ʥɨːɕiʨiɲiʨi");
        assert_eq!(reads_tagged("19日"), "ʥɨːkɯɲiʨi");
        assert_eq!(reads_tagged("27日"), "ɲiʥɨːɕiʨiɲiʨi");
        assert_eq!(reads_tagged("29日"), "ɲiʥɨːkɯɲiʨi");
        // じゅうよにん, ひゃくよにん, but よんじゅうにん
        assert_eq!(reads_tagged("14人"), "ʥɨːjoɲiɴ");
        assert_eq!(reads_tagged("24人"), "ɲiʥɨːjoɲiɴ");
        assert_eq!(reads_tagged("104人"), "çakɯjoɲiɴ");
        assert_eq!(reads_tagged("40人"), "joɲʥɨːɲiɴ");
        // さんじゅうよっか, ひゃくよっか, よんじゅうよっか, but よんじゅうにち
        assert_eq!(reads_tagged("34日"), "saɲʥɨːjoʔka");
        assert_eq!(reads_tagged("104日"), "çakɯjoʔka");
        assert_eq!(reads_tagged("44日"), "joɲʥɨːjoʔka");
        assert_eq!(reads_tagged("40日"), "joɲʥɨːɲiʨi");
        // ぜろにん, ぜろにち
        assert_eq!(reads_tagged("0人"), "ʣeɾoɲiɴ");
        assert_eq!(reads_tagged("0日"), "ʣeɾoɲiʨi");
        // Not a month, or not a whole number: the number as it is.
        assert_eq!(reads_tagged("13月"), "ʥɨːsaɴ ʦɨkʲi");
        assert_eq!(reads_tagged("1.5人"), "iʨiteŋɡo ɲiɴ");
    }

    #[test]
    fn the_first_comes_after_a_month_and_words_naming_one() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        // まいつき ついたち, らいげつ ついたち
        assert_eq!(reads_tagged("毎月1日"), "maiʦɨkʲi ʦɨitaʨi");
        assert_eq!(reads_tagged("来月1日"), "ɾaiɡeʦɨ ʦɨitaʨi");
        assert_eq!(reads_tagged("1月 1日"), "iʨiɡaʦɨ ʦɨitaʨi");
        // 1ヶ月 is a length of time, so いちにち after it
        assert_eq!(reads_tagged("1ヶ月1日"), "iʨi kaɡeʦɨ iʨiɲiʨi");
    }

    #[test]
    fn one_or_two_people_inside_a_longer_word_read_as_the_dictionary_says() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        // いちにんしょう, ににんしょう, いちにんまえ
        assert_eq!(reads_tagged("1人称"), "iʨi ɲiɴɕoː");
        assert_eq!(reads_tagged("2人称"), "ɲi ɲiɴɕoː");
        assert_eq!(reads_tagged("1人前"), "iʨi ɲimmae");
        // ふたりぐみ, ひとり たび, ふたり とも
        assert_eq!(reads_tagged("2人組"), "ɸɯtaɾʲiɡɯmʲi");
        assert_eq!(reads_tagged("1人旅"), "çitoɾʲi tabʲi");
        assert_eq!(reads_tagged("2人とも"), "ɸɯtaɾʲi tomo");
        // ひとりぐらし: one dictionary word holds the number
        assert_eq!(reads_tagged("1人暮らし"), "çitoɾʲiɡɯɾaɕi");
    }

    #[test]
    fn a_count_in_kanji_reads_as_one_in_digits() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        // ひとり, ふたり, よにん, じゅうよにん
        assert_eq!(reads_tagged("一人"), "çitoɾʲi");
        assert_eq!(reads_tagged("二人"), "ɸɯtaɾʲi");
        assert_eq!(reads_tagged("四人"), "joɲiɴ");
        assert_eq!(reads_tagged("十四人"), "ʥɨːjoɲiɴ");
        // みっか, ようか, とおか, はつか, じゅうよっか, にじゅうくにち
        assert_eq!(reads_tagged("三日"), "mʲiʔka");
        assert_eq!(reads_tagged("八日"), "joːka");
        assert_eq!(reads_tagged("十日"), "toːka");
        assert_eq!(reads_tagged("二十日"), "haʦɨka");
        assert_eq!(reads_tagged("十四日"), "ʥɨːjoʔka");
        assert_eq!(reads_tagged("二十九日"), "ɲiʥɨːkɯɲiʨi");
        // さんじゅうよっか, よんじゅうよっか, but よんじゅうにち
        assert_eq!(reads_tagged("三十四日"), "saɲʥɨːjoʔka");
        assert_eq!(reads_tagged("四十四日"), "joɲʥɨːjoʔka");
        assert_eq!(reads_tagged("四十日"), "joɲʥɨːɲiʨi");
        // いちがつ ついたち, but いちにち on its own
        assert_eq!(reads_tagged("一月一日"), "iʨiɡaʦɨ ʦɨitaʨi");
        assert_eq!(reads_tagged("一日"), "iʨiɲiʨi");
        // Words of their own keep the dictionary's reading: いちにちじゅう,
        // いちにんまえ, ににんしょう, みかづき, とおかまち, はつかだいこん.
        assert_eq!(reads_tagged("一日中"), "iʨiɲiʨiʥɨː");
        assert_eq!(reads_tagged("一人前"), "iʨi ɲimmae");
        assert_eq!(reads_tagged("二人称"), "ɲiɲiɴɕoː");
        assert_eq!(reads_tagged("三日月"), "mʲikazɨkʲi");
        assert_eq!(reads_tagged("十日町"), "toːkamaʨi");
        assert_eq!(reads_tagged("二十日大根"), "haʦɨkadaikoɴ");
        // The dictionary already reads these: しがつ, しちがつ, くがつ, ひとつ, ふたつ.
        assert_eq!(reads_tagged("四月"), "ɕiɡaʦɨ");
        assert_eq!(reads_tagged("七月"), "ɕiʨiɡaʦɨ");
        assert_eq!(reads_tagged("九月"), "kɯɡaʦɨ");
        assert_eq!(reads_tagged("一つ"), "çitoʦɨ");
        assert_eq!(reads_tagged("二つ"), "ɸɯtaʦɨ");
        // ひゃくいちにん: a count with 百 in it stays as the dictionary reads it.
        assert_eq!(reads_tagged("百一人"), "çakɯ iʨi ɲiɴ");
        // Numerals inside words: いちばん, いっしょ, じゅうぶん, きゅうしゅう, いちろう.
        assert_eq!(reads_tagged("一番"), "iʨibaɴ");
        assert_eq!(reads_tagged("一緒"), "iʔɕo");
        assert_eq!(reads_tagged("十分"), "ʥɨːbɯɴ");
        assert_eq!(reads_tagged("九州"), "kʲɨːɕɨː");
        assert_eq!(reads_tagged("一郎"), "iʨiɾoː");
    }

    #[test]
    fn every_count_to_ninety_nine_reads_alike_in_kanji_and_digits() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: String| reads_with(tagger, &text);
        let kanji = |n: usize| {
            let digit = |d: usize| "一二三四五六七八九".chars().nth(d - 1);
            let mut kanji = String::new();
            if n >= 20 {
                kanji.extend(digit(n / 10));
            }
            if n >= 10 {
                kanji.push('十');
            }
            if !n.is_multiple_of(10) {
                kanji.extend(digit(n % 10));
            }
            kanji
        };
        for n in 1..=99 {
            for counter in ["日", "人"] {
                assert_eq!(
                    reads_tagged(format!("{}{counter}", kanji(n))),
                    reads_tagged(format!("{n}{counter}")),
                    "{n}{counter}"
                );
                // After the number before it, and after a non-count, 三.
                let mut pairs = vec![(n - 1, n), (3, n)];
                // 十四、五: the last digit after it.
                if (11..=98).contains(&n) && (1..=8).contains(&(n % 10)) {
                    pairs.push((n, n % 10 + 1));
                }
                for (before, n) in pairs {
                    // The dictionary keeps 二日 as a word of its own, ふつか.
                    if before == 0 || (n, counter) == (2, "日") {
                        continue;
                    }
                    // The count after the comma: the dictionary splits a first
                    // number such as 十一, which has no counter, into words.
                    let count = |text: String| {
                        let reading = reads_tagged(text);
                        reading.split_once(", ").unwrap().1.to_string()
                    };
                    assert_eq!(
                        count(format!("{}、{}{counter}", kanji(before), kanji(n))),
                        count(format!("{before}、{n}{counter}")),
                        "{before}、{n}{counter}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_approximate_count_reads_its_second_number_plainly() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        // に、さんにち: two or three days is not みっか.
        for text in ["二、三日", "2、3日", "二，三日", "2,3日", "2, 3日"] {
            assert_eq!(reads_tagged(text), "ɲi, saɲɲiʨi", "{text}");
        }
        // に、さんにちじゅう に; に、さんにちかん
        assert_eq!(reads_tagged("二、三日中に"), "ɲi, saɲɲiʨiʥɨː ɲi");
        assert_eq!(reads_tagged("2、3日中に"), "ɲi, saɲɲiʨiʥɨː ɲi");
        assert_eq!(reads_tagged("2、3日間"), "ɲi, saɲɲiʨikaɴ");
        // よん、ごにち; ご、ろくにち; なな、はちにち; きゅう、じゅうにち
        assert_eq!(reads_tagged("四、五日"), "joɴ, ɡoɲiʨi");
        assert_eq!(reads_tagged("4、5日"), "joɴ, ɡoɲiʨi");
        assert_eq!(reads_tagged("五、六日"), "ɡo, ɾokɯɲiʨi");
        assert_eq!(reads_tagged("5、6日"), "ɡo, ɾokɯɲiʨi");
        assert_eq!(reads_tagged("七、八日"), "nana, haʨiɲiʨi");
        assert_eq!(reads_tagged("7、8日"), "nana, haʨiɲiʨi");
        assert_eq!(reads_tagged("九、十日"), "kʲɨː, ʥɨːɲiʨi");
        assert_eq!(reads_tagged("9、10日"), "kʲɨː, ʥɨːɲiʨi");
        // ろく、しちにち and はち、くにち, as a day of the month says them
        assert_eq!(reads_tagged("六、七日"), "ɾokɯ, ɕiʨiɲiʨi");
        assert_eq!(reads_tagged("8、9日"), "haʨi, kɯɲiʨi");
        // 14 or 15 days, written short: じゅうよん、ごにち
        assert_eq!(reads_tagged("十四、五日"), "ʥɨː joɴ, ɡoɲiʨi");
        assert_eq!(reads_tagged("14、5日"), "ʥɨːjoɴ, ɡoɲiʨi");
        assert_eq!(reads_tagged("二十四、五日"), "ɲi ʥɨː joɴ, ɡoɲiʨi");
        // いち、ににん: no ひとり or ふたり in an approximate count
        assert_eq!(reads_tagged("1、2人"), "iʨi, ɲiɲiɴ");
        assert_eq!(reads_tagged("一、二人"), "iʨi, ɲiɲiɴ");
        assert_eq!(reads_tagged("1、2日"), "iʨi, ɲiɲiʨi");
        // The dictionary's word 二日 keeps its ふつか.
        assert_eq!(reads_tagged("一、二日"), "iʨi, ɸɯʦɨka");
        // A run of them: いち、に、さんにち
        assert_eq!(reads_tagged("1、2、3日"), "iʨi, ɲi, saɲɲiʨi");
        assert_eq!(reads_tagged("1,2,3日"), "iʨi, ɲi, saɲɲiʨi");
        // Past 99: digits take the short form (102 or 103), kanji with 百 do
        // not.
        assert_eq!(reads_tagged("102、3日"), "çakɯɲi, saɲɲiʨi");
        assert_eq!(reads_tagged("百二、三日"), "çakɯ ɲi, mʲiʔka");
        // よっか and よにん have no plainer form: さん、よっか.
        assert_eq!(reads_tagged("三、四日"), "saɴ, joʔka");
        assert_eq!(reads_tagged("3、4日"), "saɴ, joʔka");
        assert_eq!(reads_tagged("3、4人"), "saɴ, joɲiɴ");
        assert_eq!(reads_tagged("2、3人"), "ɲi, saɲɲiɴ");
        assert_eq!(reads_tagged("二、三人"), "ɲi, saɲɲiɴ");
        // Months and things keep their words: さん、しがつ; に、みっつ.
        assert_eq!(reads_tagged("3、4月"), "saɴ, ɕiɡaʦɨ");
        assert_eq!(reads_tagged("2、3つ"), "ɲi, mʲiʔʦɨ");
        // Two days of a month with no month named read as a count.
        assert_eq!(reads_tagged("5、6日の両日"), "ɡo, ɾokɯɲiʨi no ɾʲoːʥiʦɨ");
    }

    #[test]
    fn numbers_that_are_not_an_approximate_count_read_as_before() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        // Each as the reader read it before approximate counts.
        for (text, expected) in [
            // 〜 and から are said, so the count after them keeps its word.
            ("2〜3日で届きます", "ɲi kaɾa mʲiʔka de todokʲi masɨ"),
            ("3〜5日の連休", "saɴ kaɾa iʦɨka no ɾeŋkʲɨː"),
            ("1〜2人", "iʨi kaɾa ɸɯtaɾʲi"),
            ("2～3日", "ɲi kaɾa mʲiʔka"),
            ("2から3日", "ɲi kaɾa mʲiʔka"),
            ("1から2日", "iʨi kaɾa ɸɯʦɨka"),
            ("二〜三日", "ɲi — mʲiʔka"),
            ("二～三日", "ɲi ~ mʲiʔka"),
            ("一〜二人", "iʨi — ɸɯtaɾʲi"),
            ("三〜五日", "saɴ — iʦɨka"),
            ("二から三日", "ɲi kaɾa mʲiʔka"),
            ("一から二人で作った", "iʨi kaɾa ɸɯtaɾʲi de ʦɨkɯʔta"),
            ("ゼロから一人で始めた", "ʣeɾo kaɾa çitoɾʲi de haʥime ta"),
            ("一から十日かけて", "iʨi kaɾa toːka kake te"),
            ("五〜六人", "ɡo — ɾokɯɲiɴ"),
            ("5〜6人", "ɡo kaɾa ɾokɯɲiɴ"),
            // Numbers that do not follow on are two numbers.
            ("1日目は5、2日目は3", "iʨiɲiʨi me βa ɡo, ɸɯʦɨka me βa saɴ"),
            (
                "一日目は五、二日目は三",
                "iʨiɲiʨi me βa ɡo, ɸɯʦɨka me βa saɴ",
            ),
            ("全部で5、3日後に", "ʣembɯ de ɡo, mʲiʔka ɡo ɲi"),
            ("全部で五、三日後に", "ʣembɯ de ɡo, mʲiʔka ɡo ɲi"),
            ("10、20日に支給", "ʥɨː, haʦɨka ɲi ɕikʲɨː"),
            ("十、二十日に支給", "ʥɨː, haʦɨka ɲi ɕikʲɨː"),
            ("19、20日に開催", "ʥɨːkʲɨː, haʦɨka ɲi kaisai"),
            ("十九、二十日に開催", "ʥɨː kʲɨː, haʦɨka ɲi kaisai"),
            ("2、5日", "ɲi, iʦɨka"),
            ("二、五日", "ɲi, iʦɨka"),
            ("3、2日", "saɴ, ɸɯʦɨka"),
            ("三、二日", "saɴ, ɸɯʦɨka"),
            ("1、一人で", "iʨi, çitoɾʲi de"),
            ("一、一人で", "iʨi, çitoɾʲi de"),
            ("十、一日", "ʥɨː, iʨiɲiʨi"),
            ("2,5日", "ɲi, iʦɨka"),
            ("二，五日", "ɲi, ɡo ɲiʨi"),
            // The number before is 1,001 or 1,234 whole, not its last group.
            ("1,001、2日", "seɴiʨi, ɸɯʦɨka"),
            ("1,234、5日", "seɲɲiçakɯsaɲʥɨːjoɴ, iʦɨka"),
            ("12,345、6日", "iʨimaɲɲiseɴsambʲakɯjoɲʥɨːɡo, mɯika"),
            // Ordinals and "both" after the counter.
            ("第1、2日目", "dai iʨi, ɸɯʦɨka me"),
            ("第一、二日目", "dai iʨi, ɸɯʦɨka me"),
            ("2、3日目", "ɲi, mʲiʔka me"),
            ("初日は1、2日目は3", "ɕoɲiʨi βa iʨi, ɸɯʦɨka me βa saɴ"),
            ("1、2人目", "iʨi, ɸɯtaɾʲi me"),
            ("一、二人目", "iʨi, ɸɯtaɾʲi me"),
            ("1、2人とも", "iʨi, ɸɯtaɾʲi tomo"),
            ("一、二人とも", "iʨi, ɸɯtaɾʲi tomo"),
            // ・ can be a decimal point, and a space ends the count.
            ("3・5日", "saɴ iʦɨka"),
            ("2・3日", "ɲi mʲiʔka"),
            ("二・三日", "ɲi saɴ ɲiʨi"),
            ("二、 三日", "ɲi, mʲiʔka"),
        ] {
            assert_eq!(reads_with(tagger, text), expected, "{text}");
        }
    }

    #[test]
    fn days_of_a_month_named_before_them_stay_days() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        // ついたち から みっか
        assert_eq!(reads_tagged("1月1日〜3日"), "iʨiɡaʦɨ ʦɨitaʨi kaɾa mʲiʔka");
        assert_eq!(reads_tagged("4月1日〜3日"), "ɕiɡaʦɨ ʦɨitaʨi kaɾa mʲiʔka");
        assert_eq!(reads_tagged("一月一日〜三日"), "iʨiɡaʦɨ ʦɨitaʨi — mʲiʔka");
        assert_eq!(reads_tagged("1日から3日まで"), "iʨiɲiʨi kaɾa mʲiʔka made");
        assert_eq!(reads_tagged("2日、3日"), "ɸɯʦɨka, mʲiʔka");
        // A month before the first number makes them its days.
        assert_eq!(reads_tagged("4月1〜3日"), "ɕiɡaʦɨ iʨi kaɾa mʲiʔka");
        assert_eq!(reads_tagged("四月一〜三日"), "ɕiɡaʦɨ iʨi — mʲiʔka");
        assert_eq!(reads_tagged("4月2、3日"), "ɕiɡaʦɨ ɲi, mʲiʔka");
        assert_eq!(reads_tagged("5月3、4、5日"), "ɡoɡaʦɨ saɴ, joɴ, iʦɨka");
        assert_eq!(
            reads_tagged("8月1、2日は休業します"),
            "haʨiɡaʦɨ iʨi, ɸɯʦɨka βa kʲɨːɡʲoː ɕi masɨ"
        );
    }

    #[test]
    fn a_vowel_the_dictionary_splits_off_lengthens_the_word_before() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        assert_eq!(reads_tagged("行きましょう。"), "ikʲi maɕoː.");
        assert_eq!(reads_tagged("行こう"), "ikoː");
        assert_eq!(reads_tagged("雨でしょう"), "ame deɕoː");
        assert_eq!(reads_tagged("雨だろう"), "ame daɾoː");
        // う inside a word, after a space, or written 鵜 stays as written.
        assert_eq!(reads_tagged("思う"), "omoɯ");
        assert_eq!(reads_tagged("そこ う"), "soko ɯ");
        assert_eq!(reads_tagged("その鵜"), "sono ɯ");
        assert_eq!(reads_tagged("あ、い、う"), "a, i, ɯ");
        // So does one after a line break or a character that is not read.
        assert_eq!(reads_tagged("行こ\nう"), "iko ɯ");
        assert_eq!(tagged_with(tagger, "行こ\n龘う"), ("iko ɯ".into(), 1));
        assert_eq!(tagged_with(tagger, "行こ龘う"), ("iko ɯ".into(), 1));
        assert_eq!(tagged_with(tagger, "そ〇う"), ("so ɯ".into(), 1));
        assert_eq!(tagged_with(tagger, "も\n〇う"), ("mo ɯ".into(), 1));
    }

    #[test]
    fn kanji_katakana_and_english_together() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        assert_eq!(
            reads_tagged("私はiPhoneでコーヒーを買いました。"),
            "βataɕi βa <iPhone> de koːçiː o kai maɕi ta."
        );
        assert_eq!(
            reads_tagged("昨日、新しいMacBookを買って、Pythonのコードを書きました。"),
            "kʲinoː, ataɾaɕii <MacBook> o kaʔte, <Python> no koːdo o kakʲi maɕi ta."
        );
        assert_eq!(
            reads_tagged("日本語 and emoji pass through."),
            "ɲihoŋɡo <and emoji pass through.>"
        );
        assert_eq!(
            reads_tagged("「こんにちは」と言った。"),
            "“koɲɲiʨiβa” to iʔta."
        );
    }

    #[test]
    fn a_word_the_dictionary_cannot_read_is_skipped_and_counted() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        assert_eq!(tagged_with(tagger, "龘"), (String::new(), 1));
        assert_eq!(
            tagged_with(tagger, "龘は字です。"),
            ("βa ʥi desɨ.".into(), 1)
        );
    }

    #[test]
    fn sound_marks_and_half_width_kana_join_up() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        assert_eq!(reads_tagged("か\u{3099}"), "ɡa");
        assert_eq!(reads_tagged("ｶﾞｷﾞ"), "ɡa ɡʲi");
        assert_eq!(reads_tagged("ﾃｽﾄ"), reads_tagged("テスト"));
        assert_eq!(reads_tagged("㍿"), reads_tagged("株式会社"));
    }

    #[test]
    fn line_breaks_and_tabs_keep_the_words_in_place() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let reads_tagged = |text: &str| reads_with(tagger, text);
        assert_eq!(
            reads_tagged("東京に\n行きます。\t3人"),
            reads_tagged("東京に 行きます。 3人")
        );
    }

    #[test]
    fn every_symbol_of_kanji_prose_is_one_kokoro_has() {
        // Every kana a pronunciation can hold.
        let katakana: String = ('\u{30a1}'..='\u{30fc}').collect();
        let (kana, _) = phonemes(&katakana, None, |_| Ok(String::new())).unwrap();
        let outside: Vec<char> = kana.chars().filter(|&c| !in_vocab(c)).collect();
        assert!(outside.is_empty(), "{outside:?}");
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let (prose, skipped) = tagged_with(tagger, KANJI_PROSE);
        assert_eq!(skipped, 0);
        let outside: Vec<char> = prose.chars().filter(|&c| !in_vocab(c)).collect();
        assert!(outside.is_empty(), "{outside:?}");
    }

    /// The word offsets `tag` keeps depend on the dictionary seeing one
    /// character for each of the text's, sound marks aside.
    #[test]
    fn the_dictionary_sees_one_character_for_each_but_sound_marks() {
        let Some(tagger) = tagger_for_test() else {
            return;
        };
        let pool: Vec<char> = "あかがはへをんっゃーゝゞうらこともアカガハヲンッャヴヵヶ東京行日本人年分月中組目第後初一二三四五九十百毎龘々〆〇0123456789,.、。！？「」（）・〜～，ー—%$¥\\-~`\"' 　abcＡｱｶﾞﾟ\u{3099}\u{309a}\u{309b}\u{309c}🎉\u{200d}\u{301}é\u{200b}\u{feff}"
            .chars()
            .collect();
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        for _ in 0..3000 {
            let len = {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                (seed % 20) as usize
            };
            let text: String = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    pool[(seed as usize) % pool.len()]
                })
                .collect();
            let nodes = tagger.text_to_njd(&text).unwrap().nodes;
            let seen: usize = nodes.iter().map(|n| n.get_string().chars().count()).sum();
            let expected = text.chars().filter(|&c| !is_sound_mark(c)).count();
            assert_eq!(seen, expected, "{text:?}");
            phonemes(&text, Some(tagger), |_| Ok(String::new())).unwrap();
        }
    }
}
