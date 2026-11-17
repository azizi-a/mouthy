//! The splitters on random mixed-script text: no text lost, reordered or
//! invented, no empty chunk, no panic.

use super::text::{
    normalize, split_first_clause, split_sentences, split_sentences_with, Chunk, Marks,
};

/// Marks, closers, multi-byte letters, combining accents and emoji, so a
/// random string hits every cut path at every UTF-8 width.
const PIECES: &[&str] = &[
    "hello",
    "world",
    "the",
    "naïve",
    "Dr.",
    "etc.",
    "3.14",
    "1,500",
    "10:30",
    "foo.bar",
    "A.B.C.",
    " ",
    " ",
    " ",
    " ",
    "\n",
    "\t",
    ".",
    ".",
    "!",
    "?",
    "…",
    ",",
    ";",
    ":",
    "—",
    "–",
    "-",
    "\"",
    "'",
    "”",
    "’",
    ")",
    "]",
    "»",
    "(",
    "*",
    "`",
    "#",
    ">",
    "今",
    "天",
    "的",
    "天气",
    "日本語",
    "きょう",
    "です",
    "。",
    "！",
    "？",
    "，",
    "、",
    "「",
    "」",
    "；",
    "：",
    "。」",
    "！！",
    "。。",
    "…。",
    "आज",
    "मौसम",
    "है",
    "।",
    "॥",
    "क्या",
    "\u{94d}",
    "e\u{301}",
    "\u{301}",
    "a\u{308}",
    "\u{200d}",
    "🎉",
    "👨\u{200d}👩\u{200d}👧",
    "𝔘",
    "\u{feff}",
    "\u{a0}",
    "\r\n",
];

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

fn random_strings(seed: u64, count: usize) -> Vec<String> {
    let mut rng = Rng(seed);
    (0..count)
        .map(|_| {
            let len = rng.below(60);
            (0..len).map(|_| PIECES[rng.below(PIECES.len())]).collect()
        })
        .collect()
}

fn non_whitespace(text: &str) -> Vec<char> {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Panics, or says how the chunks lose or invent text.
fn check(label: &str, input: &str, chunks: &[Chunk]) {
    for chunk in chunks {
        assert!(
            !chunk.text.trim().is_empty(),
            "{label}: an empty chunk from {input:?}: {chunks:?}"
        );
    }
    let joined: String = chunks.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(
        non_whitespace(&joined),
        non_whitespace(input),
        "{label}: chunks lost or reordered text from {input:?}: {chunks:?}"
    );
}

#[test]
fn every_splitter_keeps_all_text_and_never_panics_on_mixed_scripts() {
    let marks = [
        ("DEFAULT", &Marks::DEFAULT),
        ("CJK", &Marks::CJK),
        ("HINDI", &Marks::HINDI),
    ];
    for text in random_strings(0x5eed_0001, 4000) {
        // `normalize` is what production passes; the raw string is the harder case.
        for input in [normalize(&text), text.clone()] {
            for max_chars in [3, 20, 100, 450] {
                check(
                    "split_sentences",
                    &input,
                    &split_sentences(&input, max_chars),
                );
                for (name, marks) in marks {
                    let label = format!("{name} max_chars={max_chars}");
                    let chunks = split_sentences_with(&input, max_chars, marks);
                    check(&label, &input, &chunks);
                    check(&label, &input, &split_first_clause(chunks, marks));
                }
            }
        }
    }
}

#[test]
fn a_mark_at_the_very_end_or_in_a_run_of_marks_splits_cleanly() {
    let edge_cases = [
        "ends with multibyte 、",
        "text。",
        "。",
        "。！？，、「」",
        "。。。！！？？",
        "「ok。」「next！」",
        "你好。」下一句。」",
        "आज है।॥",
        "end…。",
    ];
    for text in edge_cases {
        for max_chars in [1, 2, 3, 450] {
            for marks in [&Marks::DEFAULT, &Marks::CJK, &Marks::HINDI] {
                let chunks = split_sentences_with(text, max_chars, marks);
                check("edge", text, &chunks);
                check("edge", text, &split_first_clause(chunks, marks));
            }
        }
    }
}

#[test]
fn a_three_byte_mark_on_the_chunk_limit_is_cut_on_a_char_boundary() {
    // The limit lands just before, on and just after the 3-byte mark.
    for pad in 97..103 {
        let text = format!("{}，{}。", "あ".repeat(pad), "い".repeat(30));
        for max_chars in [pad - 1, pad, pad + 1, pad + 2] {
            let chunks = split_sentences_with(&text, max_chars, &Marks::CJK);
            check("limit on mark", &text, &chunks);
            assert!(
                chunks.iter().all(|c| c.text.chars().count() <= max_chars),
                "a chunk is over {max_chars} chars for pad {pad}: {chunks:?}"
            );
        }
    }
}
