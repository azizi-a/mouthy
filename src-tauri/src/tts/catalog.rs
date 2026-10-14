//! The Kokoro v1.0 files Mouthy can download: onnx-community's ONNX export of
//! Kokoro-82M (Apache-2.0), pinned to one revision so the sizes and sha256
//! below (the repo's LFS metadata) always describe what the URL serves.

use super::local;
use crate::settings::ModelVariant;
use serde::Serialize;
use specta::Type;
use std::path::{Path, PathBuf};

const REVISION_URL: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/1939ad2a8e416c0acfeecc08a694d14ef25f2231";

/// Id, variant, path in the repo, size, sha256.
const MODELS: &[(&str, ModelVariant, &str, u64, &str)] = &[
    (
        "int8",
        ModelVariant::Int8,
        "onnx/model_quantized.onnx",
        92_361_116,
        "fbae9257e1e05ffc727e951ef9b9c98418e6d79f1c9b6b13bd59f5c9028a1478",
    ),
    (
        "fp32",
        ModelVariant::Fp32,
        "onnx/model.onnx",
        325_532_232,
        "8fbea51ea711f2af382e88c833d9e288c6dc82ce5e98421ea61c058ce21a34cb",
    ),
];

/// Every voice file is 510 style rows of 256 f32.
const VOICE_SIZE: u64 = 522_240;

