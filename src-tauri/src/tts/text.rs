//! Text preparation for speech: markdown/URL cleanup and sentence chunking.

/// Fragments shorter than this are merged with a neighbour so the voice keeps
/// natural prosody across abbreviations ("e.g.") and one-word sentences.
const MIN_SENTENCE_CHARS: usize = 20;
/// Longer sentences are split so the first audio chunk arrives quickly.
const MAX_SENTENCE_CHARS: usize = 300;

const TERMINATORS: &[char] = &['.', '!', '?', '…'];
const CLOSERS: &[char] = &['"', '\'', '”', '’', ')', ']', '»'];
const URL_PREFIXES: &[&str] = &["https://", "http://", "www."];

/// Turns selected text into something worth reading aloud: URLs become "link",
/// markdown syntax is dropped and all whitespace collapses to single spaces.
pub fn normalize(input: &str) -> String {
    let lines: Vec<&str> = input.lines().map(strip_line_markers).collect();
    let text = replace_urls(&strip_links(&lines.join("\n")));
    strip_inline_markers(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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

/// `[text](url)` and `![alt](url)` keep only their text.
fn strip_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let link = rest[open + 1..].find("](").and_then(|close| {
            let label_end = open + 1 + close;
            if rest[open + 1..label_end].contains(['[', '\n']) {
                return None;
            }
            let url_end = rest[label_end + 2..].find(')')?;
            Some((label_end, label_end + 2 + url_end + 1))
        });
        let Some((label_end, link_end)) = link else {
            out.push_str(&rest[..=open]);
            rest = &rest[open + 1..];
            continue;
        };
        out.push_str(rest[..open].strip_suffix('!').unwrap_or(&rest[..open]));
        out.push_str(&rest[open + 1..label_end]);
        rest = &rest[link_end..];
    }
    out.push_str(rest);
    out
}

fn replace_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut prev: Option<char> = None;
    while let Some(c) = rest.chars().next() {
        let at_word_start = !prev.is_some_and(char::is_alphanumeric);
        let is_url = at_word_start
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
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut merged: Vec<String> = Vec::new();
    for fragment in raw_sentences(text) {
        match merged.last_mut() {
            Some(last)
                if fragment.chars().count() < MIN_SENTENCE_CHARS
                    || last.chars().count() < MIN_SENTENCE_CHARS =>
            {
                last.push(' ');
                last.push_str(fragment);
            }
            _ => merged.push(fragment.to_string()),
        }
    }

    let mut sentences = Vec::with_capacity(merged.len());
    for sentence in &merged {
        split_long(sentence, &mut sentences);
    }
    sentences
}

/// Splits after terminal punctuation (plus closing quotes/brackets) that is
/// followed by whitespace, so "3.14" and "e.g." mid-word stay intact.
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
        if chars.peek().is_none_or(|&(_, next)| next.is_whitespace()) {
            sentences.push(text[start..end].trim());
            start = end;
        }
    }
    sentences.push(text[start..].trim());
    sentences.retain(|s| !s.is_empty());
    sentences
}

/// Cuts at the last clause break before the limit, else the last space, else
/// mid-word; always on a char boundary.
fn split_long(sentence: &str, out: &mut Vec<String>) {
    let mut rest = sentence.trim();
    while let Some((limit, _)) = rest.char_indices().nth(MAX_SENTENCE_CHARS) {
        let head = &rest[..limit];
        let cut = head
            .rfind([',', ';', ':'])
            .map(|i| i + 1)
            .or_else(|| head.rfind(char::is_whitespace))
            .filter(|&i| i > 0)
            .unwrap_or(limit);
        let (chunk, tail) = rest.split_at(cut);
        out.push(chunk.trim_end().to_string());
        rest = tail.trim_start();
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(normalize("# Title\n## Sub heading"), "Title Sub heading");
        assert_eq!(normalize("> quoted\n> > nested"), "quoted nested");
        assert_eq!(normalize("- one\n* two\n+ three"), "one two three");
        assert_eq!(normalize("Issue #42 - fixed"), "Issue #42 - fixed");
    }

    #[test]
    fn normalize_turns_table_pipes_into_spaces() {
        assert_eq!(normalize("| a | b |\nx|y"), "a b x y");
    }

    #[test]
    fn normalize_collapses_whitespace_and_keeps_punctuation() {
        assert_eq!(
            normalize("  It's   fine,\n\n\tisn't it?  "),
            "It's fine, isn't it?"
        );
    }

    #[test]
    fn normalize_empty_and_whitespace_only() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize(" \n\t \r\n"), "");
        assert_eq!(normalize("** __ ``"), "");
    }

    #[test]
    fn normalize_combined_markdown_sample() {
        let input = "## Release notes\n\n\
                     > **Note:** see [the guide](https://example.com/guide).\n\n\
                     - Faster `startup`\n\
                     - Fixed _crash_ on https://bugs.example.com/1";
        assert_eq!(
            normalize(input),
            "Release notes Note: see the guide. Faster startup Fixed crash on link"
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
    fn split_merges_short_fragments() {
        assert_eq!(
            split_sentences("We need tools, e.g. hammers and saws. Yes. That is all for today."),
            vec![
                "We need tools, e.g. hammers and saws. Yes.",
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
        let accented = "é".repeat(301);
        let parts = split_sentences(&accented);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].chars().count(), 300);
        assert_eq!(parts[1], "é");

        let emoji = "🎉".repeat(305);
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
        assert!(split_sentences("   ").is_empty());
        assert_eq!(split_sentences("..."), vec!["..."]);
    }
}
