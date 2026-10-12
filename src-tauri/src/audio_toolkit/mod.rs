pub mod audio;
pub mod lang_id;
pub mod text;
pub mod utils;

pub use audio::read_wav_samples;
pub use lang_id::detect_output_language;
pub use text::{
    apply_custom_words, normalize_transcription_output, remove_filler_words, OutputLanguageEvidence,
};
pub use utils::get_cpal_host;
