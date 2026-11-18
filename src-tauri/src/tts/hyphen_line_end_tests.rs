//! Text that wraps at a hyphen reaches the local English voice whole: the
//! words after the break are cut into chunks, phonemised and kept, whatever
//! the hyphen, the line break or the word.

use super::local::{in_vocab, open, phonemes_without_espeak, ModelFiles};
use super::text::{normalize, split_first_clause, split_sentences_with, Chunk};
use super::text_fuzz_tests::Rng;
use crate::settings::ModelVariant;
use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const BEFORE: &str = "The team shipped the release late last week and everyone was happy about it.";
const AFTER: &str =
    "Then the remaining sentence must still be spoken clearly. And the final one ends here.";

fn files() -> ModelFiles {
    ModelFiles::new(Path::new("models"), ModelVariant::Int8, "af_alloy")
}

/// The chunks `actions::sentences_for` cuts a selection into for the local English voice.
fn chunks(input: &str) -> Vec<Chunk> {
    let (files, text) = (files(), normalize(input));
    let marks = files.marks(&text);
    let sentences = split_sentences_with(&text, files.max_chunk_chars(&text), marks);
    split_first_clause(sentences, marks)
}

fn letters_and_digits(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Words a reader says aloud: whitespace-separated, with a letter or digit.
fn spoken_words(text: &str) -> usize {
    text.replace('\u{ad}', "")
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

/// A message for each of `inputs` that reaches the voice wrongly.
fn problems(inputs: &[String]) -> Vec<String> {
    let all: Vec<Vec<Chunk>> = inputs.iter().map(|input| chunks(input)).collect();
    let texts: Vec<String> = all
        .iter()
        .flatten()
        .map(|chunk| chunk.text.clone())
        .collect();
    // One call, since each builds the lexicon; a panic is then traced to its input.
    let Ok(phonemes) = catch_unwind(AssertUnwindSafe(|| {
        phonemes_without_espeak(&files(), &texts)
    })) else {
        return inputs
            .iter()
            .filter(|input| {
                let texts: Vec<String> = chunks(input).into_iter().map(|c| c.text).collect();
                catch_unwind(AssertUnwindSafe(|| {
                    phonemes_without_espeak(&files(), &texts)
                }))
                .is_err()
            })
            .map(|input| format!("panicked reading {input:?}"))
            .collect();
    };
    let mut phonemes = phonemes.into_iter();
    let mut found = Vec::new();
    for (input, chunks) in inputs.iter().zip(&all) {
        let mut wrong = Vec::new();
        let kept: Vec<&str> = chunks.iter().map(|chunk| chunk.text.as_str()).collect();
        if letters_and_digits(&kept.join(" ")) != letters_and_digits(input) {
            wrong.push(format!("words lost or changed, chunks are {kept:?}"));
        }
        if !chunks
            .last()
            .is_some_and(|c| letters_and_digits(&c.text).ends_with("endshere"))
        {
            wrong.push("the last sentence is not the last chunk".to_string());
        }
        for (chunk, phonemes) in chunks.iter().zip(phonemes.by_ref()) {
            if chunk.text.trim().is_empty() {
                wrong.push("blank chunk".to_string());
            }
            let groups = phonemes
                .split(' ')
                .filter(|group| group.chars().any(char::is_alphabetic))
                .count();
            if groups < spoken_words(&chunk.text) {
                wrong.push(format!(
                    "{groups} phoneme groups for {:?}: {phonemes:?}",
                    chunk.text
                ));
            }
            // misaki's ❓ marks a character it cannot read; any other unmapped symbol is a bug.
            if let Some(symbol) = phonemes.chars().find(|&c| c != '❓' && !in_vocab(c)) {
                wrong.push(format!("{symbol:?} has no token in {:?}", chunk.text));
            }
        }
        if !wrong.is_empty() {
            found.push(format!("input: {input:?}\n{wrong:#?}"));
        }
    }
    found
}

fn assert_clean(inputs: Vec<String>) {
    let found = problems(&inputs);
    assert!(
        found.is_empty(),
        "text after a line-end hyphen is lost or unreadable in {} of {} paragraphs, first:\n{}",
        found.len(),
        inputs.len(),
        found[0]
    );
}

const HYPHENS: &[&str] = &[
    "-", "\u{ad}", "\u{2011}", "\u{2010}", "\u{2013}", "\u{2014}", "\u{2212}", "--", "---", " -",
    " –", " —", "- ", "-  ",
];
const BREAKS: &[&str] = &["\n", "\r\n", "\n    ", "\n\t", " \n", "  \n", "\n\n", "\r"];
const WORDS_AROUND: &[(&str, &str)] = &[
    ("imple", "mentation of"),
    ("well", "known"),
    ("state-of-the", "art"),
    ("end", "of-life."),
    ("change", "log"),
    ("ab", "Capital"),
    ("ab", "42 things"),
    ("ab", "(paren)"),
    ("ab", ""),
    ("Dr", "Smith"),
    ("v1.0", "v2.0"),
];

#[test]
fn text_after_a_hyphen_at_a_line_end_is_still_read() {
    let mut inputs = Vec::new();
    for (head, tail) in WORDS_AROUND {
        for hyphen in HYPHENS {
            for line_break in BREAKS {
                inputs.push(format!("{BEFORE} {head}{hyphen}{line_break}{tail} {AFTER}"));
                inputs.push(format!(
                    "{BEFORE}\n{head}{hyphen}{line_break}{tail} {AFTER}\n"
                ));
            }
        }
    }
    assert_clean(inputs);
}

#[test]
fn lines_of_only_dashes_and_bullets_do_not_end_the_reading() {
    let mut inputs = Vec::new();
    for line in [
        "-",
        "--",
        "---",
        "- ",
        "-\n-\n-",
        "- one\n- two",
        "—",
        "–\n–",
    ] {
        inputs.push(format!("{BEFORE} {line} {AFTER}"));
        inputs.push(format!("{BEFORE}\n{line}\n{AFTER}"));
    }
    assert_clean(inputs);
}

const WORDS: &[&str] = &[
    "the",
    "of",
    "and",
    "release",
    "implementation",
    "well-known",
    "state-of-the-art",
    "changelog",
    "Kubernetes",
    "xkcd",
    "Dr.",
    "etc.",
    "3.14",
    "v1.2.3",
    "2026-10-13",
    "Siobhan",
    "it's",
    "don't",
    "end-of-life",
    "re-enable",
    "x-ray",
    "e-mail",
    "A.B.C.",
    "NASA",
    "API",
    "foo_bar",
    "COVID-19",
    "pre-\u{ad}processing",
    "co\u{ad}operate",
    "naïve",
    "café",
    "100%",
    "$5",
    "(note)",
    "\"quoted\"",
    "ab",
    "I",
    "a",
];
const SEPARATORS: &[&str] = &[
    " ", " ", " ", " ", ", ", ". ", "! ", "? ", "; ", ": ", " — ", " – ", " - ",
];
const LINE_ENDS: &[&str] = &[
    "\n", "\n", "\n", "\r\n", "\n  ", "\n    ", "\n\t", " \n", "\n\n",
];

fn pick<'a>(rng: &mut Rng, from: &[&'a str]) -> &'a str {
    from[rng.below(from.len())]
}

