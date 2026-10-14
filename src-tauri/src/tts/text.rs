//! Text preparation for speech: markdown/URL cleanup and sentence chunking.

/// Fragments shorter than this are merged with a neighbour so the voice keeps
/// natural prosody across one-word sentences and clause cuts.
const MIN_SENTENCE_CHARS: usize = 20;
/// Longer sentences are split so the first audio chunk arrives quickly.
const MAX_SENTENCE_CHARS: usize = 300;

const TERMINATORS: &[char] = &['.', '!', '?', '…'];
const CLOSERS: &[char] = &['"', '\'', '”', '’', ')', ']', '»'];
const CLAUSE_BREAKS: &[char] = &[',', ';', ':'];
const URL_PREFIXES: &[&str] = &["https://", "http://", "www."];
/// Words whose final "." does not end a sentence; compared case-insensitively.
const ABBREVIATIONS: &[&str] = &[
    "dr.", "mr.", "mrs.", "ms.", "st.", "vs.", "e.g.", "i.e.", "etc.", "u.s.", "u.k.", "a.m.",
    "p.m.", "jan.", "feb.", "mar.", "apr.", "jun.", "jul.", "aug.", "sep.", "sept.", "oct.",
    "nov.", "dec.",
];

/// Turns selected text into something worth reading aloud: URLs become "link",
/// markdown syntax is dropped and whitespace collapses within each line. Line
/// breaks survive for `split_sentences`; text with nothing to pronounce becomes "".
pub fn normalize(input: &str) -> String {
    let lines: Vec<&str> = input.lines().map(strip_line_markers).collect();
    let text = strip_inline_markers(&replace_urls(&strip_links(&lines.join("\n"))));
    if !text.chars().any(char::is_alphanumeric) {
        return String::new();
    }
    text.lines()
        .map(collapse_whitespace)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Drops blockquote, heading and bullet markers at the start of a line.
fn strip_line_markers(line: &str) -> &str {
    let mut line = line.trim_start();
    while let Some(rest) = line.strip_prefix('>') {
        line = rest.trim_start();
    }
    let unhashed = line.trim_start_matches('#');
    if unhashed.len() < line.len() && (unhashed.is_empty() || unhashed.starts_with(' ')) {
        line = unhashed.trim_start();
    }
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            return rest.trim_start();
        }
    }
    line
}

/// `[text](url)` and `![alt](url)` keep only their text. Both must start a
/// word, so code such as `handlers[0](event)` is left alone.
fn strip_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut search = 0;
    while let Some(found) = text[search..].find('[') {
        let open = search + found;
        search = open + 1;
        if continues_word(&text[..open]) {
            continue;
        }
        let Some((label, len)) = parse_link(&text[open + 1..]) else {
            continue;
        };
        let before = &text[copied..open];
        // After a word, `!` is punctuation ("Great![1](url)"), not an image marker.
        let image = before.ends_with('!') && !continues_word(&text[..open - 1]);
        out.push_str(if image {
            &before[..before.len() - 1]
        } else {
            before
        });
        out.push_str(label);
        copied = open + 1 + len;
        search = copied;
    }
    out.push_str(&text[copied..]);
    out
}

fn continues_word(before: &str) -> bool {
    before
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | ']' | ')'))
}

/// Parses the `label](destination)` after a `[` into the label and the length
/// consumed. The label stops at the first bracket or line break and the
/// destination at whitespace, so a stray bracket never scans far.
fn parse_link(text: &str) -> Option<(&str, usize)> {
    let label_end = text.find(['[', ']', '\n'])?;
    let destination = text[label_end..].strip_prefix("](")?;
    let destination_end = destination.find(|c: char| c == ')' || c.is_whitespace())?;
    destination[destination_end..]
        .starts_with(')')
        .then_some((&text[..label_end], label_end + 2 + destination_end + 1))
}

fn replace_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut prev: Option<char> = None;
    while let Some(c) = rest.chars().next() {
        // The first-letter check keeps long inputs fast; every prefix starts with h or w.
        let is_url = matches!(c, 'h' | 'H' | 'w' | 'W')
            && !prev.is_some_and(char::is_alphanumeric)
            && URL_PREFIXES.iter().any(|prefix| {
                rest.get(..prefix.len())
                    .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
            });
        if !is_url {
            out.push(c);
            prev = Some(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let end = rest
            .find(|ch: char| ch.is_whitespace() || "<>\"()[]".contains(ch))
            .unwrap_or(rest.len());
        // Sentence punctuation after a URL belongs to the sentence.
        let url = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', '\'']);
        out.push_str("link");
        prev = Some('k');
        rest = &rest[url.len()..];
    }
    out
}

