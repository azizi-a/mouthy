//! Text preparation for speech: markdown/URL cleanup and sentence chunking.

use std::ops::Range;

/// Fragments shorter than this are merged with a neighbour so the voice keeps
/// natural prosody across one-word sentences and clause cuts.
const MIN_SENTENCE_CHARS: usize = 20;

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

/// Turns selected text into something worth reading aloud: hard-wrapped lines
/// are joined, URLs become "link", markdown syntax is dropped and whitespace
/// collapses within each line. The remaining line breaks survive for
/// `split_sentences`; text with nothing to pronounce becomes "".
pub fn normalize(input: &str) -> String {
    let reflowed = reflow(input);
    let lines: Vec<&str> = reflowed.lines().map(strip_line_markers).collect();
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

/// A paragraph whose widest line is narrower than this keeps its line breaks:
/// at that width they belong to a list, code or verse rather than to wrapping.
const MIN_WRAP_CHARS: usize = 40;
/// Gutters drawn by terminal programs and email quoting, stripped where two or
/// more consecutive lines share one.
const GUTTERS: &[char] = &['│', '┃', '╎', '>'];
/// Claude Code marks a block's first line with one of these and indents the
/// rest to match.
const LEAD_MARKS: &[char] = &['⏺', '⎿'];
const BULLETS: &[char] = &['-', '*', '•'];
const TAB_STOP: usize = 8;

/// Joins lines that were hard-wrapped to a width, as terminals, man pages,
/// email and PDFs deliver them, so a wrapped sentence reads as one. A line
/// joins the next only when the next line's first word would not have fitted
/// on it. A break stays after sentence punctuation (`.!?…:;`) or a markdown
/// hard break, around headings, list items, quotes, prompts, tables, code,
/// chat transcripts and changes of indent, and wherever the lines are too
/// short to have been wrapped. Outside fenced code, the lines it keeps lose
/// their indent and are separated by a blank line, so reflowing the result
/// changes nothing.
pub fn reflow(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    // A stack rather than recursion: each level of `> > >` is one more pass.
    let mut pieces = vec![Piece::Block(text.split('\n').collect(), true)];
    while let Some(piece) = pieces.pop() {
        match piece {
            Piece::Block(lines, starts_text) => {
                pieces.extend(split_block(&lines, starts_text).into_iter().rev());
            }
            Piece::Paragraph(lines, starts_text) => reflow_lines(&lines, starts_text, &mut out),
            Piece::Code(lines) => {
                push_unit(&mut out, line_body(lines[0]).trim_end().to_string());
                out.extend(lines[1..].iter().map(|line| line.trim_end().to_string()));
                // A fence left open in a quote must not run on past it on a second pass.
                let marker = fence_marker(lines[0]);
                if lines.len() == 1 || lines.last().and_then(|line| fence_marker(line)) != marker {
                    out.extend(marker.map(str::to_string));
                }
            }
        }
    }
    out.join("\n")
}

enum Piece<'a> {
    /// Lines that may hold blank lines, fenced code and shared gutters.
    Block(Vec<&'a str>, bool),
    /// Lines with none of those, and whether they start the text.
    Paragraph(Vec<&'a str>, bool),
    /// Fenced code, from its opening fence to its closing one or the end.
    Code(Vec<&'a str>),
}

/// Splits lines at blank lines and fenced code, and each paragraph between
/// them at runs of lines sharing a gutter, which lose it and are split again.
fn split_block<'a>(lines: &[&'a str], starts_text: bool) -> Vec<Piece<'a>> {
    let mut pieces = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if is_blank(lines[i]) {
            i += 1;
            continue;
        }
        if let Some(marker) = fence_marker(lines[i]) {
            let end = lines[i + 1..]
                .iter()
                .position(|line| fence_marker(line) == Some(marker))
                .map_or(lines.len(), |at| i + at + 2);
            pieces.push(Piece::Code(lines[i..end].to_vec()));
            i = end;
            continue;
        }
        let end = lines[i..]
            .iter()
            .position(|line| is_blank(line) || fence_marker(line).is_some())
            .map_or(lines.len(), |at| i + at);
        let paragraph = &lines[i..end];
        for (range, shared) in gutter_runs(paragraph) {
            let run = &paragraph[range.clone()];
            pieces.push(match shared {
                Some(shared) => Piece::Block(
                    run.iter().map(|line| strip_gutter(line, shared)).collect(),
                    false,
                ),
                None => Piece::Paragraph(run.to_vec(), starts_text && i + range.start == 0),
            });
        }
        i = end;
    }
    pieces
}

fn push_unit(out: &mut Vec<String>, unit: String) {
    if out.last().is_some_and(|line| !line.is_empty()) {
        out.push(String::new());
    }
    out.push(unit);
}

fn is_box_drawing(c: char) -> bool {
    ('\u{2500}'..='\u{257f}').contains(&c)
}

/// The line without its indent, a byte order mark or the `LEAD_MARKS` before
/// its text.
fn line_body(line: &str) -> &str {
    fn skip_space(text: &str) -> &str {
        text.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
    }
    let mut body = skip_space(line);
    while let Some(after) = body
        .strip_prefix(LEAD_MARKS)
        .filter(|after| after.is_empty() || after.starts_with(' '))
    {
        body = skip_space(after);
    }
    body
}

fn is_blank(line: &str) -> bool {
    line_body(line)
        .chars()
        .all(|c| c.is_whitespace() || is_box_drawing(c))
}

fn fence_marker(line: &str) -> Option<&'static str> {
    let body = line_body(line);
    ["```", "~~~"]
        .into_iter()
        .find(|marker| body.starts_with(marker))
}

/// The gutter a line starts with; `>>>` is a Python prompt, not three quotes.
fn gutter(line: &str) -> Option<char> {
    let body = line.trim_start();
    if body.starts_with(">>>") {
        return None;
    }
    body.chars().next().filter(|c| GUTTERS.contains(c))
}

fn strip_gutter(line: &str, gutter: char) -> &str {
    let rest = &line.trim_start()[gutter.len_utf8()..];
    rest.strip_prefix(' ').unwrap_or(rest)
}

/// Splits lines into runs of two or more that share a gutter (`Some`) and the
/// lines between them (`None`).
fn gutter_runs(lines: &[&str]) -> Vec<(Range<usize>, Option<char>)> {
    let mut runs = Vec::new();
    let (mut plain_from, mut i) = (0, 0);
    while i < lines.len() {
        let shared = gutter(lines[i]);
        let len = lines[i..]
            .iter()
            .take_while(|line| shared.is_some() && gutter(line) == shared)
            .count();
        if len < 2 {
            i += 1;
            continue;
        }
        if plain_from < i {
            runs.push((plain_from..i, None));
        }
        runs.push((i..i + len, shared));
        i += len;
        plain_from = i;
    }
    if plain_from < lines.len() {
        runs.push((plain_from..lines.len(), None));
    }
    runs
}

fn reflow_lines(lines: &[&str], starts_text: bool, out: &mut Vec<String>) {
    let lines: Vec<Line> = lines.iter().map(|line| Line::parse(line)).collect();
    let Some(head) = lines.first() else {
        return;
    };
    let widest = lines.iter().map(|line| line.width).max().unwrap_or(0);
    let speakers = lines.iter().filter(|line| is_speaker(line.body)).count();
    let keep_all = is_code(&lines) || speakers >= 2;
    let mut unit = head.body.to_string();
    for (i, b) in lines.iter().enumerate().skip(1) {
        let a = &lines[i - 1];
        let decision = if keep_all {
            Join::Keep
        } else {
            join(a, b, lines.get(i + 1), starts_text && i == 1, widest)
        };
        match decision {
            Join::Keep => {
                push_unit(out, std::mem::take(&mut unit));
                unit.push_str(b.body);
            }
            Join::Space => {
                unit.push(' ');
                unit.push_str(b.body);
            }
            Join::Hyphen => unit.push_str(b.body),
        }
    }
    push_unit(out, unit);
}