/// Words, hyphens of every kind and line breaks in random places.
fn scattered(rng: &mut Rng) -> String {
    let mut text = format!("{BEFORE} ");
    for _ in 0..2 + rng.below(25) {
        let word = pick(rng, WORDS);
        match rng.below(5) {
            0 => {
                let cut = word
                    .char_indices()
                    .nth(1 + rng.below(3))
                    .map_or(word.len(), |(i, _)| i);
                text += &format!(
                    "{}{}{}{}",
                    &word[..cut],
                    pick(rng, HYPHENS),
                    pick(rng, LINE_ENDS),
                    pick(rng, WORDS)
                );
            }
            1 => text += &format!("{word}{}{}", pick(rng, HYPHENS), pick(rng, LINE_ENDS)),
            2 => text += &format!("{word}{}", pick(rng, LINE_ENDS)),
            _ => text += word,
        }
        text += pick(rng, SEPARATORS);
    }
    text + AFTER
}

/// A paragraph hard-wrapped to a width, breaking after a hyphen or hyphenating a long word.
fn hard_wrapped(rng: &mut Rng) -> String {
    let mut text = String::new();
    for _ in 0..8 + rng.below(40) {
        text += pick(rng, WORDS);
        text += pick(rng, SEPARATORS);
    }
    text += AFTER;
    let width = 30 + rng.below(70);
    let indent = pick(rng, &["", "", "  ", "    ", "\t"]);
    let eol = pick(rng, &["\n", "\n", "\r\n"]);
    let (mut lines, mut line) = (Vec::new(), String::new());
    for word in text.split(' ').filter(|word| !word.is_empty()) {
        let mut rest = word.to_string();
        while !rest.is_empty() {
            let gap = usize::from(!line.is_empty());
            let room = width.saturating_sub(line.chars().count() + gap);
            if rest.chars().count() <= room {
                line += &" ".repeat(gap);
                line += &rest;
                break;
            }
            let at_hyphen = rest
                .char_indices()
                .filter(|&(i, c)| {
                    c == '-' && i + 1 < rest.len() && rest[..=i].chars().count() <= room
                })
                .map(|(i, _)| i + 1)
                .next_back();
            let hyphenate = (room >= 4 && rng.below(2) == 0)
                .then(|| rest.char_indices().nth(room - 1).map(|(i, _)| i))
                .flatten()
                .filter(|&i| i > 0);
            match (at_hyphen, hyphenate) {
                (Some(at), _) => {
                    line += &" ".repeat(gap);
                    line += &rest[..at];
                    rest = rest[at..].to_string();
                }
                (None, Some(at)) => {
                    line += &" ".repeat(gap);
                    line += &rest[..at];
                    line.push('-');
                    rest = rest[at..].to_string();
                }
                (None, None) if line.is_empty() => {
                    line = std::mem::take(&mut rest);
                }
                (None, None) => {}
            }
            lines.push(format!("{indent}{}", std::mem::take(&mut line)));
        }
    }
    lines.push(format!("{indent}{line}"));
    lines.join(eol)
}