/// Drops emphasis, code and table symbols; `_` survives inside words like `snake_case`.
fn strip_inline_markers(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '*' | '`' | '~' => {}
            '|' => out.push(' '),
            '_' => {
                let before = i > 0 && chars[i - 1].is_alphanumeric();
                let after = chars.get(i + 1).is_some_and(|n| n.is_alphanumeric());
                if before && after {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// A first sentence longer than this many words is cut at a clause break, so
/// its audio is ready sooner.
const FIRST_CHUNK_WORDS: usize = 8;

/// Cuts the first sentence at its first clause break (`,;:` or a dash, but
/// not one inside a range such as "1914–1918") when it is longer than
/// `FIRST_CHUNK_WORDS`. Both parts keep at least `MIN_SENTENCE_CHARS`; without
/// such a break it stays whole.
pub fn split_first_clause(mut sentences: Vec<String>) -> Vec<String> {
    let Some(first) = sentences.first() else {
        return sentences;
    };
    if first.split_whitespace().count() <= FIRST_CHUNK_WORDS {
        return sentences;
    }
    if let Some((head, tail)) = first_clause_break(first) {
        let parts = [head.to_string(), tail.to_string()];
        sentences.splice(0..1, parts);
    }
    sentences
}

fn first_clause_break(sentence: &str) -> Option<(&str, &str)> {
    let is_short = |s: &str| s.chars().count() < MIN_SENTENCE_CHARS;
    for (i, c) in sentence.char_indices() {
        let end = i + c.len_utf8();
        let after = &sentence[end..];
        let is_break = match c {
            // Not inside "1,500" or "10:30".
            ',' | ';' | ':' => after.starts_with(char::is_whitespace),
            '—' | '–' => !joins_numbers(&sentence[..i], after),
            // A spaced hyphen is a dash; "long-delayed" is not.
            '-' => {
                sentence[..i].ends_with(' ')
                    && after.starts_with(' ')
                    && !joins_numbers(&sentence[..i], after)
            }
            _ => false,
        };
        if !is_break {
            continue;
        }
        let (head, tail) = (sentence[..end].trim_end(), after.trim_start());
        if is_short(head) {
            continue;
        }
        // Later breaks only leave a shorter tail.
        return (!is_short(tail)).then_some((head, tail));
    }
    None
}

/// A dash with a number on each side, as in "1914–1918", "9:00 – 17:00" or
/// "£5-£10", is a range.
fn joins_numbers(before: &str, after: &str) -> bool {
    let has_digit =
        |word: Option<&str>| word.is_some_and(|w| w.contains(|c: char| c.is_ascii_digit()));
    has_digit(before.split_whitespace().next_back()) && has_digit(after.split_whitespace().next())
}

/// Splits normalized text into chunks to synthesise one request at a time.
/// Every line break ends a sentence, so headings and bullets need no punctuation.
pub fn split_sentences(text: &str) -> Vec<String> {
    let sentences = text
        .lines()
        .flat_map(raw_sentences)
        .map(collapse_whitespace);
    merge_short(sentences)
        .iter()
        .flat_map(|sentence| merge_short(split_long(sentence)))
        .collect()
}

/// Joins a fragment onto the previous one when either is short.
fn merge_short(fragments: impl IntoIterator<Item = String>) -> Vec<String> {
    let is_short = |s: &str| s.chars().count() < MIN_SENTENCE_CHARS;
    let mut merged: Vec<String> = Vec::new();
    for fragment in fragments {
        match merged.last_mut() {
            Some(last) if is_short(&fragment) || is_short(last) => {
                last.push(' ');
                last.push_str(&fragment);
            }
            _ => merged.push(fragment),
        }
    }
    merged
}

/// Splits after terminal punctuation (plus closing quotes/brackets) that is
/// followed by whitespace, so "3.14" stays intact, unless it ends an abbreviation.
fn raw_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if !TERMINATORS.contains(&c) {
            continue;
        }
        let mut end = i + c.len_utf8();
        while let Some(&(j, next)) = chars.peek() {
            if !TERMINATORS.contains(&next) && !CLOSERS.contains(&next) {
                break;
            }
            end = j + next.len_utf8();
            chars.next();
        }
        let at_break = chars.peek().is_none_or(|&(_, next)| next.is_whitespace());
        if at_break && !ends_with_abbreviation(&text[start..end]) {
            sentences.push(text[start..end].trim());
            start = end;
        }
    }
    sentences.push(text[start..].trim());
    sentences.retain(|s| !s.is_empty());
    sentences
}

fn ends_with_abbreviation(text: &str) -> bool {
    let word = text
        .rsplit(char::is_whitespace)
        .next()
        .unwrap_or_default()
        .trim_start_matches(|c: char| !c.is_alphanumeric());
    ABBREVIATIONS
        .iter()
        .any(|abbreviation| word.eq_ignore_ascii_case(abbreviation))
}

/// Cuts at the last clause break (`,;:` then whitespace, so "$1,500" and "10:30"
/// survive) before the limit, else the last space, else mid-word; always on a
/// char boundary.
fn split_long(sentence: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = sentence.trim();
    while let Some((limit, _)) = rest.char_indices().nth(MAX_SENTENCE_CHARS) {
        let head = &rest[..limit];
        let clause_break = head
            .char_indices()
            .rev()
            .find(|&(i, c)| {
                CLAUSE_BREAKS.contains(&c) && rest[i + 1..].starts_with(char::is_whitespace)
            })
            .map(|(i, _)| i + 1);
        let cut = clause_break
            .or_else(|| head.rfind(char::is_whitespace))
            .filter(|&i| i > 0)
            .unwrap_or(limit);
        let (piece, tail) = rest.split_at(cut);
        pieces.push(piece.trim_end().to_string());
        rest = tail.trim_start();
    }
    if !rest.is_empty() {
        pieces.push(rest.to_string());
    }
    pieces
}

/// Rewrites the numbers the local voice's phonemiser misreads as words:
/// currency, decimals, versions, years and decades, ordinals, times and
/// percentages. Plain integers, which it reads well, are left alone.
pub fn expand_numbers(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut rest = text;
    let mut prev: Option<char> = None;
    while let Some(c) = rest.chars().next() {
        if let Some(number) = may_start_number(prev, rest)
            .then(|| Number::parse(rest))
            .flatten()
        {
            let (span, tail) = rest.split_at(number.len);
            rest = tail;
            prev = span.chars().next_back();
            let Some(spoken) = number.spoken() else {
                out.push_str(span);
                continue;
            };
            if number.is_short_decade() {
                // "'90s": the apostrophe stands for the dropped century.
                out.truncate(out.trim_end_matches(['\'', '’']).len());
            }
            out.push_str(&spoken);
            // "1990-2000", "9:00-17:00": a dash between two of a kind reads "to".
            if let Some(after_dash) = rest.strip_prefix(['-', '–']) {
                let same_kind = may_start_number(Some('-'), after_dash)
                    .then(|| Number::parse(after_dash))
                    .flatten()
                    .is_some_and(|next| {
                        next.kind() == number.kind()
                            && next.kind() != Kind::Other
                            && next.spoken().is_some()
                    });
                if same_kind {
                    out.push_str(" to ");
                    rest = after_dash;
                    prev = Some(' ');
                }
            }
            continue;
        }
        out.push(c);
        prev = Some(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Whether a number can start at `rest`: a digit, or a sign, currency symbol
/// or `v` right before one, not inside a word.
fn may_start_number(prev: Option<char>, rest: &str) -> bool {
    if prev.is_some_and(|p| p.is_alphanumeric() || p == '_') {
        return false;
    }
    let mut chars = rest.chars();
    let (Some(first), second) = (chars.next(), chars.next()) else {
        return false;
    };
    let digit_next = second.is_some_and(|c| c.is_ascii_digit());
    match first {
        c if c.is_ascii_digit() => true,
        '£' | '$' | '€' | 'v' | 'V' => digit_next,
        // A sign only where a word could start: "pages 5-10" has no minus.
        '-' => digit_next && prev.is_none_or(|p| p.is_whitespace() || "([{\"'“‘".contains(p)),
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Currency {
    Pound,
    Dollar,
    Euro,
}

impl Currency {
    fn at(text: &str) -> Option<(Self, usize)> {
        let symbol = text.chars().next()?;
        let currency = match symbol {
            '£' => Currency::Pound,
            '$' => Currency::Dollar,
            '€' => Currency::Euro,
            _ => return None,
        };
        Some((currency, symbol.len_utf8()))
    }

    /// Singular and plural names of the unit and of its hundredth.
    fn names(self) -> [&'static str; 4] {
        match self {
            Currency::Pound => ["pound", "pounds", "penny", "pence"],
            Currency::Dollar => ["dollar", "dollars", "cent", "cents"],
            Currency::Euro => ["euro", "euros", "cent", "cents"],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Suffix {
    None,
    Percent,
    Ordinal,
    /// "1990s", "90's".
    Plural,
    /// `keeps_stop`: a dotted "p.m." whose full stop also ended the sentence.
    Meridiem {
        pm: bool,
        keeps_stop: bool,
    },
    /// Letters that make it a code or unit, such as "2B" or "5km".
    Glued,
}

/// What a number is, for reading "A-B" as a range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Money,
    Clock,
    Percent,
    Year,
    Decade,
    Decimal,
    Other,
}

const SCALES: &[&str] = &["thousand", "million", "billion", "trillion"];

/// A number as written, before deciding how to say it.
#[derive(Debug)]
struct Number<'a> {
    negative: bool,
    /// Written "v1.2.3".
    version: bool,
    currency: Option<Currency>,
    /// Digits, possibly grouped with commas.
    whole: &'a str,
    /// Digit runs after each full stop: one for a decimal, more for a version.
    parts: Vec<&'a str>,
    minutes: Option<&'a str>,
    /// "million" in "$3 million".
    scale: Option<&'a str>,
    suffix: Suffix,
    /// Bytes of the text it spans.
    len: usize,
}

impl<'a> Number<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        let mut i = 0;
        let negative = text.starts_with('-');
        i += usize::from(negative);
        let version = !negative && text[i..].starts_with(['v', 'V']);
        i += usize::from(version);
        let currency = if negative || version {
            None
        } else {
            Currency::at(&text[i..])
        };
        i += currency.map_or(0, |(_, len)| len);

        let whole_start = i;
        i += digits(&text[i..]);
        if i == whole_start {
            return None;
        }
        if i - whole_start <= 3 {
            while text[i..].starts_with(',') && digits(&text[i + 1..]) == 3 {
                i += 4;
            }
        }
        let whole = &text[whole_start..i];

        let mut parts = Vec::new();
        while let Some(after) = text[i..].strip_prefix('.') {
            let len = digits(after);
            if len == 0 {
                break;
            }
            parts.push(&after[..len]);
            i += 1 + len;
        }

        // Only "H:MM" is a time; "3:1" and "10:30:15" stay as written.
        let clock_start = i;
        let mut groups = 0;
        while let Some(after) = text[i..].strip_prefix(':') {
            let len = digits(after);
            if len == 0 {
                break;
            }
            groups += 1;
            i += 1 + len;
        }
        let clock = &text[clock_start..i];
        let is_time = groups == 1
            && clock.len() == 3
            && whole.len() <= 2
            && parts.is_empty()
            && currency.is_none()
            && !negative
            && !version;
        let minutes = is_time.then(|| &clock[1..]);

        let (mut suffix, len) = if groups > 0 && !is_time {
            (Suffix::Glued, 0)
        } else {
            read_suffix(&text[i..])
        };
        i += len;

        let mut scale = None;
        if currency.is_some() && suffix == Suffix::None {
            if let Some((word, len)) = scale_word(&text[i..]) {
                scale = Some(word);
                i += len;
            }
        }

        let glued = text[i..].chars().next().is_some_and(char::is_alphanumeric);
        if glued {
            suffix = Suffix::Glued;
            i += text[i..]
                .find(|c: char| !c.is_alphanumeric())
                .unwrap_or(text.len() - i);
        }

        Some(Self {
            negative,
            version,
            currency: currency.map(|(currency, _)| currency),
            whole,
            parts,
            minutes,
            scale,
            suffix,
            len: i,
        })
    }

    fn kind(&self) -> Kind {
        if self.currency.is_some() {
            return Kind::Money;
        }
        match self.suffix {
            _ if self.minutes.is_some() => Kind::Clock,
            Suffix::Meridiem { .. } => Kind::Clock,
            Suffix::Percent => Kind::Percent,
            Suffix::Plural => Kind::Decade,
            Suffix::None if self.parts.len() == 1 => Kind::Decimal,
            Suffix::None if self.parts.is_empty() && self.whole.len() == 4 => Kind::Year,
            _ => Kind::Other,
        }
    }

    /// "'90s" as opposed to "1990s".
    fn is_short_decade(&self) -> bool {
        self.suffix == Suffix::Plural && self.whole.len() == 2
    }

    /// The words to say, or `None` to leave it as written.
    fn spoken(&self) -> Option<String> {
        if self.suffix == Suffix::Glued {
            return None;
        }
        if let Some(currency) = self.currency {
            return self.money(currency);
        }
        let meridiem = match self.suffix {
            Suffix::Meridiem { pm, keeps_stop } => Some((if pm { "PM" } else { "AM" }, keeps_stop)),
            _ => None,
        };
        if let Some(minutes) = self.minutes {
            if !matches!(self.suffix, Suffix::None | Suffix::Meridiem { .. }) {
                return None;
            }
            return clock(int_value(self.whole)?, minutes.parse().ok()?, meridiem);
        }

        let integer = !self.negative && !self.version && self.parts.is_empty();
        let sign = if self.negative { "minus " } else { "" };
        match self.suffix {
            Suffix::Percent if !self.version => Some(format!("{sign}{} percent", self.amount()?)),
            Suffix::Ordinal if integer => ordinal(int_value(self.whole)?),
            Suffix::Plural if integer => self.decade(),
            Suffix::Meridiem { .. } if integer => clock(int_value(self.whole)?, 0, meridiem),
            Suffix::None => match self.parts.len() {
                // The phonemiser drops the sign but reads the digits well.
                0 if self.negative => Some(format!("minus {}", self.whole)),
                0 if integer && self.whole.len() == 4 && !self.whole.starts_with('0') => {
                    year(int_value(self.whole)?)
                }
                0 => None,
                1 if self.version => Some(format!("version {}", self.amount()?)),
                1 => Some(format!("{sign}{}", self.amount()?)),
                _ if !self.negative && !self.whole.contains(',') => {
                    let prefix = if self.version { "version " } else { "" };
                    Some(format!("{prefix}{}", self.version_words()?))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// An integer or decimal amount: "twelve", "three point two".
    fn amount(&self) -> Option<String> {
        let whole = cardinal(int_value(self.whole)?)?;
        match self.parts.as_slice() {
            [] => Some(whole),
            [fraction] => Some(format!("{whole} point {}", digit_words(fraction))),
            _ => None,
        }
    }

    fn money(&self, currency: Currency) -> Option<String> {
        if self.suffix != Suffix::None || self.parts.len() > 1 {
            return None;
        }
        let [one, many, one_minor, many_minor] = currency.names();
        if let Some(scale) = self.scale {
            return Some(format!(
                "{} {} {many}",
                self.amount()?,
                scale.to_ascii_lowercase()
            ));
        }
        let named = |n: u64, one: &str, many: &str| {
            Some(format!(
                "{} {}",
                cardinal(n)?,
                if n == 1 { one } else { many }
            ))
        };
        let major = int_value(self.whole)?;
        match self.parts.as_slice() {
            [] => named(major, one, many),
            [minor] if minor.len() == 2 => match (major, minor.parse::<u64>().ok()?) {
                (_, 0) => named(major, one, many),
                (0, minor) => named(minor, one_minor, many_minor),
                (_, minor) => Some(format!(
                    "{} and {}",
                    named(major, one, many)?,
                    named(minor, one_minor, many_minor)?
                )),
            },
            _ => Some(format!("{} {many}", self.amount()?)),
        }
    }

    fn decade(&self) -> Option<String> {
        let value = int_value(self.whole)?;
        match self.whole.len() {
            4 if !self.whole.starts_with('0') => Some(plural(&year(value)?)),
            2 if self.whole.ends_with('0') && !self.whole.starts_with('0') => {
                Some(plural(&below_hundred(value)))
            }
            _ => None,
        }
    }

    /// "two point zero point one": each part as a number, but "05" as digits.
    fn version_words(&self) -> Option<String> {
        let words: Option<Vec<String>> = std::iter::once(self.whole)
            .chain(self.parts.iter().copied())
            .map(|part| {
                if part.len() > 1 && part.starts_with('0') {
                    Some(digit_words(part))
                } else {
                    cardinal(part.parse().ok()?)
                }
            })
            .collect();
        Some(words?.join(" point "))
    }
}

fn digits(text: &str) -> usize {
    text.bytes().take_while(u8::is_ascii_digit).count()
}

fn int_value(digits: &str) -> Option<u64> {
    digits.replace(',', "").parse().ok()
}

/// "%", "st", "s", "'s", "pm", "a.m." or " pm" right after the digits, with
/// the bytes it spans.
fn read_suffix(rest: &str) -> (Suffix, usize) {
    if rest.starts_with('%') {
        return (Suffix::Percent, 1);
    }
    for apostrophe_s in ["'s", "’s"] {
        if rest.starts_with(apostrophe_s) {
            return (Suffix::Plural, apostrophe_s.len());
        }
    }
    if let Some((pm, len, keeps_stop)) = meridiem_at(rest) {
        return (Suffix::Meridiem { pm, keeps_stop }, len);
    }
    if let Some((pm, len, keeps_stop)) = rest.strip_prefix(' ').and_then(meridiem_at) {
        return (Suffix::Meridiem { pm, keeps_stop }, 1 + len);
    }
    let letters = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
    match rest[..letters].to_ascii_lowercase().as_str() {
        "st" | "nd" | "rd" | "th" => (Suffix::Ordinal, letters),
        "s" => (Suffix::Plural, letters),
        _ => (Suffix::None, 0),
    }
}

/// "am", "pm", "a.m." or "p.m." as a whole word: whether it is pm, its
/// length, and whether its final full stop also ends the sentence.
fn meridiem_at(text: &str) -> Option<(bool, usize, bool)> {
    let lower = text.get(..4).map(str::to_ascii_lowercase);
    let (pm, len, dotted) = match lower.as_deref() {
        Some("a.m.") => (false, 4, true),
        Some("p.m.") => (true, 4, true),
        _ => match text.get(..2).map(str::to_ascii_lowercase).as_deref() {
            Some("am") => (false, 2, false),
            Some("pm") => (true, 2, false),
            _ => return None,
        },
    };
    let after = &text[len..];
    if after.starts_with(char::is_alphanumeric) {
        return None;
    }
    let ends_sentence = after.is_empty()
        || after.trim_start().starts_with(char::is_uppercase)
            && after.starts_with(char::is_whitespace);
    Some((pm, len, dotted && ends_sentence))
}

/// " million" and the like after an amount of money.
fn scale_word(rest: &str) -> Option<(&str, usize)> {
    let word_start = rest.strip_prefix(' ')?;
    let len = word_start
        .bytes()
        .take_while(u8::is_ascii_alphabetic)
        .count();
    let word = &word_start[..len];
    SCALES
        .iter()
        .any(|scale| word.eq_ignore_ascii_case(scale))
        .then_some((word, 1 + len))
}

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

fn below_hundred(n: u64) -> String {
    let n = n as usize;
    match (n, n % 10) {
        (0..=19, _) => ONES[n].to_string(),
        (_, 0) => TENS[n / 10].to_string(),
        (_, units) => format!("{}-{}", TENS[n / 10], ONES[units]),
    }
}

fn below_thousand(n: u64) -> String {
    match (n / 100, n % 100) {
        (0, rest) => below_hundred(rest),
        (hundreds, 0) => format!("{} hundred", ONES[hundreds as usize]),
        (hundreds, rest) => format!(
            "{} hundred and {}",
            ONES[hundreds as usize],
            below_hundred(rest)
        ),
    }
}

/// British style, as the phonemiser reads plain integers: "one hundred and
/// five". `None` from a trillion up.
fn cardinal(n: u64) -> Option<String> {
    if n == 0 {
        return Some(ONES[0].to_string());
    }
    if n >= 1_000_000_000_000 {
        return None;
    }
    let mut words = Vec::new();
    for (scale, name) in [
        (1_000_000_000, "billion"),
        (1_000_000, "million"),
        (1_000, "thousand"),
    ] {
        let group = n / scale % 1000;
        if group > 0 {
            words.push(format!("{} {name}", below_thousand(group)));
        }
    }
    match n % 1000 {
        0 => {}
        units if units < 100 && !words.is_empty() => {
            words.push(format!("and {}", below_hundred(units)));
        }
        units => words.push(below_thousand(units)),
    }
    Some(words.join(" "))
}

fn ordinal(n: u64) -> Option<String> {
    let words = cardinal(n)?;
    let (head, last) = words.split_at(words.rfind([' ', '-']).map_or(0, |i| i + 1));
    let last = match last {
        "one" => "first".to_string(),
        "two" => "second".to_string(),
        "three" => "third".to_string(),
        "five" => "fifth".to_string(),
        "eight" => "eighth".to_string(),
        "nine" => "ninth".to_string(),
        "twelve" => "twelfth".to_string(),
        word => match word.strip_suffix('y') {
            Some(stem) => format!("{stem}ieth"),
            None => format!("{word}th"),
        },
    };
    Some(format!("{head}{last}"))
}

/// A four-digit number read as a year, as misaki reads them for Kokoro:
/// "nineteen ninety", "nineteen oh five", "two thousand and five".
fn year(n: u64) -> Option<String> {
    let (century, rest) = (n / 100, n % 100);
    if !(10..=99).contains(&century) {
        return None;
    }
    if century % 10 == 0 && rest < 10 {
        return cardinal(n);
    }
    let rest = match rest {
        0 => "hundred".to_string(),
        1..=9 => format!("oh {}", ONES[rest as usize]),
        _ => below_hundred(rest),
    };
    Some(format!("{} {rest}", below_hundred(century)))
}

fn plural(words: &str) -> String {
    match words.strip_suffix('y') {
        Some(stem) => format!("{stem}ies"),
        None if words.ends_with(['s', 'x']) => format!("{words}es"),
        None => format!("{words}s"),
    }
}

fn digit_words(digits: &str) -> String {
    digits
        .bytes()
        .map(|digit| ONES[usize::from(digit - b'0')])
        .collect::<Vec<_>>()
        .join(" ")
}

/// "ten thirty", "nine o'clock", "fourteen oh five", "three PM". `None` for
/// an hour or minute out of range.
fn clock(hour: u64, minutes: u64, meridiem: Option<(&str, bool)>) -> Option<String> {
    let hour_ok = match meridiem {
        Some(_) => (1..=12).contains(&hour),
        None => hour <= 23,
    };
    if !hour_ok || minutes > 59 {
        return None;
    }
    let h = below_hundred(hour);
    let time = match (minutes, meridiem) {
        (0, Some(_)) => h,
        (0, None) if (1..=12).contains(&hour) => format!("{h} o'clock"),
        (0, None) => format!("{h} hundred"),
        (1..=9, _) => format!("{h} oh {}", ONES[minutes as usize]),
        _ => format!("{h} {}", below_hundred(minutes)),
    };
    Some(match meridiem {
        Some((marker, true)) => format!("{time} {marker}."),
        Some((marker, false)) => format!("{time} {marker}"),
        None => time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn normalize_replaces_urls_with_link() {
        assert_eq!(
            normalize("See https://example.com/a?b=1 and www.rust-lang.org."),
            "See link and link."
        );
        assert_eq!(normalize("(http://x.io/path)"), "(link)");
        assert_eq!(normalize("HTTPS://EXAMPLE.COM"), "link");
    }

    #[test]
    fn normalize_leaves_url_lookalikes_inside_words() {
        assert_eq!(normalize("awww.cute"), "awww.cute");
    }

    #[test]
    fn normalize_keeps_link_text_only() {
        assert_eq!(
            normalize("Read [the docs](https://docs.rs) and ![a logo](logo.png)."),
            "Read the docs and a logo."
        );
        assert_eq!(normalize("array[0] stays"), "array[0] stays");
        assert_eq!(normalize("a [b. Then [c](d)"), "a [b. Then c");
    }

    #[test]
    fn normalize_leaves_code_that_looks_like_links() {
        assert_eq!(normalize("handlers[0](event)"), "handlers[0](event)");
        assert_eq!(normalize("grid[0][1](x)"), "grid[0][1](x)");
        assert_eq!(
            normalize("[not a link](has space)"),
            "[not a link](has space)"
        );
        assert_eq!(normalize("[label]\n(next line)"), "[label]\n(next line)");
    }

    #[test]
    fn normalize_keeps_punctuation_before_a_link() {
        assert_eq!(normalize("Great![1](https://x.io/ref)"), "Great!1");
    }

    #[test]
    fn normalize_handles_large_json_quickly_without_losing_text() {
        let blob = format!(
            "{}{}",
            r#"{"ids": [1, 2, 3], "tags": ["a", "b"]}, "#.repeat(1_000),
            r#"{"call": "handlers[0](event)", "see": "[docs](https://docs.rs)"}"#
        );
        let started = Instant::now();
        let normalized = normalize(&blob);
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "took {:?}",
            started.elapsed()
        );
        assert_eq!(normalized, blob.replace("[docs](https://docs.rs)", "docs"));
    }

    #[test]
    fn normalize_drops_emphasis_and_code() {
        assert_eq!(
            normalize("**Bold**, *italic*, __strong__, _em_ and `code`."),
            "Bold, italic, strong, em and code."
        );
        assert_eq!(normalize("~~gone~~ text"), "gone text");
    }

    #[test]
    fn normalize_keeps_underscores_inside_words() {
        assert_eq!(normalize("call my_function now"), "call my_function now");
    }

    #[test]
    fn normalize_drops_line_markers() {
        assert_eq!(normalize("# Title\n## Sub heading"), "Title\nSub heading");
        assert_eq!(normalize("> quoted\n> > nested"), "quoted\nnested");
        assert_eq!(normalize("- one\n* two\n+ three"), "one\ntwo\nthree");
        assert_eq!(normalize("Issue #42 - fixed"), "Issue #42 - fixed");
    }

    #[test]
    fn normalize_turns_table_pipes_into_spaces() {
        assert_eq!(normalize("| a | b |\nx|y"), "a b\nx y");
    }

    #[test]
    fn normalize_collapses_whitespace_within_lines() {
        assert_eq!(
            normalize("  It's   fine,\n\n\tisn't it?  \r\n"),
            "It's fine,\nisn't it?"
        );
    }

    #[test]
    fn normalize_keeps_numbers_and_times() {
        assert_eq!(
            normalize("It costs $1,500. At 10:30, we leave."),
            "It costs $1,500. At 10:30, we leave."
        );
    }

    #[test]
    fn normalize_empty_and_whitespace_only() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize(" \n\t \r\n"), "");
        assert_eq!(normalize("** __ ``"), "");
    }

    #[test]
    fn normalize_without_anything_pronounceable_is_empty() {
        assert_eq!(normalize("..."), "");
        assert_eq!(normalize("—"), "");
        assert_eq!(normalize("🎉🎉🎉"), "");
        assert_eq!(normalize("… — !?\n- 👍"), "");
        assert_eq!(normalize("👍 ok"), "👍 ok");
    }

    #[test]
    fn normalize_combined_markdown_sample() {
        let input = "## Release notes\n\n\
                     > **Note:** see [the guide](https://example.com/guide).\n\n\
                     - Faster `startup`\n\
                     - Fixed _crash_ on https://bugs.example.com/1";
        assert_eq!(
            normalize(input),
            "Release notes\nNote: see the guide.\nFaster startup\nFixed crash on link"
        );
    }

    #[test]
    fn split_on_terminal_punctuation() {
        assert_eq!(
            split_sentences(
                "This is the first sentence. Is this the second one? Yes, this is the third!"
            ),
            vec![
                "This is the first sentence.",
                "Is this the second one?",
                "Yes, this is the third!",
            ]
        );
    }

    #[test]
    fn split_on_line_breaks_first() {
        assert_eq!(
            split_sentences(
                "Installing the application\nDownload the latest build\nOpen it twice to confirm"
            ),
            vec![
                "Installing the application",
                "Download the latest build",
                "Open it twice to confirm",
            ]
        );
        assert_eq!(
            split_sentences(
                "A sentence that is wrapped\nacross two lines of text. And\tanother   one here."
            ),
            vec![
                "A sentence that is wrapped",
                "across two lines of text.",
                "And another one here.",
            ]
        );
    }

    #[test]
    fn split_keeps_closing_quotes_and_brackets() {
        assert_eq!(
            split_sentences("She said \"this is it.\" Then she left the room (quietly.) The end came soon after."),
            vec![
                "She said \"this is it.\"",
                "Then she left the room (quietly.)",
                "The end came soon after.",
            ]
        );
    }

    #[test]
    fn split_ignores_punctuation_inside_words() {
        assert_eq!(
            split_sentences("Pi is roughly 3.14159 and version 1.2.3 shipped today…"),
            vec!["Pi is roughly 3.14159 and version 1.2.3 shipped today…"]
        );
    }

    #[test]
    fn split_does_not_end_sentences_at_abbreviations() {
        assert_eq!(
            split_sentences(
                "Dr. Smith wrote to the U.S. Department of Energy. We met at 9 a.m. on Jan. 5, i.e. early."
            ),
            vec![
                "Dr. Smith wrote to the U.S. Department of Energy.",
                "We met at 9 a.m. on Jan. 5, i.e. early.",
            ]
        );
        assert_eq!(
            split_sentences(
                "Bring tools (e.g. hammers) and more. Mrs. Jones vs. Mr. Brown is next."
            ),
            vec![
                "Bring tools (e.g. hammers) and more.",
                "Mrs. Jones vs. Mr. Brown is next.",
            ]
        );
    }

    #[test]
    fn split_merges_short_fragments() {
        assert_eq!(
            split_sentences("We need tools like hammers and saws. Yes. That is all for today."),
            vec![
                "We need tools like hammers and saws. Yes.",
                "That is all for today."
            ]
        );
        assert_eq!(
            split_sentences("Hi. This sentence is long enough to stand alone."),
            vec!["Hi. This sentence is long enough to stand alone."]
        );
    }

    #[test]
    fn split_long_sentence_at_clause_breaks() {
        let clause = "word ".repeat(40); // 200 chars
        let text = format!("{}, {}, {}.", clause.trim(), clause.trim(), clause.trim());
        let parts = split_sentences(&text);
        assert_eq!(parts.len(), 3);
        assert!(parts[0].ends_with(','));
        assert!(parts
            .iter()
            .all(|p| p.chars().count() <= MAX_SENTENCE_CHARS));
        assert_eq!(parts.join(" "), text);
    }

    #[test]
    fn split_long_sentence_not_inside_numbers() {
        let text = format!(
            "{} it costs $1,500 at 10:30 and {}",
            "word ".repeat(50).trim(),
            "word ".repeat(20).trim()
        );
        let parts = split_sentences(&text);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains("$1,500 at 10:30"), "{:?}", parts[0]);
        assert_eq!(parts.join(" "), text);
    }

    #[test]
    fn split_long_sentence_remerges_short_pieces() {
        // The only clause break is near the start, and the cut leaves a short tail.
        let text = format!("Well, {}", "word ".repeat(60).trim());
        let parts = split_sentences(&text);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0], text);
    }

    #[test]
    fn split_long_sentence_without_breaks() {
        let words = "word ".repeat(100);
        let parts = split_sentences(words.trim());
        assert!(parts.len() > 1);
        assert!(parts
            .iter()
            .all(|p| p.chars().count() <= MAX_SENTENCE_CHARS));
        assert!(parts
            .iter()
            .all(|p| p.starts_with("word") && p.ends_with("word")));

        let blob = "x".repeat(650);
        let parts = split_sentences(&blob);
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![300, 300, 50]
        );
    }

    #[test]
    fn split_is_utf8_safe() {
        let accented = "é".repeat(350);
        let parts = split_sentences(&accented);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].chars().count(), 300);
        assert_eq!(parts[1], "é".repeat(50));

        let emoji = "🎉".repeat(650);
        let parts = split_sentences(&emoji);
        assert_eq!(parts.concat(), emoji);
        assert!(parts
            .iter()
            .all(|p| p.chars().count() <= MAX_SENTENCE_CHARS));

        assert_eq!(
            split_sentences(
                "Ça marche très bien aujourd’hui. Où est la bibliothèque, s’il vous plaît ?"
            ),
            vec![
                "Ça marche très bien aujourd’hui.",
                "Où est la bibliothèque, s’il vous plaît ?",
            ]
        );
    }

    #[test]
    fn split_never_returns_empty_strings() {
        assert!(split_sentences("").is_empty());
        assert!(split_sentences("   \n\n  ").is_empty());
        assert_eq!(split_sentences("..."), vec!["..."]);
    }

    #[test]
    fn hard_wrapped_paragraph_keeps_every_word_in_order() {
        let paragraph = "The quick brown fox jumps over the lazy dog and keeps\n\
                         running through the field until it reaches the river. Then\n\
                         it rests for a while, drinks some water, and goes home\n\
                         before the sun goes down.";
        let parts = split_sentences(&normalize(paragraph));
        assert_eq!(
            parts.join(" ").split_whitespace().collect::<Vec<_>>(),
            paragraph.split_whitespace().collect::<Vec<_>>(),
            "wrapping must not drop, duplicate or reorder words"
        );
        assert!(
            parts
                .iter()
                .all(|p| p.chars().count() <= MAX_SENTENCE_CHARS && !p.contains('\n')),
            "{parts:?}"
        );
    }

    #[test]
    fn abbreviations_survive_the_normalize_then_split_pipeline() {
        let text = normalize("Dr. Smith wrote to the U.S. Department of Energy about $1,500. At 10:30, we all leave together.");
        assert_eq!(
            split_sentences(&text),
            vec![
                "Dr. Smith wrote to the U.S. Department of Energy about $1,500.",
                "At 10:30, we all leave together.",
            ]
        );
    }

    #[test]
    fn splitting_a_large_json_blob_is_fast_and_lossless() {
        let blob = r#"{"ids": [1, 2, 3], "tags": ["a", "b"]}, "#.repeat(1_000);
        let started = Instant::now();
        let parts = split_sentences(&normalize(&blob));
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "took {:?}",
            started.elapsed()
        );
        assert!(parts
            .iter()
            .all(|p| p.chars().count() <= MAX_SENTENCE_CHARS));
    }

    fn first_chunks(text: &str) -> Vec<String> {
        split_first_clause(split_sentences(text))
    }

    #[test]
    fn long_first_sentence_splits_at_its_first_clause() {
        assert_eq!(
            first_chunks(
                "When the committee finally reconvened after the summer recess, it became clear that none of the proposals had survived. The chair asked for patience."
            ),
            vec![
                "When the committee finally reconvened after the summer recess,",
                "it became clear that none of the proposals had survived.",
                "The chair asked for patience.",
            ]
        );
    }

    #[test]
    fn only_the_first_sentence_is_split() {
        let text = "This opening sentence has quite a few words, so it gets cut right here. \
                    The second sentence also has many words, but it stays whole.";
        let chunks = first_chunks(text);
        assert_eq!(chunks.len(), 3);
        assert_eq!(
            chunks[2],
            "The second sentence also has many words, but it stays whole."
        );
    }

    #[test]
    fn short_first_sentence_stays_whole() {
        // Eight words: short enough already.
        let text = "One two three four five six, seven eight.";
        assert_eq!(first_chunks(text), vec![text]);
    }

    #[test]
    fn first_sentence_without_a_clause_break_stays_whole() {
        let text = "The quick brown fox jumps over the lazy dog near the riverbank in Oxford.";
        assert_eq!(first_chunks(text), vec![text]);
    }

    #[test]
    fn first_split_skips_a_head_too_short_to_stand_alone() {
        assert_eq!(
            first_chunks(
                "Well, the quick brown fox jumped over the dog, and then it ran away home."
            ),
            vec![
                "Well, the quick brown fox jumped over the dog,",
                "and then it ran away home.",
            ]
        );
    }

    #[test]
    fn first_split_keeps_a_short_tail_attached() {
        let text = "I walked all the way to the shop yesterday afternoon, alone.";
        assert_eq!(first_chunks(text), vec![text]);
    }

    #[test]
    fn first_split_takes_dashes_and_semicolons_but_not_numbers() {
        assert_eq!(
            first_chunks(
                "It costs $1,500 at 10:30 every single morning — and nobody seems to mind it."
            ),
            vec![
                "It costs $1,500 at 10:30 every single morning —",
                "and nobody seems to mind it.",
            ]
        );
        assert_eq!(
            first_chunks("I can't believe it's already October; we haven't even booked the hotel."),
            vec![
                "I can't believe it's already October;",
                "we haven't even booked the hotel.",
            ]
        );
        assert_eq!(
            first_chunks(
                "The long-delayed housing white paper arrived - finally - after many months."
            ),
            vec![
                "The long-delayed housing white paper arrived -",
                "finally - after many months.",
            ]
        );
    }

    #[test]
    fn first_split_never_breaks_a_number_range() {
        for text in [
            "The war that began in Europe that summer lasted from 1914–1918 and changed everything.",
            "The office downstairs is open from 9:00–17:00 on weekdays and closed at weekends.",
            "Tickets for the whole season went up from £5 – £10 each in a single year.",
            "Every class from the 1990s - 2000s is invited back to the reunion next spring.",
        ] {
            assert_eq!(first_chunks(text), vec![text]);
        }
        assert_eq!(
            first_chunks(
                "Charles Darwin (1809–1882) was an English naturalist, geologist and biologist."
            ),
            vec![
                "Charles Darwin (1809–1882) was an English naturalist,",
                "geologist and biologist.",
            ]
        );
        assert_eq!(
            first_chunks(
                "Prices have doubled since the 1990s — and nobody seems to mind it at all."
            ),
            vec![
                "Prices have doubled since the 1990s —",
                "and nobody seems to mind it at all."
            ]
        );
    }

    #[test]
    fn first_split_keeps_every_word_in_order() {
        let text = "Ministers are expected, it seems, to publish the paper before Parliament rises. Then what?";
        let chunks = first_chunks(text);
        assert_eq!(chunks.join(" "), text);
        assert!(chunks.iter().all(|c| !c.is_empty()));
        assert!(split_first_clause(Vec::new()).is_empty());
    }

    #[test]
    fn money_reads_as_words() {
        assert_eq!(expand_numbers("£4.50"), "four pounds and fifty pence");
        assert_eq!(expand_numbers("$12"), "twelve dollars");
        assert_eq!(expand_numbers("€3"), "three euros");
        assert_eq!(expand_numbers("£1 or $1"), "one pound or one dollar");
        assert_eq!(expand_numbers("$0.99"), "ninety-nine cents");
        assert_eq!(expand_numbers("£0.01"), "one penny");
        assert_eq!(expand_numbers("£2.00"), "two pounds");
        assert_eq!(
            expand_numbers("$1,500.05"),
            "one thousand five hundred dollars and five cents"
        );
        assert_eq!(
            expand_numbers("The average home costs £285,000."),
            "The average home costs two hundred and eighty-five thousand pounds."
        );
    }

    #[test]
    fn money_with_a_scale_word_names_the_currency_last() {
        assert_eq!(
            expand_numbers("Revenue rose to $3.2 billion in the quarter."),
            "Revenue rose to three point two billion dollars in the quarter."
        );
        assert_eq!(expand_numbers("€5 Million"), "five million euros");
        assert_eq!(expand_numbers("$3.5"), "three point five dollars");
    }

    #[test]
    fn decimals_read_digit_by_digit_after_the_point() {
        assert_eq!(expand_numbers("4.5"), "four point five");
        assert_eq!(
            expand_numbers("Pi is about 3.14159."),
            "Pi is about three point one four one five nine."
        );
        assert_eq!(expand_numbers("0.25"), "zero point two five");
        assert_eq!(
            expand_numbers("1,234.5"),
            "one thousand two hundred and thirty-four point five"
        );
    }

    #[test]
    fn versions_read_each_part_as_a_number() {
        assert_eq!(expand_numbers("1.2.3"), "one point two point three");
        assert_eq!(
            expand_numbers("Version 2.0.1 fixes it."),
            "Version two point zero point one fixes it."
        );
        assert_eq!(
            expand_numbers("v1.10.02"),
            "version one point ten point zero two"
        );
        assert_eq!(expand_numbers("v2.0"), "version two point zero");
        assert_eq!(expand_numbers("a V8 engine"), "a V8 engine");
    }

    #[test]
    fn four_digit_numbers_read_as_years() {
        assert_eq!(expand_numbers("1990"), "nineteen ninety");
        assert_eq!(
            expand_numbers("In 1666 it burned."),
            "In sixteen sixty-six it burned."
        );
        assert_eq!(expand_numbers("1905"), "nineteen oh five");
        assert_eq!(expand_numbers("1900"), "nineteen hundred");
        assert_eq!(expand_numbers("2000"), "two thousand");
        assert_eq!(expand_numbers("2005"), "two thousand and five");
        assert_eq!(expand_numbers("2026"), "twenty twenty-six");
        assert_eq!(expand_numbers("1066"), "ten sixty-six");
    }

    #[test]
    fn decades_read_as_plurals() {
        assert_eq!(
            expand_numbers("Prices doubled since the 1990s."),
            "Prices doubled since the nineteen nineties."
        );
        assert_eq!(expand_numbers("the 2000s"), "the two thousands");
        assert_eq!(expand_numbers("the 1900's"), "the nineteen hundreds");
        assert_eq!(expand_numbers("back in the '90s"), "back in the nineties");
        assert_eq!(expand_numbers("in their 30s"), "in their thirties");
        assert_eq!(expand_numbers("rolled two 5s"), "rolled two 5s");
    }

    #[test]
    fn ordinals_read_as_words() {
        assert_eq!(
            expand_numbers("1st 2nd 3rd 4th"),
            "first second third fourth"
        );
        assert_eq!(
            expand_numbers("11th 12th 13th"),
            "eleventh twelfth thirteenth"
        );
        assert_eq!(
            expand_numbers("21st 22nd 30th"),
            "twenty-first twenty-second thirtieth"
        );
        assert_eq!(expand_numbers("100th"), "one hundredth");
        assert_eq!(expand_numbers("101st"), "one hundred and first");
        assert_eq!(expand_numbers("3rd-party plugins"), "third-party plugins");
        assert_eq!(
            expand_numbers("the 1,000th visitor"),
            "the one thousandth visitor"
        );
    }

    #[test]
    fn twenty_four_hour_times_read_as_words() {
        assert_eq!(expand_numbers("10:30"), "ten thirty");
        assert_eq!(expand_numbers("14:05"), "fourteen oh five");
        assert_eq!(expand_numbers("9:00"), "nine o'clock");
        assert_eq!(expand_numbers("09:15"), "nine fifteen");
        assert_eq!(expand_numbers("21:00"), "twenty-one hundred");
        assert_eq!(expand_numbers("00:00"), "zero hundred");
        assert_eq!(expand_numbers("John 3:16"), "John three sixteen");
    }

    #[test]
    fn twelve_hour_times_read_with_am_and_pm() {
        assert_eq!(expand_numbers("3pm"), "three PM");
        assert_eq!(expand_numbers("at 3 PM today"), "at three PM today");
        assert_eq!(expand_numbers("10:30am"), "ten thirty AM");
        assert_eq!(
            expand_numbers("10:30 a.m. on Monday"),
            "ten thirty AM on Monday"
        );
        assert_eq!(expand_numbers("12:00 p.m."), "twelve PM.");
        assert_eq!(
            expand_numbers("We met at 9 a.m. Then we left."),
            "We met at nine AM. Then we left."
        );
        assert_eq!(expand_numbers("5 amps"), "5 amps");
    }

    #[test]
    fn impossible_times_stay_as_written() {
        assert_eq!(expand_numbers("25:00"), "25:00");
        assert_eq!(expand_numbers("10:75"), "10:75");
        assert_eq!(expand_numbers("13pm"), "13pm");
        assert_eq!(expand_numbers("a 16:9 screen"), "a 16:9 screen");
        assert_eq!(expand_numbers("10:30:15"), "10:30:15");
        assert_eq!(expand_numbers("won 3:1"), "won 3:1");
    }

    #[test]
    fn percentages_read_as_words() {
        assert_eq!(expand_numbers("50%"), "fifty percent");
        assert_eq!(
            expand_numbers("Rates held at 4.5% on Thursday."),
            "Rates held at four point five percent on Thursday."
        );
        assert_eq!(expand_numbers("-3%"), "minus three percent");
    }

    #[test]
    fn negative_numbers_keep_their_sign() {
        assert_eq!(expand_numbers("-5 degrees"), "minus 5 degrees");
        assert_eq!(expand_numbers("(-4.5)"), "(minus four point five)");
        assert_eq!(expand_numbers("pages 5-10"), "pages 5-10");
    }

    #[test]
    fn ranges_of_one_kind_read_with_to() {
        assert_eq!(
            expand_numbers("1914-1918"),
            "nineteen fourteen to nineteen eighteen"
        );
        assert_eq!(
            expand_numbers("open 9:00–17:00"),
            "open nine o'clock to seventeen hundred"
        );
        assert_eq!(expand_numbers("£5-£10"), "five pounds to ten pounds");
        assert_eq!(expand_numbers("2026-10-13"), "twenty twenty-six-10-13");
    }

    #[test]
    fn plain_numbers_and_codes_stay_as_written() {
        for text in [
            "37 bugs",
            "13,000 houses",
            "12345",
            "0123",
            "x86",
            "MP3",
            "room 2B",
            "5km",
            "abc123",
            "1,2,3",
            "US$5",
            "$",
            "£ and €",
        ] {
            assert_eq!(expand_numbers(text), text, "{text}");
        }
    }

    #[test]
    fn number_expansion_is_utf8_safe() {
        assert_eq!(
            expand_numbers("café 4.5€ 🎉 3rd"),
            "café four point five€ 🎉 third"
        );
        assert_eq!(expand_numbers("…1990…"), "…nineteen ninety…");
        assert_eq!(expand_numbers(""), "");
    }

    #[test]
    fn cardinals_cover_the_scales() {
        let words = |n| cardinal(n).unwrap();
        assert_eq!(words(0), "zero");
        assert_eq!(words(15), "fifteen");
        assert_eq!(words(40), "forty");
        assert_eq!(words(105), "one hundred and five");
        assert_eq!(words(1_005), "one thousand and five");
        assert_eq!(words(1_100), "one thousand one hundred");
        assert_eq!(words(2_000_001), "two million and one");
        assert_eq!(
            words(999_999_999_999),
            "nine hundred and ninety-nine billion nine hundred and ninety-nine million \
             nine hundred and ninety-nine thousand nine hundred and ninety-nine"
        );
        assert_eq!(cardinal(1_000_000_000_000), None);
        assert_eq!(expand_numbers("$1000000000000"), "$1000000000000");
    }
}