/// How a line starts, which decides whether it can join its neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    Text,
    /// A list item or quote; lines indented to its text continue it.
    Marker,
    /// A heading, prompt, table or box row, or code fence: never joined.
    Fixed,
}

struct Line<'a> {
    /// The line without its indent, `LEAD_MARKS` or trailing whitespace.
    body: &'a str,
    /// In columns, a tab reaching the next multiple of `TAB_STOP`.
    indent: usize,
    width: usize,
    start: Start,
    /// The column a wrapped continuation of this line is indented to: after a
    /// list or quote marker, or after a wide gap as in `-A      Include …`.
    hang: Option<usize>,
    /// Two trailing spaces, markdown's line break.
    hard_break: bool,
}

impl<'a> Line<'a> {
    fn parse(line: &'a str) -> Self {
        let trimmed = line.trim_end_matches(|c: char| c.is_whitespace() || is_box_drawing(c));
        let body = line_body(trimmed);
        let indent = columns(0, &trimmed[..trimmed.len() - body.len()]);
        let (start, marker_len) = classify(body);
        let hang = match start {
            Start::Marker => Some(marker_len),
            Start::Text => definition_column(body),
            Start::Fixed => None,
        };
        Line {
            body,
            indent,
            width: columns(indent, body),
            start,
            hang: hang.map(|at| columns(indent, &body[..at])),
            hard_break: line.ends_with("  "),
        }
    }
}

fn columns(from: usize, text: &str) -> usize {
    text.chars().fold(from, |column, c| match c {
        '\t' => (column / TAB_STOP + 1) * TAB_STOP,
        _ => column + 1,
    })
}

/// The kind of start and, for a marker, its byte length with the spaces after it.
fn classify(body: &str) -> (Start, usize) {
    let unhashed = body.trim_start_matches('#');
    let heading = unhashed.len() < body.len() && (unhashed.is_empty() || unhashed.starts_with(' '));
    // Columns of a table: a tab, or a cell edge between words.
    let table = body.contains('\t') || body.contains(" │ ") || body.contains(" | ");
    if heading
        || table
        || ["$ ", ">>>", "... ", "|"]
            .iter()
            .any(|prefix| body.starts_with(prefix))
        || body.starts_with(is_box_drawing)
        || fence_marker(body).is_some()
    {
        return (Start::Fixed, 0);
    }
    if let Some(rest) = body.strip_prefix('>') {
        return (
            Start::Marker,
            body.len() - rest.trim_start_matches(' ').len(),
        );
    }
    match list_marker(body) {
        Some(len) => (Start::Marker, len),
        None => (Start::Text, 0),
    }
}

/// `- `, `* `, `• `, `1. ` or `2) `, as a byte length including the spaces.
fn list_marker(body: &str) -> Option<usize> {
    let first = body.chars().next()?;
    let marker = if BULLETS.contains(&first) {
        first.len_utf8()
    } else {
        let digits = body.len() - body.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if !(1..=3).contains(&digits) || !body[digits..].starts_with(['.', ')']) {
            return None;
        }
        digits + 1
    };
    let rest = &body[marker..];
    let spaces = rest.len() - rest.trim_start_matches(' ').len();
    (spaces > 0).then_some(marker + spaces)
}

/// The byte offset of the first word after a gap of two or more spaces or a
/// tab, where man pages and `--help` output indent an option's wrapped text.
fn definition_column(body: &str) -> Option<usize> {
    let mut gap = None;
    for (i, c) in body.char_indices() {
        if c == ' ' || c == '\t' {
            gap.get_or_insert(i);
        } else if let Some(from) = gap.take() {
            if i - from >= 2 || body[from..i].contains('\t') {
                return Some(i);
            }
        }
    }
    None
}

/// Code opens a block with a line ending in `{` or `:` and indents the next
/// one; its lines are statements, so none of them are joined.
fn is_code(lines: &[Line]) -> bool {
    lines.windows(2).any(|pair| {
        let [a, b] = pair else { return false };
        a.body.ends_with(['{', ':'])
            && b.start == Start::Text
            && b.indent > a.indent + 1
            && a.hang != Some(b.indent)
    })
}

/// A chat line: `alice: hi`, `Q: …`, or Discord's `azizi — Today at 10:42`.
fn is_speaker(body: &str) -> bool {
    let name = |word: &str| {
        !word.is_empty()
            && word
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-'))
    };
    let mut words = body.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    first.strip_suffix(':').is_some_and(name) || (name(first) && words.next() == Some("—"))
}

enum Join {
    Keep,
    Space,
    Hyphen,
}

/// Greedy wrapping moves a word to the next line only when it does not fit,
/// and fills every line but a paragraph's last, so a break that left room for
/// the next word, or before a line that itself left room, was made by hand.
fn join(a: &Line, b: &Line, c: Option<&Line>, first_pair: bool, widest: usize) -> Join {
    // The text's first line may be the tail of a line the selection began in,
    // so neither its indent nor its width is known.
    let cut = first_pair && a.indent == 0 && b.indent > 0;
    if breaks(a, b, cut, widest) {
        return Join::Keep;
    }
    let lowercase = starts_lowercase(b.body);
    let by_hand =
        fits(a, b, widest) || c.is_some_and(|c| !breaks(b, c, false, widest) && fits(b, c, widest));
    if by_hand && !(cut && lowercase) {
        return Join::Keep;
    }
    if wrapped_hyphen(a.body) {
        return if lowercase { Join::Hyphen } else { Join::Keep };
    }
    // A capital mid-sentence (a name, "I", a number) only next to lower-case prose,
    // so a list or a heading over a short line stays apart.
    let prose = lowercase
        || starts_lowercase(a.body)
        || c.is_some_and(|c| c.start == Start::Text && starts_lowercase(c.body));
    if prose {
        Join::Space
    } else {
        Join::Keep
    }
}

/// Breaks that stay whatever the widths.
fn breaks(a: &Line, b: &Line, cut: bool, widest: usize) -> bool {
    let continues = cut
        || (a.start == Start::Text && a.indent.abs_diff(b.indent) <= 1)
        || a.hang == Some(b.indent);
    !continues
        || b.start != Start::Text
        || a.start == Start::Fixed
        || a.hard_break
        || widest < MIN_WRAP_CHARS
        || ends_sentence(a.body)
}

/// Whether `b`'s first word would have fitted on `a`.
fn fits(a: &Line, b: &Line, widest: usize) -> bool {
    let word = b.body.split_whitespace().next().unwrap_or_default();
    a.width + 1 + word.chars().count() <= widest
}

/// A lower-case word, as wrapped prose continues; "a1b2c3d" and "x86" are not.
fn starts_lowercase(body: &str) -> bool {
    let word = body.split_whitespace().next().unwrap_or_default();
    word.starts_with(char::is_lowercase) && !word.contains(|c: char| c.is_ascii_digit())
}

fn ends_sentence(body: &str) -> bool {
    let end = body.trim_end_matches(|c: char| CLOSERS.contains(&c) || matches!(c, '*' | '_' | '`'));
    end.ends_with(|c: char| TERMINATORS.contains(&c) || c == ':' || c == ';')
        && !ends_with_abbreviation(end)
}

/// A word broken at a hyphen; the hyphen stays, since it may belong to a
/// compound such as "well-known".
fn wrapped_hyphen(body: &str) -> bool {
    let mut end = body.chars().rev();
    end.next() == Some('-') && end.next().is_some_and(char::is_alphabetic)
}

/// A first sentence longer than this many words is cut at a clause break, so
/// its audio is ready sooner.
const FIRST_CHUNK_WORDS: usize = 8;

