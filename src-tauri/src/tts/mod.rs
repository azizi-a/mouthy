//! Speech synthesis: text preparation and the two backends.

pub mod catalog;
pub mod download;
pub mod espeak;
pub mod http;
pub mod local;
pub mod text;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Both backends produce mono audio at Kokoro's native rate.
pub const SAMPLE_RATE: u32 = 24_000;

/// Where a speech session's audio comes from, decided from the settings when
/// it starts.
pub enum Backend {
    Http { base_url: String, voice: String },
    Local(local::ModelFiles),
}

impl Backend {
    /// Readies the backend on the synth thread. `stop` cuts a local inference
    /// short.
    pub fn open(self, stop: &Arc<AtomicBool>) -> Result<Synth, String> {
        match self {
            Backend::Http { base_url, voice } => Ok(Synth::Http {
                client: http::client(),
                base_url,
                voice,
            }),
            Backend::Local(files) => local::open(&files, stop).map(Synth::Local),
        }
    }
}

/// A backend ready to synthesise, owned by the synth thread for one session.
pub enum Synth {
    Http {
        client: reqwest::Client,
        base_url: String,
        voice: String,
    },
    Local(local::Loaded),
}
