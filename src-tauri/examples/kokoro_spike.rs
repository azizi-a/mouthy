//! Spike: Kokoro-82M in-process through `ort` and `misaki-rs`. Not part of the app.
//!
//! Loads an onnx-community Kokoro export and a raw voice `.bin`, times session
//! load and warm inference, and writes WAVs for a human to check. `--corpus`
//! also runs the G2P corpus below and reports OOV words and symbols Kokoro
//! cannot take.
//!
//! ```text
//! cargo run --release --example kokoro_spike -- <model.onnx> <voice.bin> <out_dir>
//!     [--threads N] [--runs N] [--corpus] [--raw] [--play] [--no-spin] [--default-policy]
//! ```

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use misaki_rs::fallback::FallbackError;
use misaki_rs::{Fallback, Language, G2P};
use ort::environment::Environment;
use ort::session::builder::AutoDevicePolicy;
use ort::session::Session;
use ort::value::{Tensor, TensorRef};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const SAMPLE_RATE: u32 = 24_000;
/// Kokoro's context is 512 ids, two of which are the pads around the text.
const MAX_TOKENS: usize = 510;
const STYLE_DIM: usize = 256;

/// Kokoro v1.0 phoneme vocabulary (onnx-community `tokenizer.json`; id 0 is the pad).
#[rustfmt::skip]
const VOCAB: &[(char, i64)] = &[
    (';', 1), (':', 2), (',', 3), ('.', 4), ('!', 5), ('?', 6), ('—', 9), ('…', 10), ('"', 11),
    ('(', 12), (')', 13), ('“', 14), ('”', 15), (' ', 16), ('\u{303}', 17), ('ʣ', 18),
    ('ʥ', 19), ('ʦ', 20), ('ʨ', 21), ('ᵝ', 22), ('\u{ab67}', 23), ('A', 24), ('I', 25),
    ('O', 31), ('Q', 33), ('S', 35), ('T', 36), ('W', 39), ('Y', 41), ('ᵊ', 42), ('a', 43),
    ('b', 44), ('c', 45), ('d', 46), ('e', 47), ('f', 48), ('h', 50), ('i', 51), ('j', 52),
    ('k', 53), ('l', 54), ('m', 55), ('n', 56), ('o', 57), ('p', 58), ('q', 59), ('r', 60),
    ('s', 61), ('t', 62), ('u', 63), ('v', 64), ('w', 65), ('x', 66), ('y', 67), ('z', 68),
    ('ɑ', 69), ('ɐ', 70), ('ɒ', 71), ('æ', 72), ('β', 75), ('ɔ', 76), ('ɕ', 77), ('ç', 78),
    ('ɖ', 80), ('ð', 81), ('ʤ', 82), ('ə', 83), ('ɚ', 85), ('ɛ', 86), ('ɜ', 87), ('ɟ', 90),
    ('ɡ', 92), ('ɥ', 99), ('ɨ', 101), ('ɪ', 102), ('ʝ', 103), ('ɯ', 110), ('ɰ', 111),
    ('ŋ', 112), ('ɳ', 113), ('ɲ', 114), ('ɴ', 115), ('ø', 116), ('ɸ', 118), ('θ', 119),
    ('œ', 120), ('ɹ', 123), ('ɾ', 125), ('ɻ', 126), ('ʁ', 128), ('ɽ', 129), ('ʂ', 130),
    ('ʃ', 131), ('ʈ', 132), ('ʧ', 133), ('ʊ', 135), ('ʋ', 136), ('ʌ', 138), ('ɣ', 139),
    ('ɤ', 140), ('χ', 142), ('ʎ', 143), ('ʒ', 147), ('ʔ', 148), ('ˈ', 156), ('ˌ', 157),
    ('ː', 158), ('ʰ', 162), ('ʲ', 164), ('↓', 169), ('→', 171), ('↗', 172), ('↘', 173),
    ('ᵻ', 177),
];