#[test]
fn random_paragraphs_with_hyphens_and_line_breaks_lose_no_words() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let inputs = (0..1_000)
        .map(|i| {
            if i % 2 == 0 {
                scattered(&mut rng)
            } else {
                hard_wrapped(&mut rng)
            }
        })
        .collect();
    assert_clean(inputs);
}

/// Needs the model files: `MOUTHY_KOKORO_MODELS=<models dir> cargo test -- --ignored hyphen --nocapture`.
#[test]
#[ignore = "needs Kokoro model files"]
fn local_voice_speaks_every_chunk_around_a_line_end_hyphen() {
    let models = PathBuf::from(std::env::var("MOUTHY_KOKORO_MODELS").unwrap());
    let files = ModelFiles::new(&models, ModelVariant::Int8, "af_alloy");
    let mut loaded = open(&files, &Arc::new(AtomicBool::new(false))).unwrap();
    let mut seen = HashSet::new();
    for (head, tail) in WORDS_AROUND {
        for hyphen in HYPHENS {
            let input = format!("{BEFORE} {head}{hyphen}\n{tail} {AFTER}");
            for chunk in chunks(&input) {
                if !seen.insert(chunk.text.clone()) {
                    continue;
                }
                // A session skips a chunk with no audio silently: this is how speech goes missing.
                let samples = loaded.synthesize(&chunk.text, 1.0, chunk.ends_sentence);
                let samples = samples.unwrap_or_else(|e| panic!("{e} for {:?}", chunk.text));
                let words = spoken_words(&chunk.text);
                let ms = samples.len() * 1000 / crate::tts::SAMPLE_RATE as usize;
                assert!(
                    ms >= 100 * words,
                    "{ms} ms of audio for {words} words in {:?}",
                    chunk.text
                );
            }
        }
    }
}