/// Cuts the first sentence at its first clause break (`,;:` or a dash, but
/// not one inside a range such as "1914–1918") when it is longer than
/// `FIRST_CHUNK_WORDS`. Both parts keep at least `MIN_SENTENCE_CHARS`; without
/// such a break it stays whole.
pub fn split_first_clause(mut chunks: Vec<Chunk>) -> Vec<Chunk> {
    let Some(first) = chunks.first() else {
        return chunks;
    };
    if first.text.split_whitespace().count() <= FIRST_CHUNK_WORDS {
        return chunks;
    }
    if let Some((head, tail)) = first_clause_break(&first.text) {
        let parts = [
            Chunk {
                text: head.to_string(),
                ends_sentence: true,
            },
            Chunk {
                text: tail.to_string(),
                ends_sentence: first.ends_sentence,
            },
        ];
        chunks.splice(0..1, parts);
    }
    chunks
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
pub(super) fn joins_numbers(before: &str, after: &str) -> bool {
    let has_digit =
        |word: Option<&str>| word.is_some_and(|w| w.contains(|c: char| c.is_ascii_digit()));
    has_digit(before.split_whitespace().next_back()) && has_digit(after.split_whitespace().next())
}

/// Text synthesised in one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    /// False where a sentence was cut only for its length, so the local voice
    /// does not pause there as if the author had.
    pub ends_sentence: bool,
}