/// misaki-rs 0.6 lexicons tie diphthongs and affricates with U+200D, eSpeak
/// style; Kokoro was trained on misaki's single-symbol forms.
const TIES_COMMON: &[(&str, &str)] = &[
    ("e\u{200d}ɪ", "A"),
    ("a\u{200d}ɪ", "I"),
    ("a\u{200d}ʊ", "W"),
    ("ɔ\u{200d}ɪ", "Y"),
    ("d\u{200d}ʒ", "ʤ"),
    ("t\u{200d}ʃ", "ʧ"),
    ("ə\u{200d}l", "ᵊl"),
    ("ɬ", "l"),
];
const TIES_GB: &[(&str, &str)] = &[
    ("ə\u{200d}ʊ", "Q"),
    ("i\u{200d}ə", "ɪə"),
    ("e\u{200d}ə", "ɛː"),
];
const TIES_US: &[(&str, &str)] = &[
    ("o\u{200d}ʊ", "O"),
    ("ː\u{200d}ɹ", "ɹ"),
    ("ː", ""),
    ("n\u{329}", "ᵊn"),
];

/// Sentences of 3, 5, 14 and 28 words.
const BENCH: &[&str] = &[
    "Please read this.",
    "Please read this sentence aloud.",
    "The quick brown fox jumps over the lazy dog near the riverbank in Oxford.",
    "When the committee finally reconvened after the summer recess, it became clear that none \
     of the proposals circulated in July had survived contact with the treasury's revised \
     forecasts.",
];

const CORPUS: &[&str] = &[
    // News prose
    "The Bank of England held interest rates at 4.5% on Thursday, citing persistent inflation in the services sector.",
    "Ministers are expected to publish the long-delayed housing white paper before Parliament rises for the summer recess.",
    "Heavy rain caused flooding across parts of Yorkshire and Lancashire, with several train services cancelled overnight.",
    "The government said the new rules would come into force on 1 April 2027, subject to a final vote in the Commons.",
    "Shares in the retailer fell sharply after it warned that profits would be lower than analysts had forecast.",
    // Code docs
    "Kubernetes schedules pods onto nodes, and the kubelet restarts any container whose liveness probe fails.",
    "Run the migration against PostgreSQL before deploying, otherwise the API will return a 500 error.",
    "The async runtime spawns a worker thread per core; call block_on only from a dedicated thread.",
    "Set the RUST_LOG environment variable to debug to see every HTTP request the client makes.",
    "Clone the repository from GitHub, then run npm install and start the dev server with Vite.",
    // British places and surnames
    "We drove from Leicester to Loughborough, then on to Worcester and Gloucester before dark.",
    "Mr Featherstonehaugh and Mrs Cholmondeley were seated beside Siobhan and Niamh at dinner.",
    "The train from Edinburgh calls at Berwick-upon-Tweed, Alnmouth and Newcastle.",
    "She grew up in Keighley, studied at Magdalen College, Oxford, and now lives near Southwark.",
    // Numbers, dates, currency
    "The tickets cost £45 each, or £120 for a family of four.",
    "Revenue rose to $3.2 billion in the third quarter, up 12 percent from a year earlier.",
    "The meeting is at 10:30 on Tuesday 14 October 2026, in room 2B.",
    "Prices have roughly doubled since the 1990s, and the average home now costs £285,000.",
    "In 1666 the Great Fire of London destroyed more than 13,000 houses.",
    "Version 2.0.1 fixes 37 bugs and adds support for 3rd-party plugins.",
    // Conversational
    "I can't believe it's already October; we haven't even booked the hotel.",
];

struct Args {
    model: PathBuf,
    voice: PathBuf,
    out_dir: PathBuf,
    threads: usize,
    runs: usize,
    corpus: bool,
    raw: bool,
    play: bool,
    spin: bool,
    prefer_cpu: bool,
}

