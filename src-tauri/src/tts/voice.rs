//! A voice setting: one Kokoro voice or a blend in Kokoro-FastAPI's syntax,
//! `name` or `name(weight)` items joined by `+`, such as
//! `bm_lewis(70)+bm_george(30)`. A weight defaults to 1; the blend divides
//! each by their sum.

use std::fmt;

/// The most voices one blend mixes.
const MAX_PARTS: usize = 4;

/// A parsed voice setting: at least one part, every name a possible file name,
/// every weight finite and above zero.
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceSpec {
    parts: Vec<(String, f32)>,
}

impl VoiceSpec {
    /// Reads a voice setting, or says in a sentence for the user what is wrong.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err("Type a voice name, such as af_alloy".into());
        }
        // Before the name check, which would take `a-b` for one name.
        if spec.contains('-') {
            return Err(
                "Voice names cannot contain '-': blend voices with + (subtracting one is not supported)"
                    .into(),
            );
        }
        let items: Vec<&str> = spec.split('+').collect();
        if items.len() > MAX_PARTS {
            return Err(format!("A blend can mix up to {MAX_PARTS} voices"));
        }
        let parts = items
            .into_iter()
            .map(parse_part)
            .collect::<Result<_, _>>()?;
        Ok(Self { parts })
    }

    /// Each voice with its weight as written (1 when none was).
    pub fn parts(&self) -> &[(String, f32)] {
        &self.parts
    }

    /// The voice whose dialect a blend takes, as Kokoro-FastAPI does.
    pub fn first(&self) -> &str {
        &self.parts[0].0
    }

    /// Each voice with its share of the blend, from 0 to 1.
    pub fn shares(&self) -> impl Iterator<Item = (&str, f32)> {
        let total: f64 = self.parts.iter().map(|&(_, w)| f64::from(w)).sum();
        self.parts
            .iter()
            .map(move |(name, w)| (name.as_str(), (f64::from(*w) / total) as f32))
    }
}

/// The canonical form: no spaces, a weight only where it is not 1.
impl fmt::Display for VoiceSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (name, weight)) in self.parts.iter().enumerate() {
            if i > 0 {
                f.write_str("+")?;
            }
            f.write_str(name)?;
            if *weight != 1.0 {
                write!(f, "({weight})")?;
            }
        }
        Ok(())
    }
}

fn parse_part(item: &str) -> Result<(String, f32), String> {
    let item = item.trim();
    if item.is_empty() {
        return Err("Each + needs a voice on both sides".into());
    }
    let (name, weight) = match item.split_once('(') {
        Some((name, rest)) => {
            let weight = rest
                .strip_suffix(')')
                .and_then(|w| w.trim().parse::<f32>().ok())
                .filter(|w| w.is_finite() && *w > 0.0)
                .ok_or_else(|| format!("'{item}' needs a weight above 0, such as bm_lewis(70)"))?;
            (name.trim(), weight)
        }
        None => (item, 1.0),
    };
    if name.is_empty() {
        return Err(format!("'{item}' has no voice name"));
    }
    if !is_voice_name(name) {
        return Err(format!("'{name}' is not a voice name"));
    }
    Ok((name.to_string(), weight))
}