/// Splits normalized text into chunks to synthesise one request at a time,
/// cutting sentences longer than `max_chars`. Every line break ends a
/// sentence, so headings and bullets need no punctuation.
pub fn split_sentences(text: &str, max_chars: usize) -> Vec<Chunk> {
    let sentences = text
        .lines()
        .flat_map(raw_sentences)
        .map(collapse_whitespace);
    merge_short(sentences)
        .iter()
        .flat_map(|sentence| {
            let parts = merge_short(split_long(sentence, max_chars));
            let last = parts.len().saturating_sub(1);
            parts.into_iter().enumerate().map(move |(i, text)| Chunk {
                text,
                ends_sentence: i == last,
            })
        })
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
/// survive) before `max_chars`, else the last space, else mid-word; always on
/// a char boundary.
fn split_long(sentence: &str, max_chars: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = sentence.trim();
    while let Some((limit, _)) = rest.char_indices().nth(max_chars) {
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
            let (count, len) = dash_joined(rest);
            if count > 2 {
                // Dates, phone and card numbers ("2026-10-13", "020-7946-0958")
                // read better as written than as years or ranges.
                let (span, tail) = rest.split_at(len);
                out.push_str(span);
                rest = tail;
                prev = span.chars().next_back();
                continue;
            }
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

/// How many numbers are joined by dashes from the start of `text`, and the
/// bytes they span.
fn dash_joined(text: &str) -> (usize, usize) {
    let (mut count, mut len) = (0, 0);
    let (mut rest, mut prev) = (text, None);
    while let Some(number) = may_start_number(prev, rest)
        .then(|| Number::parse(rest))
        .flatten()
    {
        count += 1;
        len = text.len() - rest.len() + number.len;
        match rest[number.len..].strip_prefix(['-', '–']) {
            Some(after) => (rest, prev) = (after, Some('-')),
            None => break,
        }
    }
    (count, len)
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
        // British "7.30pm".
        let dotted_time = matches!(suffix, Suffix::Meridiem { .. })
            && minutes.is_none()
            && matches!(parts.as_slice(), [m] if m.len() == 2)
            && whole.len() <= 2
            && currency.is_none()
            && !negative
            && !version;
        let minutes = if dotted_time { parts.pop() } else { minutes };

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

/// British style, "one hundred and five"; the phonemiser reads "105" without
/// the "and". `None` from a trillion up.
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

/// What `spell_out` asks of the local voice's lexicon.
pub trait Lexicon {
    /// Whether it has an entry for `word` with exactly these capitals.
    fn has(&self, word: &str) -> bool;
    /// Text the phonemiser reads as the names of `letters` (ASCII capitals),
    /// ending in an "s" sound when `plural`.
    fn spell(&self, letters: &str, plural: bool) -> String;
}

/// Initialisms whose letters also spell a word the lexicon has. Next to
/// lower-case words they are the initialism ("your ID", "the US", "mac OS");
/// next to other capitals, a shouted word ("DO IT NOW").
const INITIALISMS_THAT_SPELL_WORDS: &[&str] = &[
    "AD", "AM", "BA", "BI", "COO", "ER", "ETA", "ID", "IDE", "IT", "LED", "MA", "OS", "PA", "SEC",
    "US",
];
/// Said as words but missing from the lexicon, and a spelling it reads that way.
const SAID_AS_WORDS: &[(&str, &str)] = &[
    ("AJAX", "ajax"),
    ("CORS", "cors"),
    ("CUDA", "cuda"),
    ("JSON", "jason"),
    ("TOML", "tom el"),
    ("UNIX", "unix"),
    ("YAML", "yam el"),
];
/// Longer capitals with a vowel are read as a word ("UNESCO").
const MAX_SPELLED_LETTERS: usize = 4;

/// Rewrites, for the local voice, words its phonemiser would misread:
/// initialisms are spelled letter by letter ("ID", "the API URL", "IDs") and
/// words joined in camel case are split ("GitHub", "macOS", "HTMLParser").
/// Words the lexicon has as written ("NASA", "iOS", "McDonald") and capitals
/// that spell an English word ("NOTE", "DO NOT") are left as they are.
pub fn spell_out(text: &str, lexicon: &impl Lexicon) -> String {
    let words = whitespace_words(text);
    let mut out = String::with_capacity(text.len() + text.len() / 2);
    let mut copied = 0;
    for (k, &(start, word)) in words.iter().enumerate() {
        out.push_str(&text[copied..start]);
        copied = start + word.len();
        let shouted_before = k > 0
            && word.starts_with(|c: char| c.is_ascii_alphabetic())
            && is_shouted_word(trailing_letters(words[k - 1].1), lexicon);
        let shouted_after = word.ends_with(|c: char| c.is_ascii_alphabetic())
            && words
                .get(k + 1)
                .is_some_and(|&(_, next)| is_shouted_word(leading_letters(next), lexicon));
        spell_word(word, shouted_before || shouted_after, lexicon, &mut out);
    }
    out.push_str(&text[copied..]);
    out
}

/// Each run of non-whitespace and the byte offset it starts at.
fn whitespace_words(text: &str) -> Vec<(usize, &str)> {
    let mut words = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(from)) => {
                words.push((from, &text[from..i]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        words.push((from, &text[from..]));
    }
    words
}

fn leading_letters(word: &str) -> &str {
    let len = word.bytes().take_while(u8::is_ascii_alphabetic).count();
    &word[..len]
}

fn trailing_letters(word: &str) -> &str {
    let len = word
        .bytes()
        .rev()
        .take_while(u8::is_ascii_alphabetic)
        .count();
    &word[word.len() - len..]
}

/// An English word in capitals, or the pronoun "I" or article "A".
fn is_shouted_word(word: &str, lexicon: &impl Lexicon) -> bool {
    matches!(word, "I" | "A")
        || (word.len() >= 2 && is_capitals(word) && knows_as_word(word, lexicon))
}

fn is_capitals(word: &str) -> bool {
    word.bytes().all(|b| b.is_ascii_uppercase())
}

/// The lexicon has the word as written, in lower case or capitalised.
fn knows_as_word(capitals: &str, lexicon: &impl Lexicon) -> bool {
    let lower = capitals.to_ascii_lowercase();
    let capitalised = format!("{}{}", &capitals[..1], &lower[1..]);
    lexicon.has(capitals) || lexicon.has(&lower) || lexicon.has(&capitalised)
}

/// Rewrites each run of ASCII letters in one whitespace-delimited word.
fn spell_word(word: &str, shouted: bool, lexicon: &impl Lexicon, out: &mut String) {
    let mut i = 0;
    while let Some(c) = word[i..].chars().next() {
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        let letters = leading_letters(&word[i..]);
        let after = &word[i + letters.len()..];
        // "ID's": the apostrophe and s belong to the initialism.
        let possessive = ["'s", "'S"].iter().any(|s| after.starts_with(s))
            && !after[2..].starts_with(|c: char| c.is_ascii_alphanumeric());
        let suffix_len = if possessive { 2 } else { 0 };
        match say_letters(letters, possessive, shouted, lexicon) {
            Some(said) => {
                out.push_str(&said);
                i += letters.len() + suffix_len;
            }
            None => {
                out.push_str(letters);
                i += letters.len();
            }
        }
    }
}

/// What to say for a run of letters, or `None` to leave it as written.
/// `possessive`: an "'s" follows, which the result then includes.
fn say_letters(
    letters: &str,
    possessive: bool,
    shouted: bool,
    lexicon: &impl Lexicon,
) -> Option<String> {
    if letters.len() >= 2 && is_capitals(letters) {
        return say_capitals(letters, possessive, shouted, lexicon);
    }
    // "URLs": capitals with a plural s.
    if let Some(stem) = letters.strip_suffix('s') {
        if stem.len() >= 2 && is_capitals(stem) && !possessive {
            return say_capitals(stem, true, shouted, lexicon);
        }
    }
    let respelt = SAID_AS_WORDS
        .iter()
        .find(|(capitals, _)| capitals.eq_ignore_ascii_case(letters))
        .filter(|_| letters.bytes().all(|b| b.is_ascii_lowercase()));
    if let Some((_, said)) = respelt {
        return Some(with_suffix(said, possessive));
    }
    let parts = camel_parts(letters, lexicon);
    if parts.len() < 2 {
        return None;
    }
    let last = parts.len() - 1;
    let said: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(n, part)| {
            let possessive = possessive && n == last;
            let capitals = part.len() >= 2 && is_capitals(part);
            capitals
                .then(|| say_capitals(part, possessive, false, lexicon))
                .flatten()
                .unwrap_or_else(|| with_suffix(part, possessive))
        })
        .collect();
    Some(said.join(" "))
}

fn with_suffix(word: &str, possessive: bool) -> String {
    if possessive {
        format!("{word}'s")
    } else {
        word.to_string()
    }
}

/// An all-capital word: spelled unless it is said as a word. `plural` covers
/// an "s" or "'s" after it.
fn say_capitals(
    capitals: &str,
    plural: bool,
    shouted: bool,
    lexicon: &impl Lexicon,
) -> Option<String> {
    if let Some((_, said)) = SAID_AS_WORDS.iter().find(|(word, _)| *word == capitals) {
        return Some(if plural {
            format!("{said}s")
        } else {
            said.to_string()
        });
    }
    let spell = if INITIALISMS_THAT_SPELL_WORDS.contains(&capitals) {
        !shouted
    } else {
        !knows_as_word(capitals, lexicon)
            && (capitals.len() <= MAX_SPELLED_LETTERS
                || !capitals.contains(['A', 'E', 'I', 'O', 'U', 'Y']))
    };
    spell.then(|| lexicon.spell(capitals, plural))
}

/// A word in camel case cut where a lower-case letter meets a capital
/// ("Git|Hub") and before the last capital of a run followed by two lower-case
/// letters ("HTML|Parser", but not "IPv6" or "IDs"). Words the lexicon has as
/// written ("iOS", "YouTube") and "Mc" names stay whole.
fn camel_parts<'a>(word: &'a str, lexicon: &impl Lexicon) -> Vec<&'a str> {
    if lexicon.has(word) || word.starts_with("Mc") {
        return vec![word];
    }
    let b = word.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    for i in 1..b.len() {
        let lower_to_upper = b[i - 1].is_ascii_lowercase() && b[i].is_ascii_uppercase();
        let upper_run_ends = b[i - 1].is_ascii_uppercase()
            && b[i].is_ascii_uppercase()
            && b.get(i + 1).is_some_and(u8::is_ascii_lowercase)
            && b.get(i + 2).is_some_and(u8::is_ascii_lowercase);
        if lower_to_upper || upper_run_ends {
            parts.push(&word[start..i]);
            start = i;
        }
    }
    parts.push(&word[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const MAX_CHARS: usize = 300;

    fn texts(chunks: Vec<Chunk>) -> Vec<String> {
        chunks.into_iter().map(|chunk| chunk.text).collect()
    }

    fn sentences(text: &str) -> Vec<String> {
        texts(split_sentences(text, MAX_CHARS))
    }

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

    /// The lines `reflow` keeps, without their indent or the blank lines between them.
    fn reflowed(text: &str) -> Vec<String> {
        reflow(text)
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| line.trim_start().to_string())
            .collect()
    }

    fn lines(lines: &[&str]) -> String {
        lines.join("\n")
    }

    /// Short real-world selections and the lines each should be read as.
    fn corpus() -> Vec<(&'static str, String, Vec<&'static str>)> {
        vec![
            (
                "Claude Code reply, 60 columns",
                lines(&[
                    "> can you check why the speak hotkey reads every wrapped",
                    "  line of my terminal selection as its own sentence",
                    "",
                    "⏺ The splitter treats every line break as the end of a",
                    "  sentence, so each wrapped line of a terminal selection",
                    "  gets its own pause and its own falling intonation. That",
                    "  rule exists so headings and bullets without punctuation",
                    "  still read as sentences.",
                    "",
                    "  Here is what I changed:",
                    "",
                    "  - Added reflow in src-tauri/src/tts/text.rs, which joins",
                    "    lines that were wrapped by the terminal",
                    "  - Kept the splitter as it was",
                    "  - Added tests for each rule and a small corpus of samples",
                    "    taken from Claude Code, man pages, Git and email",
                    "",
                    "  1. Run the tests with cargo test",
                    "  2. Select this reply in the terminal and press the hotkey;",
                    "     it should read as whole sentences",
                    "",
                    "⏺ Bash(cd src-tauri && cargo test tts::text)",
                    "  ⎿  running 48 tests",
                    "     test result: ok. 48 passed; 0 failed",
                ]),
                vec![
                    "> can you check why the speak hotkey reads every wrapped line of my terminal selection as its own sentence",
                    "The splitter treats every line break as the end of a sentence, so each wrapped line of a terminal selection gets its own pause and its own falling intonation. That rule exists so headings and bullets without punctuation still read as sentences.",
                    "Here is what I changed:",
                    "- Added reflow in src-tauri/src/tts/text.rs, which joins lines that were wrapped by the terminal",
                    "- Kept the splitter as it was",
                    "- Added tests for each rule and a small corpus of samples taken from Claude Code, man pages, Git and email",
                    "1. Run the tests with cargo test",
                    // A wrap right after `;` keeps its break.
                    "2. Select this reply in the terminal and press the hotkey;",
                    "it should read as whole sentences",
                    "Bash(cd src-tauri && cargo test tts::text)",
                    "running 48 tests",
                    "test result: ok. 48 passed; 0 failed",
                ],
            ),
            (
                "man ls",
                lines(&[
                    "DESCRIPTION",
                    "     For each operand that names a file of a type other than directory, ls",
                    "     displays its name as well as any requested, associated information.  For",
                    "     each operand that names a file of type directory, ls displays the names of",
                    "     files contained within that directory.",
                    "",
                    "     -A      Include directory entries whose names begin with a dot (‘.’) except",
                    "\t     for . and ...  Automatically set for the super-user unless -I is",
                    "\t     specified.",
                    "",
                    "     -D format",
                    "\t     When printing in the long (-l) format, use format to format the",
                    "\t     date and time output.",
                ]),
                vec![
                    "DESCRIPTION",
                    "For each operand that names a file of a type other than directory, ls displays its name as well as any requested, associated information.  For each operand that names a file of type directory, ls displays the names of files contained within that directory.",
                    "-A      Include directory entries whose names begin with a dot (‘.’) except for . and ...  Automatically set for the super-user unless -I is specified.",
                    "-D format",
                    "When printing in the long (-l) format, use format to format the date and time output.",
                ],
            ),
            (
                "Git commit body",
                lines(&[
                    "fix: show that the model is loading",
                    "",
                    "Recording starts while the model loads in the background. On a cold start",
                    "(first use, or after the idle unload) the user can finish speaking before",
                    "the load does, and the overlay then showed \"Transcribing...\" for the whole",
                    "load. With a slow disk or a large model that looks like the app has hung,",
                    "and people cancel and retry.",
                    "",
                    "Refs #2093",
                ]),
                vec![
                    "fix: show that the model is loading",
                    "Recording starts while the model loads in the background. On a cold start (first use, or after the idle unload) the user can finish speaking before the load does, and the overlay then showed \"Transcribing...\" for the whole load. With a slow disk or a large model that looks like the app has hung, and people cancel and retry.",
                    "Refs #2093",
                ],
            ),
            (
                "wrapped email",
                lines(&[
                    "Hi Sam,",
                    "",
                    "Thanks for sending the draft over. I read it on the train this morning and",
                    "I think the structure works, but the second section repeats a lot of what",
                    "the introduction already says.",
                    "",
                    "On Tue, 20 Oct 2026 at 09:12, Sam Lee <sam@example.com> wrote:",
                    "> Here is the draft of the proposal we talked about last week. It is still",
                    "> rough in places, so any comments are welcome.",
                    ">",
                    "> Sam",
                    "",
                    "Best,",
                    "Azizi",
                ]),
                vec![
                    "Hi Sam,",
                    "Thanks for sending the draft over. I read it on the train this morning and I think the structure works, but the second section repeats a lot of what the introduction already says.",
                    "On Tue, 20 Oct 2026 at 09:12, Sam Lee <sam@example.com> wrote:",
                    "Here is the draft of the proposal we talked about last week. It is still rough in places, so any comments are welcome.",
                    "Sam",
                    "Best,",
                    "Azizi",
                ],
            ),
            (
                "markdown document",
                lines(&[
                    "# Mouthy",
                    "",
                    "Mouthy reads the text you select in any app aloud. Press the hotkey once to",
                    "start and again to stop.",
                    "",
                    "## Install",
                    "- Download the latest build from the releases page",
                    "- Open it twice to confirm the Gatekeeper prompt, which macOS shows for apps",
                    "  that are not notarised",
                    "- Grant Accessibility when asked",
                    "",
                    "```sh",
                    "bun install",
                    "bun run tauri dev",
                    "```",
                ]),
                vec![
                    "# Mouthy",
                    "Mouthy reads the text you select in any app aloud. Press the hotkey once to start and again to stop.",
                    "## Install",
                    "- Download the latest build from the releases page",
                    "- Open it twice to confirm the Gatekeeper prompt, which macOS shows for apps that are not notarised",
                    "- Grant Accessibility when asked",
                    "```sh",
                    "bun install",
                    "bun run tauri dev",
                    "```",
                ],
            ),
            (
                "Safari paragraph",
                "Mouthy reads the text you select in any app aloud with an offline voice that runs on your Mac. Press the hotkey once to start and again to stop.".to_string(),
                vec!["Mouthy reads the text you select in any app aloud with an offline voice that runs on your Mac. Press the hotkey once to start and again to stop."],
            ),
            (
                "bullets of short lines",
                lines(&["- one", "- two", "* three", "• four"]),
                vec!["- one", "- two", "* three", "• four"],
            ),
            (
                "code without a fence",
                lines(&[
                    "function reflow(text) {",
                    "  const lines = text.split(\"\\n\")",
                    "  const joined = lines.map((line) => line.trimEnd()).filter(Boolean).join(\" \")",
                    "  return joined",
                    "}",
                ]),
                vec![
                    "function reflow(text) {",
                    "const lines = text.split(\"\\n\")",
                    "const joined = lines.map((line) => line.trimEnd()).filter(Boolean).join(\" \")",
                    "return joined",
                    "}",
                ],
            ),
        ]
    }

    #[test]
    fn reflow_reads_each_sample_as_intended() {
        for (name, text, expected) in corpus() {
            assert_eq!(reflowed(&text), expected, "{name}");
        }
    }

    #[test]
    fn reflow_is_idempotent() {
        let extra = [
            "> ```\n> code in a quote\n> ```\nafter it",
            "A long first line of wrapped prose that is followed by a short\nlast line.\nThen a capitalised line",
            "- item one that wraps onto a second line because it is long\n  enough\n  and then\n- item two",
            "\t│ a boxed line\n╰──╯\n  ⎿  ⏺ two marks before a line;",
            "⏺ ```sh\nls\n```\nafter the fence, some text that goes on",
            "  ⎿  ```\n     code\n     ```\ntext",
            "> ```sh\n> cd src-tauri\n> ```\nafter it",
            "> > ```\n> > left open in a quote\n```macOS\ntext after",
        ];
        let texts = corpus()
            .into_iter()
            .map(|(_, text, _)| text)
            .chain(extra.iter().map(|text| text.to_string()));
        for text in texts {
            let once = reflow(&text);
            assert_eq!(reflow(&once), once, "{text}");
        }
    }

    #[test]
    fn reflow_leaves_single_lines_alone() {
        for text in [
            "",
            "Hello.",
            "Issue #42 - fixed",
            "Mouthy reads the text you select in any app aloud. Press the hotkey to stop.",
        ] {
            assert_eq!(reflow(text), text);
        }
        assert_eq!(reflow("  An indented line   "), "An indented line");
        assert_eq!(reflow("⏺ Done."), "Done.");
        assert_eq!(reflow("│ ⏺ Boxed\n│ ⏺ marks"), "Boxed\n\nmarks");
    }

    #[test]
    fn reflow_keeps_breaks_after_sentence_punctuation() {
        let next = "and a lower-case next line";
        for end in [".", "!", "?", "…", ":", ";", ".\"", "?)", ".**"] {
            let text =
                format!("A line long enough to have been wrapped by the terminal{end}\n{next}");
            assert_eq!(reflowed(&text).len(), 2, "{text}");
        }
        let comma = format!("A line long enough to have been wrapped by the terminal,\n{next}");
        assert_eq!(reflowed(&comma).len(), 1);
        let abbreviation = "Bring the tools you need for the job, such as hammers, e.g.\nthe ones from the shed by the door";
        assert_eq!(reflowed(abbreviation).len(), 1);
    }

    #[test]
    fn reflow_keeps_headings_lists_quotes_prompts_and_tables() {
        let text = lines(&[
            "## A heading that is long enough to look like a wrapped line of text",
            "- a bullet that is long enough to look like a wrapped line of text too",
            "1. a numbered step long enough to look like it was wrapped as well",
            "> a quoted line",
            "$ cargo test --lib tts::text",
            "| a table row | with cells |",
            "│ a box row",
            "├── a tree branch",
            "a plain line after them all, which is long enough to have been wrapped",
        ]);
        assert_eq!(reflowed(&text).len(), 9);
        // A list item takes only the lines indented under its text.
        let lazy = "- a bullet that is long enough to look like a wrapped line\nthe next line, at the bullet's own indent";
        assert_eq!(reflowed(lazy).len(), 2);
    }

    #[test]
    fn reflow_joins_a_hanging_indent_to_its_marker() {
        let list = lines(&[
            "  - Added reflow in src-tauri/src/tts/text.rs, which joins lines that were",
            "    wrapped by the terminal and keeps real breaks",
            "  - Kept the splitter as it was",
            "  10. A numbered step that is long enough to wrap onto the line under it",
            "      and is indented to its text",
        ]);
        assert_eq!(
            reflowed(&list),
            vec![
                "- Added reflow in src-tauri/src/tts/text.rs, which joins lines that were wrapped by the terminal and keeps real breaks",
                "- Kept the splitter as it was",
                "10. A numbered step that is long enough to wrap onto the line under it and is indented to its text",
            ]
        );
        let prompt = "> can you check why the speak hotkey reads every wrapped line\n  of my terminal selection as its own sentence";
        assert_eq!(reflowed(prompt).len(), 1);
        let help = lines(&[
            "  -c, --config <FILE>    Sets a custom config file that is quite long and",
            "                         wraps under its description",
            "  -v, --verbose          Turns on verbose output",
        ]);
        assert_eq!(reflowed(&help).len(), 2);
    }

    #[test]
    fn reflow_keeps_changes_of_indent() {
        let text = lines(&[
            "Settings come from the store on disk.",
            "Every option is read at startup, and the ones that matter here are these",
            "    max_chunk_chars sets the longest piece of text sent in one request",
            "and the paragraph then carries on in lower case at its own indent",
        ]);
        assert_eq!(reflowed(&text).len(), 4);
    }

    #[test]
    fn reflow_keeps_code() {
        let fenced = lines(&[
            "Run these, then read on below",
            "```sh",
            "bun install",
            "bun run tauri dev -- and enough arguments to make this line long",
            "",
            "bun run lint",
            "```",
            "and the text after the block carries on in lower case like this",
        ]);
        assert_eq!(
            reflow(&fenced),
            lines(&[
                "Run these, then read on below",
                "",
                "```sh",
                "bun install",
                "bun run tauri dev -- and enough arguments to make this line long",
                "",
                "bun run lint",
                "```",
                "",
                "and the text after the block carries on in lower case like this",
            ])
        );
        let python = lines(&[
            "def reflow(text):",
            "    lines = text.splitlines()",
            "    joined = \" \".join(line.strip() for line in lines if line.strip())",
            "    return joined",
        ]);
        assert_eq!(reflowed(&python).len(), 4);
    }

    #[test]
    fn reflow_leaves_lines_too_short_to_be_wrapped() {
        for text in [
            "Installing the application\nDownload the latest build\nOpen it twice to confirm",
            "Roses are red\nviolets are blue",
            "src-tauri/src/tts\n├── mod.rs\n└── text.rs",
        ] {
            assert_eq!(reflowed(text), text.lines().collect::<Vec<_>>());
        }
    }

    #[test]
    fn reflow_joins_capitalised_lines_only_in_wrapped_prose() {
        let list = "Download the latest build from the releases page\nOpen it twice to confirm the Gatekeeper prompt\nGrant Accessibility when asked";
        assert_eq!(reflowed(list).len(), 3);
        let web = "Mouthy reads selected text aloud with an offline voice on your Mac\nInstall";
        assert_eq!(reflowed(web).len(), 2);
        let output = "$ cargo test\nrunning 48 tests\ntest result: ok. 48 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out";
        assert_eq!(reflowed(output).len(), 3);
    }

    #[test]
    fn reflow_joins_a_word_broken_at_a_hyphen() {
        assert_eq!(
            reflowed("The fox runs off into the neighbouring field, where the well-\nknown farmer keeps his sheep."),
            vec!["The fox runs off into the neighbouring field, where the well-known farmer keeps his sheep."]
        );
        let capital = "The fox runs off into the neighbouring field, where the Anglo-\nSaxon farmer keeps his sheep.";
        assert_eq!(reflowed(capital).len(), 2);
    }

    #[test]
    fn reflow_strips_terminal_chrome() {
        let boxed = lines(&[
            "╭──────────────────────────────────────────────────────────────╮",
            "│ The splitter treats every line break as the end of a         │",
            "│ sentence, so each wrapped line gets its own pause.           │",
            "╰──────────────────────────────────────────────────────────────╯",
            "────────────────────────────────────────────────────────────────",
            "│ A second box. It is not joined to the first one, even though │",
            "│ it starts in lower case.                                     │",
        ]);
        assert_eq!(
            reflowed(&boxed),
            vec![
                "The splitter treats every line break as the end of a sentence, so each wrapped line gets its own pause.",
                "A second box. It is not joined to the first one, even though it starts in lower case.",
            ]
        );
        let nested = lines(&[
            "> > Can you send the final numbers by Friday so that I can put them into",
            "> > the board pack?",
            "> Yes, I will have them ready by Thursday afternoon at the latest, and I",
            "> will copy Priya in as well.",
        ]);
        assert_eq!(
            reflowed(&nested),
            vec![
                "Can you send the final numbers by Friday so that I can put them into the board pack?",
                "Yes, I will have them ready by Thursday afternoon at the latest, and I will copy Priya in as well.",
            ]
        );
        // One quoted line in prose is a quote: it is neither stripped nor joined.
        let quote = "The reply said, in so many words, that the plan would not work\n> because the budget is too small\nand that we should try again next year with more money";
        assert_eq!(
            reflowed(quote),
            vec![
                "The reply said, in so many words, that the plan would not work",
                "> because the budget is too small",
                "and that we should try again next year with more money",
            ]
        );
    }

    #[test]
    fn reflow_allows_for_a_selection_that_starts_mid_line() {
        // The margin before the first word is not copied...
        let paragraph = lines(&[
            "The splitter treats every line break as the end of a sentence, so each wrapped",
            "  line of a terminal selection gets its own pause and its own falling",
            "  intonation.",
        ]);
        assert_eq!(reflowed(&paragraph).len(), 1);
        // ...nor is the start of the first line.
        let fragment = lines(&[
            "its own pause and its own falling",
            "  intonation. That rule exists so headings and bullets without punctuation still",
            "  read as sentences.",
        ]);
        assert_eq!(reflowed(&fragment).len(), 1);
    }

    #[test]
    fn reflow_reads_crlf_and_cr_as_line_breaks() {
        let text = "A line from a Windows file that was wrapped at seventy characters and\r\n\
                    then continues on the next line.\r\n\r\nSecond paragraph.\r\n";
        let expected = "A line from a Windows file that was wrapped at seventy characters and \
                        then continues on the next line.\n\nSecond paragraph.";
        assert_eq!(reflow(text), expected);
        assert_eq!(reflow(&text.replace("\r\n", "\r")), expected);
    }

    #[test]
    fn reflow_keeps_tables_and_chat_apart() {
        let cases = [
            "Task\tOwner\tStatus\tDue date\nwrite the reflow tests\tazizi\tdone\t2026-10-21\nupdate the changelog\tazizi\tin progress\t2026-10-22",
            "Setting | Default | Description\n------- | ------- | -----------\nvoice | af_alloy | the voice used for speech\nspeed | 1.0 | how fast the voice reads text aloud",
            "alice: hey, can you look at the reflow branch when you get a chance\nbob: sure, I am looking at it now and will reply in a minute or two\nalice: thanks, the tests are in text.rs near the bottom",
            "azizi — Today at 10:42\nhey, can you look at the reflow branch when you get a chance\nsam — Today at 10:43\nsure, looking at it now",
            "a1b2c3d fix: reflow wrapped lines before sentence splitting\n9f8e7d6 feat: default voice alloy for fresh installs\ne4f5a6b chore: bump version to 0.2.0 for the release",
            ">>> import mouthy.text as text_module_with_a_long_name_for_wrapping\n>>> text_module_with_a_long_name_for_wrapping.reflow(\"a\\nb\")",
        ];
        for text in cases {
            assert_eq!(reflowed(text).len(), text.lines().count(), "{text}");
        }
        let boxed = lines(&[
            "┌──────────────┬────────────────────────────────────────┐",
            "│ Setting      │ Description                            │",
            "├──────────────┼────────────────────────────────────────┤",
            "│ voice        │ the voice used for speech              │",
            "│ speed        │ how fast the voice reads text aloud    │",
            "└──────────────┴────────────────────────────────────────┘",
        ]);
        assert_eq!(
            reflowed(&boxed),
            vec![
                "│ Setting      │ Description",
                "voice        │ the voice used for speech",
                "speed        │ how fast the voice reads text aloud",
            ]
        );
    }

    #[test]
    fn reflow_keeps_program_output_apart() {
        let tests = lines(&[
            "⏺ Bash(cd src-tauri && cargo test tts::text)",
            "  ⎿  running 4 tests",
            "     test tts::text::tests::reflow_keeps_code ... ok",
            "     test tts::text::tests::reflow_is_idempotent ... ok",
            "     test tts::text::tests::normalize_reads_wrapped_lines_as_whole_sentences ... ok",
        ]);
        assert_eq!(reflowed(&tests).len(), 5);
        let log = lines(&[
            "⏺ Bash(git log --oneline -3)",
            "  ⎿  a1b2c3d fix: reflow wrapped lines before sentence splitting",
            "     9f8e7d6 feat: default voice alloy for fresh installs",
            "     e4f5a6b chore: bump version to 0.2.0 for the release",
        ]);
        assert_eq!(reflowed(&log).len(), 4);
    }

    #[test]
    fn reflow_keeps_a_short_line_that_left_room_for_the_next_word() {
        let headings = lines(&[
            "  Changes to the reflow rules in the text module",
            "  Second heading about the tests that were added",
            "  The splitter treats every line break as the end of a",
            "  sentence, so each wrapped line gets its own pause.",
        ]);
        assert_eq!(
            reflowed(&headings),
            vec![
                "Changes to the reflow rules in the text module",
                "Second heading about the tests that were added",
                "The splitter treats every line break as the end of a sentence, so each wrapped line gets its own pause.",
            ]
        );
        let order = "Thank you for your order, it will ship within two business days\nOrder number: 4821-7734\nestimated delivery is Thursday between 9am and 5pm";
        assert_eq!(reflowed(order).len(), 3);
    }

    #[test]
    fn reflow_keeps_markdown_hard_breaks_and_drops_a_byte_order_mark() {
        let address = "Mouthy Ltd, Suite 4, The Old Brewery, 12 High Street  \nCambridge CB1 2AB, United Kingdom  \nregistered in England and Wales, company no. 01234567";
        assert_eq!(reflowed(address).len(), 3);
        let wrapped =
            "A line that is long enough to have been wrapped by a terminal  \nand a next line";
        assert_eq!(reflowed(wrapped).len(), 2);
        assert_eq!(
            reflowed("\u{feff}# Release notes for the version shipped this week\nfixes a crash when the clipboard holds an image and nothing is selected"),
            vec![
                "# Release notes for the version shipped this week",
                "fixes a crash when the clipboard holds an image and nothing is selected",
            ]
        );
    }

    #[test]
    fn reflow_handles_long_and_deeply_quoted_text() {
        let wrapped =
            "a line of wrapped prose that goes on for quite a while before it ends\n".repeat(5_000);
        assert_eq!(reflowed(&wrapped).len(), 1);
        let deep = format!("{0}x\n{0}y", "> ".repeat(5_000));
        assert_eq!(reflowed(&deep), vec!["x", "y"]);
    }

    #[test]
    fn normalize_reads_wrapped_lines_as_whole_sentences() {
        let reply = lines(&[
            "⏺ The splitter treats every line break as the end of a",
            "  sentence, so each wrapped line of a terminal selection",
            "  gets its own pause. That rule exists so headings and",
            "  bullets without punctuation still read as sentences.",
            "",
            "  ## What changed in the splitter",
            "  - Kept the splitter as it was",
        ]);
        assert_eq!(
            sentences(&normalize(&reply)),
            vec![
                "The splitter treats every line break as the end of a sentence, so each wrapped line of a terminal selection gets its own pause.",
                "That rule exists so headings and bullets without punctuation still read as sentences.",
                "What changed in the splitter",
                "Kept the splitter as it was",
            ]
        );
        let headings =
            "Installing the application\nDownload the latest build\nOpen it twice to confirm";
        assert_eq!(normalize(headings), headings);
    }

    #[test]
    fn split_on_terminal_punctuation() {
        assert_eq!(
            sentences(
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
            sentences(
                "Installing the application\nDownload the latest build\nOpen it twice to confirm"
            ),
            vec![
                "Installing the application",
                "Download the latest build",
                "Open it twice to confirm",
            ]
        );
        assert_eq!(
            sentences(
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
            sentences("She said \"this is it.\" Then she left the room (quietly.) The end came soon after."),
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
            sentences("Pi is roughly 3.14159 and version 1.2.3 shipped today…"),
            vec!["Pi is roughly 3.14159 and version 1.2.3 shipped today…"]
        );
    }

    #[test]
    fn split_does_not_end_sentences_at_abbreviations() {
        assert_eq!(
            sentences(
                "Dr. Smith wrote to the U.S. Department of Energy. We met at 9 a.m. on Jan. 5, i.e. early."
            ),
            vec![
                "Dr. Smith wrote to the U.S. Department of Energy.",
                "We met at 9 a.m. on Jan. 5, i.e. early.",
            ]
        );
        assert_eq!(
            sentences("Bring tools (e.g. hammers) and more. Mrs. Jones vs. Mr. Brown is next."),
            vec![
                "Bring tools (e.g. hammers) and more.",
                "Mrs. Jones vs. Mr. Brown is next.",
            ]
        );
    }

    #[test]
    fn split_merges_short_fragments() {
        assert_eq!(
            sentences("We need tools like hammers and saws. Yes. That is all for today."),
            vec![
                "We need tools like hammers and saws. Yes.",
                "That is all for today."
            ]
        );
        assert_eq!(
            sentences("Hi. This sentence is long enough to stand alone."),
            vec!["Hi. This sentence is long enough to stand alone."]
        );
    }

    #[test]
    fn split_long_sentence_at_clause_breaks() {
        let clause = "word ".repeat(40); // 200 chars
        let text = format!("{}, {}, {}.", clause.trim(), clause.trim(), clause.trim());
        let parts = sentences(&text);
        assert_eq!(parts.len(), 3);
        assert!(parts[0].ends_with(','));
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_CHARS));
        assert_eq!(parts.join(" "), text);
    }

    #[test]
    fn split_long_sentence_not_inside_numbers() {
        let text = format!(
            "{} it costs $1,500 at 10:30 and {}",
            "word ".repeat(50).trim(),
            "word ".repeat(20).trim()
        );
        let parts = sentences(&text);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains("$1,500 at 10:30"), "{:?}", parts[0]);
        assert_eq!(parts.join(" "), text);
    }

    #[test]
    fn split_long_sentence_remerges_short_pieces() {
        // The only clause break is near the start, and the cut leaves a short tail.
        let text = format!("Well, {}", "word ".repeat(60).trim());
        let parts = sentences(&text);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0], text);
    }

    #[test]
    fn only_a_length_cut_does_not_end_a_sentence() {
        let long = "word ".repeat(70);
        let text = format!(
            "A heading long enough to stand alone\n{}\nThe last line, which has a full stop.",
            long.trim()
        );
        let ends: Vec<bool> = split_sentences(&text, MAX_CHARS)
            .iter()
            .map(|chunk| chunk.ends_sentence)
            .collect();
        assert_eq!(ends, vec![true, false, true, true]);
    }

    #[test]
    fn a_first_clause_cut_ends_a_chunk_and_keeps_the_sentence_end() {
        let first = Chunk {
            text: "When the committee reconvened after the recess, it became clear that none had survived".into(),
            ends_sentence: false,
        };
        let ends: Vec<bool> = split_first_clause(vec![first])
            .iter()
            .map(|chunk| chunk.ends_sentence)
            .collect();
        assert_eq!(ends, vec![true, false]);
    }

    #[test]
    fn split_long_sentence_without_breaks() {
        let words = "word ".repeat(100);
        let parts = sentences(words.trim());
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_CHARS));
        assert!(parts
            .iter()
            .all(|p| p.starts_with("word") && p.ends_with("word")));

        let blob = "x".repeat(650);
        let parts = sentences(&blob);
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![300, 300, 50]
        );
    }

    #[test]
    fn the_limit_decides_whether_a_long_sentence_is_cut() {
        let words = "word ".repeat(70);
        let sentence = words.trim();
        assert_eq!(split_sentences(sentence, 300).len(), 2);
        assert_eq!(texts(split_sentences(sentence, 450)), vec![sentence]);
    }

    #[test]
    fn split_is_utf8_safe() {
        let accented = "é".repeat(350);
        let parts = sentences(&accented);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].chars().count(), 300);
        assert_eq!(parts[1], "é".repeat(50));

        let emoji = "🎉".repeat(650);
        let parts = sentences(&emoji);
        assert_eq!(parts.concat(), emoji);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_CHARS));

        assert_eq!(
            sentences("Ça marche très bien aujourd’hui. Où est la bibliothèque, s’il vous plaît ?"),
            vec![
                "Ça marche très bien aujourd’hui.",
                "Où est la bibliothèque, s’il vous plaît ?",
            ]
        );
    }

    #[test]
    fn split_never_returns_empty_strings() {
        assert!(sentences("").is_empty());
        assert!(sentences("   \n\n  ").is_empty());
        assert_eq!(sentences("..."), vec!["..."]);
    }

    #[test]
    fn hard_wrapped_paragraph_keeps_every_word_in_order() {
        let paragraph = "The quick brown fox jumps over the lazy dog and keeps\n\
                         running through the field until it reaches the river. Then\n\
                         it rests for a while, drinks some water, and goes home\n\
                         before the sun goes down.";
        let parts = sentences(&normalize(paragraph));
        assert_eq!(
            parts.join(" ").split_whitespace().collect::<Vec<_>>(),
            paragraph.split_whitespace().collect::<Vec<_>>(),
            "wrapping must not drop, duplicate or reorder words"
        );
        assert!(
            parts
                .iter()
                .all(|p| p.chars().count() <= MAX_CHARS && !p.contains('\n')),
            "{parts:?}"
        );
    }

    #[test]
    fn abbreviations_survive_the_normalize_then_split_pipeline() {
        let text = normalize("Dr. Smith wrote to the U.S. Department of Energy about $1,500. At 10:30, we all leave together.");
        assert_eq!(
            sentences(&text),
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
        let parts = sentences(&normalize(&blob));
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "took {:?}",
            started.elapsed()
        );
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_CHARS));
    }

    fn first_chunks(text: &str) -> Vec<String> {
        texts(split_first_clause(split_sentences(text, MAX_CHARS)))
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
    fn british_dotted_times_read_as_times() {
        assert_eq!(expand_numbers("7.30pm"), "seven thirty PM");
        assert_eq!(
            expand_numbers("at 7.30 p.m. on Friday"),
            "at seven thirty PM on Friday"
        );
        assert_eq!(expand_numbers("7.05am"), "seven oh five AM");
        assert_eq!(expand_numbers("7.3pm"), "7.3pm");
        assert_eq!(expand_numbers("7.75pm"), "7.75pm");
        assert_eq!(expand_numbers("7.30"), "seven point three zero");
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
        assert_eq!(
            expand_numbers("7.30pm–9.30pm"),
            "seven thirty PM to nine thirty PM"
        );
    }

    #[test]
    fn three_or_more_joined_numbers_stay_as_written() {
        for text in [
            "2026-10-13",
            "on 2026-10-13.",
            "1234-5678-9012-3456",
            "call 020-7946-0958 now",
            "1914-1918-1939",
            "£5-£10-£20",
        ] {
            assert_eq!(expand_numbers(text), text, "{text}");
        }
        assert_eq!(
            expand_numbers("1914-1918 and 2026-10-13"),
            "nineteen fourteen to nineteen eighteen and 2026-10-13"
        );
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

    /// A lexicon with a few entries that marks spelled letters `<A P I>`.
    struct Words(&'static [&'static str]);

    impl Lexicon for Words {
        fn has(&self, word: &str) -> bool {
            self.0.contains(&word)
        }

        fn spell(&self, letters: &str, plural: bool) -> String {
            let spaced: Vec<String> = letters.chars().map(String::from).collect();
            let s = if plural { "s" } else { "" };
            format!("<{}>{s}", spaced.join(" "))
        }
    }

    const WORDS: Words = Words(&[
        "NASA", "GIF", "RAM", "covid", "iOS", "YouTube", "McDonald", "IT", "US", "ID", "OS", "AM",
        "it", "us", "id", "do", "not", "now", "let", "know", "note", "must", "the", "you", "here",
        "arm", "pm", "readme",
    ]);

    fn spelled(text: &str) -> String {
        spell_out(text, &WORDS)
    }

    #[test]
    fn initialisms_are_spelled() {
        assert_eq!(
            spelled("Copy the ID from the URL."),
            "Copy the <I D> from the <U R L>."
        );
        assert_eq!(spelled("the API URL"), "the <A P I> <U R L>");
        assert_eq!(spelled("I use API keys"), "I use <A P I> keys");
        assert_eq!(spelled("(HTTPS)"), "(<H T T P S>)");
    }

    #[test]
    fn a_plural_or_possessive_s_joins_the_initialism() {
        assert_eq!(spelled("IDs and URLs"), "<I D>s and <U R L>s");
        assert_eq!(spelled("the API's docs"), "the <A P I>s docs");
        assert_eq!(spelled("CPUs, GPUs."), "<C P U>s, <G P U>s.");
    }

    #[test]
    fn capitals_the_lexicon_reads_as_words_stay() {
        for kept in [
            "NASA and GIF",
            "more RAM",
            "NOTE: you MUST",
            "DO NOT",
            "ARM64",
            "seven PM",
        ] {
            assert_eq!(spelled(kept), kept, "{kept}");
        }
    }

    #[test]
    fn longer_capitals_are_a_word_when_they_have_a_vowel() {
        assert_eq!(spelled("UNESCO and README"), "UNESCO and README");
        assert_eq!(spelled("HTTPS"), "<H T T P S>");
        assert_eq!(spelled("WWDC"), "<W W D C>");
    }

    #[test]
    fn single_letters_stay() {
        for kept in ["I think a B is fine", "Plan A", "x", "3D", "H2O"] {
            assert_eq!(spelled(kept), kept);
        }
    }

    #[test]
    fn an_initialism_that_spells_a_word_is_a_word_among_capitals() {
        assert_eq!(spelled("the IT team"), "the <I T> team");
        assert_eq!(spelled("the US and the UK"), "the <U S> and the <U K>");
        assert_eq!(spelled("seven AM"), "seven <A M>");
        for kept in ["DO IT NOW", "LET US KNOW", "I AM here", "DO IT, now"] {
            assert_eq!(spelled(kept), kept, "{kept}");
        }
        // A comma or an unknown initialism is no shouted neighbour.
        assert_eq!(spelled("IT, US"), "<I T>, <U S>");
        assert_eq!(spelled("the API ID"), "the <A P I> <I D>");
    }

    #[test]
    fn camel_case_is_split_into_words() {
        assert_eq!(spelled("GitHub"), "Git Hub");
        assert_eq!(spelled("macOS"), "mac <O S>");
        assert_eq!(spelled("HTMLParser"), "<H T M L> Parser");
        assert_eq!(spelled("iPhone"), "i Phone");
        assert_eq!(spelled("OAuth"), "O Auth");
        assert_eq!(spelled("XMLHttpRequest"), "<X M L> Http Request");
        assert_eq!(spelled("GitHub's API"), "Git Hub's <A P I>");
    }

    #[test]
    fn camel_case_the_lexicon_has_stays_whole() {
        for kept in [
            "iOS 17", "YouTube", "McDonald", "McKinsey", "IPv6", "Hello", "hello",
        ] {
            assert_eq!(spelled(kept), kept, "{kept}");
        }
    }

    #[test]
    fn acronyms_said_as_words_are_respelt() {
        assert_eq!(spelled("JSON and YAML"), "jason and yam el");
        assert_eq!(spelled("a json file"), "a jason file");
        assert_eq!(spelled("JSONs"), "jasons");
        assert_eq!(spelled("Json"), "Json");
    }

    #[test]
    fn letters_next_to_digits_and_punctuation_are_spelled_alone() {
        assert_eq!(spelled("MP3 and ID3"), "<M P>3 and <I D>3");
        assert_eq!(spelled("URL: here"), "<U R L>: here");
        assert_eq!(spelled("EC2-API"), "<E C>2-<A P I>");
        assert_eq!(spelled("COVID-19"), "COVID-19");
    }

    #[test]
    fn spelling_keeps_every_other_character() {
        for kept in [
            "",
            "   ",
            "café ünïcode 日本",
            "don't O'Reilly",
            "a\tb\n c ",
        ] {
            assert_eq!(spelled(kept), kept);
        }
        assert_eq!(spelled("  the  ID\n"), "  the  <I D>\n");
    }
}