fn parse_args() -> Result<Args> {
    let mut positional = Vec::new();
    let mut args = Args {
        model: PathBuf::new(),
        voice: PathBuf::new(),
        out_dir: PathBuf::new(),
        threads: 0,
        runs: 5,
        corpus: false,
        raw: false,
        play: false,
        spin: true,
        prefer_cpu: true,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--threads" => args.threads = it.next().ok_or("--threads needs a value")?.parse()?,
            "--runs" => args.runs = it.next().ok_or("--runs needs a value")?.parse()?,
            "--corpus" => args.corpus = true,
            "--raw" => args.raw = true,
            "--play" => args.play = true,
            "--no-spin" => args.spin = false,
            "--default-policy" => args.prefer_cpu = false,
            _ if arg.starts_with("--") => return Err(format!("unknown flag {arg}").into()),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let [model, voice, out_dir]: [PathBuf; 3] = positional
        .try_into()
        .map_err(|_| "usage: kokoro_spike <model.onnx> <voice.bin> <out_dir> [flags]")?;
    args.model = model;
    args.voice = voice;
    args.out_dir = out_dir;
    args.runs = args.runs.max(1);
    Ok(args)
}

fn main() -> Result<()> {
    let args = parse_args()?;
    fs::create_dir_all(&args.out_dir)?;
    let british = voice_is_british(&args.voice);
    let lang = if british {
        Language::EnglishGB
    } else {
        Language::EnglishUS
    };
    println!("rss at start: {}", rss());

    let t = Instant::now();
    let g2p = G2P::new(lang);
    println!("G2P::new({lang:?}): {} ms, rss {}", ms(t.elapsed()), rss());

    let voice = Voice::load(&args.voice)?;
    let env = ort::init()
        .with_name("kokoro-spike")
        .with_telemetry(false)
        .build()?;
    let t = Instant::now();
    let mut session = load_session(&env, &args)?;
    println!(
        "session load {}: {} ms, rss {}",
        args.model.display(),
        ms(t.elapsed()),
        rss()
    );
    for input in session.inputs() {
        println!("  input  {} {}", input.name(), input.dtype());
    }
    for output in session.outputs() {
        println!("  output {} {}", output.name(), output.dtype());
    }

    let mut first_run = true;
    let mut played = !args.play;
    println!("\n| words | tokens | g2p | first run | warm median | audio | rtf | same output |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for text in BENCH {
        let t = Instant::now();
        let phonemes = phonemize(&g2p, text, british, args.raw)?;
        let g2p_time = t.elapsed();
        let (ids, _) = token_ids(&phonemes);
        let style = voice.style(ids.len());

        let mut times = Vec::with_capacity(args.runs);
        let mut audio = Vec::new();
        let mut first = Duration::ZERO;
        let mut deterministic = true;
        for run in 0..=args.runs {
            let t = Instant::now();
            let out = synthesize(&mut session, &ids, style, 1.0)?;
            let elapsed = t.elapsed();
            if run == 0 {
                first = elapsed;
            } else {
                times.push(elapsed);
                deterministic &= out == audio;
            }
            audio = out;
        }
        times.sort();
        let warm = times[times.len() / 2];
        let seconds = audio.len() as f64 / f64::from(SAMPLE_RATE);
        let words = text.split_whitespace().count();
        println!(
            "| {words} | {} | {:.2} ms | {} ms{} | {} ms | {seconds:.2} s | {:.2} | {deterministic} |",
            ids.len(),
            g2p_time.as_secs_f64() * 1000.0,
            ms(first),
            if first_run { " (after load)" } else { "" },
            ms(warm),
            warm.as_secs_f64() / seconds,
        );
        first_run = false;

        let stem = args.out_dir.join(format!("bench_{words}w"));
        write_wav(&stem.with_extension("wav"), &audio)?;
        write_f32(&stem.with_extension("f32"), &audio)?;
        let id_list: Vec<String> = ids.iter().map(i64::to_string).collect();
        fs::write(
            stem.with_extension("txt"),
            format!("{text}\n{phonemes}\n{}\n", id_list.join(" ")),
        )?;
        if !played && words == 14 {
            played = true;
            if let Err(e) = play(audio.clone()) {
                println!("playback failed: {e}");
            }
        }
    }

    if args.corpus {
        run_corpus(&g2p, lang, british, &args, &mut session, &voice)?;
    }
    Ok(())
}

/// `b*` voices are British English, everything else American.
fn voice_is_british(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('b'))
}