/// Voice names are typed by the user and become file names, so they are
/// limited to characters that keep them inside the voices folder.
pub(crate) fn is_voice_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(spec: &str) -> Vec<(String, f32)> {
        VoiceSpec::parse(spec).unwrap().parts().to_vec()
    }

    fn part(name: &str, weight: f32) -> (String, f32) {
        (name.to_string(), weight)
    }

    #[test]
    fn a_single_voice_has_weight_one() {
        assert_eq!(parts("bm_lewis"), vec![part("bm_lewis", 1.0)]);
        assert_eq!(VoiceSpec::parse("bm_lewis").unwrap().first(), "bm_lewis");
    }

    #[test]
    fn a_blend_without_weights_weighs_each_voice_one() {
        assert_eq!(
            parts("bm_lewis+bm_george"),
            vec![part("bm_lewis", 1.0), part("bm_george", 1.0)]
        );
    }

    #[test]
    fn weights_are_read_from_brackets() {
        assert_eq!(
            parts("bm_lewis(2)+bm_george(1)"),
            vec![part("bm_lewis", 2.0), part("bm_george", 1.0)]
        );
        assert_eq!(parts("af_bella(0.25)"), vec![part("af_bella", 0.25)]);
    }

    #[test]
    fn spaces_around_names_weights_and_pluses_are_ignored() {
        assert_eq!(
            parts("  bm_lewis ( 70 ) +  bm_george(30)  "),
            vec![part("bm_lewis", 70.0), part("bm_george", 30.0)]
        );
    }

    #[test]
    fn a_weight_must_be_a_number_above_zero() {
        for spec in [
            "bm_lewis(0)",
            "bm_lewis()",
            "bm_lewis(x)",
            "bm_lewis(nan)",
            "bm_lewis(inf)",
            "bm_lewis(70",
            "bm_lewis(70)x",
            "bm_lewis((70))",
            "bm_lewis+bm_george(0.0)",
        ] {
            let error = VoiceSpec::parse(spec).unwrap_err();
            assert!(error.contains("weight"), "{spec}: {error}");
        }
    }

    #[test]
    fn subtraction_is_rejected_before_names_are_checked() {
        for spec in ["bm_lewis-bm_george", "bm_lewis+af_sky-af_bella", "my-voice"] {
            let error = VoiceSpec::parse(spec).unwrap_err();
            assert!(error.contains("'-'"), "{spec}: {error}");
        }
        assert!(VoiceSpec::parse("bm_lewis(-1)")
            .unwrap_err()
            .contains("'-'"));
    }

    #[test]
    fn every_plus_needs_a_voice_on_both_sides() {
        for spec in ["+", "bm_lewis+", "+bm_lewis", "bm_lewis++bm_george", " + "] {
            assert!(VoiceSpec::parse(spec).is_err(), "{spec}");
        }
        assert!(VoiceSpec::parse("").is_err());
        assert!(VoiceSpec::parse("   ").is_err());
        assert!(VoiceSpec::parse("(70)")
            .unwrap_err()
            .contains("no voice name"));
    }

    #[test]
    fn names_cannot_leave_the_voices_folder() {
        for spec in ["../secret", "a/b", "voice.bin", "bm lewis", "bm_lewis+../x"] {
            let error = VoiceSpec::parse(spec).unwrap_err();
            assert!(error.contains("not a voice name"), "{spec}: {error}");
        }
    }

    #[test]
    fn a_blend_mixes_at_most_four_voices() {
        assert_eq!(parts("af_a+af_b+af_c+af_d").len(), 4);
        let error = VoiceSpec::parse("af_a+af_b+af_c+af_d+af_e").unwrap_err();
        assert!(error.contains('4'), "{error}");
    }

    #[test]
    fn display_writes_the_canonical_form() {
        for (spec, canonical) in [
            ("bm_lewis", "bm_lewis"),
            (
                " bm_lewis ( 70 ) + bm_george (30) ",
                "bm_lewis(70)+bm_george(30)",
            ),
            ("bm_lewis(1)+bm_george(1.0)", "bm_lewis+bm_george"),
            ("af_bella(2)+af_sky", "af_bella(2)+af_sky"),
            ("af_bella(0.25)+af_sky(1e2)", "af_bella(0.25)+af_sky(100)"),
        ] {
            let parsed = VoiceSpec::parse(spec).unwrap();
            assert_eq!(parsed.to_string(), canonical, "{spec}");
            assert_eq!(VoiceSpec::parse(canonical).unwrap(), parsed, "{spec}");
        }
    }

    #[test]
    fn shares_divide_each_weight_by_the_sum() {
        let spec = VoiceSpec::parse("bm_lewis(70)+bm_george(30)").unwrap();
        let shares: Vec<_> = spec.shares().collect();
        assert_eq!(shares, vec![("bm_lewis", 0.7), ("bm_george", 0.3)]);
        let spec = VoiceSpec::parse("bm_lewis(5)").unwrap();
        assert_eq!(spec.shares().collect::<Vec<_>>(), vec![("bm_lewis", 1.0)]);
    }
}