/// The v1.0 voices for the two dialects misaki-rs reads: British (`b*`) and
/// American (`a*`). The repo's other voices expect other languages' phonemes.
#[rustfmt::skip]
const VOICES: &[(&str, &str)] = &[
    ("bf_alice", "08afa6ba24da61ea5e8efa139e5aadc938d83f0a6da5a900adaf763ac1da5573"),
    ("bf_emma", "669fe0647f9dd04fcab92f1439a40eeb4c8b4ab1f82e4996fe3d918ce4a63b73"),
    ("bf_isabella", "3754352c4aaa46d17f27654ab7518d65b62ad6163a0f55a5f4330c2da2c4e94f"),
    ("bf_lily", "5e0ee32ebe64a467124976b14e69590746f1c4ce41a12b587a50c862edfea335"),
    ("bm_daniel", "6b3194bbceffb746733cbc22c8f593dd44e401a71d53895a2dca891bc595a1e8"),
    ("bm_fable", "f889083196807b4adb15e9204252165f503b8d33d3982e681c52443c49d798f1"),
    ("bm_george", "c4b235a4c1f2cd3b939fed08b899ce9385638b763f7b73a59616c4fc9bd6c9bc"),
    ("bm_lewis", "b8f671cef828c30e66fdf0b0756a76bba58f6bb3398cbbf27058642acbcedb97"),
    ("af_alloy", "c4a6b876047fd7fb472edf4ebd63cfac7c3b958a7cae7c106e8f038ca6308c45"),
    ("af_aoede", "4a004c33430762e2461eedb2013fad808ef4ab3121f5300f554476caf58d8361"),
    ("af_bella", "f69d836209b78eb8c66e75e3cda491e26ea838a3674257e9d4e5703cbaf55c8b"),
    ("af_heart", "d583ccff3cdca2f7fae535cb998ac07e9fcb90f09737b9a41fa2734ec44a8f0b"),
    ("af_jessica", "a240a5e3c15b43563d6e923bdca8ef5613a23471d9b77653694012435df23bd8"),
    ("af_kore", "9be5221b6a941c04b561959b8ff0b06e809444dcc4ab7e75a7b23606f691819e"),
    ("af_nicole", "cd2191ab31b914ed7b318416b0e4440fdf392ddad9106a060819aa600a64f59a"),
    ("af_nova", "18778272caa0d0eebaea251c35fd635f038434f9eee5e691d02a174bd328414f"),
    ("af_river", "00a2bcf82b1d86e8f19902ede58c65ccf6c0e43b44b7d74fad54e5d8933c9c30"),
    ("af_sarah", "4409fbc125afabacc615d94db5398d847006a737b0247d6892b7a9a0007a2f0a"),
    ("af_sky", "4435255c9744f3f31659e0d714ab7689bf65d9e77ec1cce060f083912614f0b9"),
    ("am_adam", "162b035ed91cfc48b6046982184c645f72edcdd1b82843347f605d7bf7b15716"),
    ("am_echo", "3968b92c3c4cd1c4416dbded36c13eaa388a90d5788d02a13e4d781f5f8cf3c3"),
    ("am_eric", "e8b5be17edd1e3636901ce7598baafe2dc8dd8ff707a0c23bf9e461add7e2832"),
    ("am_fenrir", "c27989f741f7ee34d273a39d8a595cc0837d35f5ced9a29b7cc162614616df43"),
    ("am_liam", "52403be32fd047c6a44517cb0bcd6b134f2a18baa73e70ef41651e0eab921ade"),
    ("am_michael", "1d1f21dd8da39c30705cd4c75d039d265e9bc4a2a93ed09bc9e1b1225eb95ba1"),
    ("am_onyx", "da5d135b424164916d75a68ffb4c2abce3d7d5ccc82dd1ee6cf447ce286145e6"),
    ("am_puck", "fcf73c989033e9233e0b98713eca600c8c74dcc1614b37009d5450ff4a2274a0"),
    ("am_santa", "61150cf726ab6c5ed7a99f90a304f91f5a72c00c592e89ec94e5df11c319227a"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ModelFileKind {
    Model,
    Voice,
}

/// One downloadable file: where it comes from, how to check it, where it goes.
#[derive(Clone, Debug)]
pub struct CatalogFile {
    /// A model variant (`int8`) or a voice (`bm_lewis`).
    pub id: String,
    pub kind: ModelFileKind,
    pub url: String,
    pub size: u64,
    pub sha256: &'static str,
    pub path: PathBuf,
}

/// Every file, models first.
pub fn all(models_dir: &Path) -> Vec<CatalogFile> {
    let models = MODELS
        .iter()
        .map(|&(id, variant, remote, size, sha256)| CatalogFile {
            id: id.to_string(),
            kind: ModelFileKind::Model,
            url: format!("{REVISION_URL}/{remote}"),
            size,
            sha256,
            path: local::model_path(models_dir, variant),
        });
    let voices = VOICES.iter().filter_map(|&(id, sha256)| {
        Some(CatalogFile {
            id: id.to_string(),
            kind: ModelFileKind::Voice,
            url: format!("{REVISION_URL}/voices/{id}.bin"),
            size: VOICE_SIZE,
            sha256,
            path: local::voice_path(models_dir, id)?,
        })
    });
    models.chain(voices).collect()
}

pub fn find(models_dir: &Path, id: &str) -> Option<CatalogFile> {
    all(models_dir).into_iter().find(|file| file.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_voice_is_listed_and_ids_are_unique() {
        let files = all(Path::new("/models"));
        assert_eq!(files.len(), MODELS.len() + VOICES.len());
        let ids: HashSet<_> = files.iter().map(|file| file.id.as_str()).collect();
        assert_eq!(ids.len(), files.len());
        for id in [
            "bm_lewis",
            "bf_emma",
            "af_heart",
            "af_bella",
            "am_adam",
            "am_michael",
        ] {
            assert!(ids.contains(id), "{id}");
        }
    }

    #[test]
    fn files_land_where_the_local_backend_reads_them() {
        let models = Path::new("/models");
        for (variant, path) in [
            (ModelVariant::Int8, "/models/kokoro-v1.0/int8.onnx"),
            (ModelVariant::Fp32, "/models/kokoro-v1.0/fp32.onnx"),
        ] {
            let id = MODELS.iter().find(|m| m.1 == variant).unwrap().0;
            assert_eq!(find(models, id).unwrap().path, Path::new(path));
        }
        let voice = find(models, "bm_lewis").unwrap();
        assert_eq!(
            voice.path,
            Path::new("/models/kokoro-v1.0/voices/bm_lewis.bin")
        );
        assert_eq!(voice.kind, ModelFileKind::Voice);
        assert!(find(models, "../bm_lewis").is_none());
    }

    #[test]
    fn urls_are_pinned_and_checksums_are_sha256() {
        for file in all(Path::new("/models")) {
            assert!(file.url.starts_with(REVISION_URL), "{}", file.url);
            assert_eq!(file.sha256.len(), 64, "{}", file.id);
            assert!(file
                .sha256
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        }
        assert!(find(Path::new("/m"), "int8")
            .unwrap()
            .url
            .ends_with("/onnx/model_quantized.onnx"));
        assert!(find(Path::new("/m"), "bm_lewis")
            .unwrap()
            .url
            .ends_with("/voices/bm_lewis.bin"));
    }

    #[test]
    fn voice_size_is_whole_style_rows() {
        assert_eq!(VOICE_SIZE % (4 * 256), 0);
        assert_eq!(VOICE_SIZE / (4 * 256), 510);
    }
}