fn load_session(env: &Environment, args: &Args) -> ort::Result<Session> {
    let mut builder = Session::builder(env)?;
    // rc.14 defaults to MaxEfficiency: CoreML on macOS, ~3 s loads and a different waveform.
    if args.prefer_cpu {
        builder = builder.with_auto_device(AutoDevicePolicy::PreferCPU)?;
    }
    if args.threads > 0 {
        builder = builder.with_intra_threads(args.threads)?;
    }
    if !args.spin {
        builder = builder.with_intra_op_spinning(false)?;
    }
    builder.commit_from_file(&args.model)
}

fn synthesize(
    session: &mut Session,
    ids: &[i64],
    style: &[f32],
    speed: f32,
) -> ort::Result<Vec<f32>> {
    let mut input = Vec::with_capacity(ids.len() + 2);
    input.push(0);
    input.extend_from_slice(ids);
    input.push(0);
    let len = input.len();
    let outputs = session.run(ort::inputs![
        "input_ids" => Tensor::from_array(([1usize, len], input))?,
        "style" => TensorRef::from_array_view(([1usize, STYLE_DIM], style))?,
        "speed" => Tensor::from_array(([1usize], vec![speed]))?,
    ])?;
    let (_, audio) = outputs["waveform"].try_extract_tensor::<f32>()?;
    Ok(audio.to_vec())
}

/// Text to Kokoro phonemes. `raw` skips the tie mapping so the effect can be heard.
fn phonemize(g2p: &G2P, text: &str, british: bool, raw: bool) -> Result<String> {
    let (phonemes, _) = g2p.g2p(text)?;
    let mapped = if raw {
        phonemes
    } else {
        let ties = if british { TIES_GB } else { TIES_US };
        TIES_COMMON
            .iter()
            .chain(ties)
            .fold(phonemes, |s, (from, to)| s.replace(from, to))
            // Triphthongs such as `a‍ɪ‍ə` keep their second tie.
            .replace('\u{200d}', "")
    };
    Ok(tidy_spacing(&mapped))
}

