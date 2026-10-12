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
}