/// misaki-rs ends every token with a space, punctuation included; Kokoro was
/// trained with punctuation attached to the word before it.
fn tidy_spacing(phonemes: &str) -> String {
    let mut out = String::with_capacity(phonemes.len());
    for c in phonemes.chars() {
        match c {
            ' ' if out.is_empty() || out.ends_with(' ') => {}
            ';' | ':' | ',' | '.' | '!' | '?' => {
                if out.ends_with(' ') {
                    out.pop();
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.trim_end().to_string()
}

/// Ids for every symbol Kokoro knows, plus the symbols it had to drop.
fn token_ids(phonemes: &str) -> (Vec<i64>, Vec<char>) {
    let mut ids = Vec::new();
    let mut dropped = Vec::new();
    for c in phonemes.chars() {
        match VOCAB.iter().find(|(v, _)| *v == c) {
            Some((_, id)) => ids.push(*id),
            None => dropped.push(c),
        }
    }
    ids.truncate(MAX_TOKENS);
    (ids, dropped)
}

/// A raw little-endian f32 voice file: one 256-wide style row per input length.
struct Voice {
    rows: Vec<f32>,
}

impl Voice {
    fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path)?;
        if bytes.is_empty() || bytes.len() % (4 * STYLE_DIM) != 0 {
            return Err(format!("{} is not a Kokoro voice file", path.display()).into());
        }
        let rows = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        Ok(Self { rows })
    }

    /// The row index is the token count without the pads.
    fn style(&self, tokens: usize) -> &[f32] {
        let row = tokens.min(self.rows.len() / STYLE_DIM - 1);
        &self.rows[row * STYLE_DIM..(row + 1) * STYLE_DIM]
    }
}

/// Records each word misaki-rs cannot find, for the OOV count.
struct OovRecorder(Arc<Mutex<Vec<String>>>);

impl Fallback for OovRecorder {
    fn phonemize(&self, word: &str) -> std::result::Result<String, FallbackError> {
        if let Ok(mut words) = self.0.lock() {
            words.push(word.to_string());
        }
        Ok(String::new())
    }
}

fn run_corpus(
    g2p: &G2P,
    lang: Language,
    british: bool,
    args: &Args,
    session: &mut Session,
    voice: &Voice,
) -> Result<()> {
    let oov = Arc::new(Mutex::new(Vec::new()));
    let probe = G2P::with_fallback(lang, Some(Box::new(OovRecorder(oov.clone()))));

    let mut report = String::new();
    let mut words = 0;
    let mut unknown = 0;
    let mut dropped_all = Vec::new();
    let mut oov_all: Vec<String> = Vec::new();
    for (i, text) in CORPUS.iter().enumerate() {
        oov.lock().map_err(|_| "poisoned")?.clear();
        let (_, tokens) = probe.g2p(text)?;
        words += tokens
            .iter()
            .filter(|t| t.text.chars().any(char::is_alphabetic))
            .count();
        let misses = oov.lock().map_err(|_| "poisoned")?.clone();

        let (raw, _) = g2p.g2p(text)?;
        unknown += raw.matches('❓').count();
        let phonemes = phonemize(g2p, text, british, args.raw)?;
        let (ids, dropped) = token_ids(&phonemes);
        dropped_all.extend(dropped.iter().copied());

        let audio = synthesize(session, &ids, voice.style(ids.len()), 1.0)?;
        write_wav(
            &args.out_dir.join(format!("corpus_{:02}.wav", i + 1)),
            &audio,
        )?;
        writeln!(
            report,
            "{:02} {text}\n   {phonemes}\n   oov {misses:?}, dropped {dropped:?}, {} tokens",
            i + 1,
            ids.len()
        )?;
        oov_all.extend(misses);
    }
    let numeric = oov_all
        .iter()
        .filter(|w| w.starts_with(|c: char| c.is_ascii_digit()))
        .count();
    let oov_hits = oov_all.len();
    oov_all.sort();
    oov_all.dedup();

    writeln!(report, "\nOOV words, spelled letter by letter:")?;
    for word in &oov_all {
        writeln!(
            report,
            "   {word}: {}",
            phonemize(g2p, word, british, args.raw)?
        )?;
    }
    let spelled = phonemize(g2p, &oov_all.join(", "), british, args.raw)?;
    let (ids, _) = token_ids(&spelled);
    let audio = synthesize(session, &ids, voice.style(ids.len()), 1.0)?;
    write_wav(&args.out_dir.join("oov_words.wav"), &audio)?;

    writeln!(
        report,
        "\ncorpus: {} sentences, {words} words, {oov_hits} OOV ({} words, {numeric} numbers), \
         {} distinct, ❓ {unknown}, symbols outside the vocab after mapping {dropped_all:?}",
        CORPUS.len(),
        oov_hits - numeric,
        oov_all.len(),
    )?;
    println!("\n{report}");
    fs::write(args.out_dir.join("corpus.txt"), report)?;
    Ok(())
}

fn play(samples: Vec<f32>) -> Result<()> {
    let mut sink = rodio::DeviceSinkBuilder::from_default_device()?
        .with_buffer_size(rodio::cpal::BufferSize::Default)
        .open_stream()?;
    sink.log_on_drop(false);
    let player = rodio::Player::connect_new(sink.mixer());
    let rate = NonZero::new(SAMPLE_RATE).ok_or("zero sample rate")?;
    player.append(rodio::buffer::SamplesBuffer::new(
        NonZero::<u16>::MIN,
        rate,
        samples,
    ));
    player.sleep_until_end();
    Ok(())
}

fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    let data_len = u32::try_from(samples.len() * 2)?;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    fs::write(path, out)?;
    Ok(())
}

/// Raw samples, for comparing against Python onnxruntime on the same ids.
fn write_f32(path: &Path, samples: &[f32]) -> Result<()> {
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    fs::write(path, bytes)?;
    Ok(())
}

fn ms(d: Duration) -> u128 {
    d.as_millis()
}

/// Resident set size via `ps`; good enough for a spike.
fn rss() -> String {
    Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map_or_else(|| "?".to_string(), |kb| format!("{} MB", kb / 1024))
}
