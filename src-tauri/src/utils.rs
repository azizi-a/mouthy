// Re-export all utility modules for easy access
pub use crate::overlay::*;
pub use crate::tray::*;

/// Returns true when the environment variable is set to a truthy value
/// (e.g. "1", "true", "yes", "on").
/// "0", "false", "no", "off" and empty string are treated as falsy (case-insensitive).
/// Returns false when the variable is not set.
#[cfg(any(target_os = "linux", test))]
pub fn env_flag_enabled(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        ),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_flag_enabled_true_for_truthy_values() {
        for value in ["1", "true", "TRUE", "yes", "on", " 1 "] {
            std::env::set_var("MOUTHY_TEST_FLAG_TRUTHY", value);
            assert!(env_flag_enabled("MOUTHY_TEST_FLAG_TRUTHY"), "{value:?}");
        }
        std::env::remove_var("MOUTHY_TEST_FLAG_TRUTHY");
    }

    #[test]
    fn env_flag_enabled_false_for_falsy_or_unset() {
        assert!(!env_flag_enabled("MOUTHY_TEST_FLAG_UNSET"));

        for value in ["0", "false", "FALSE", "no", "off", ""] {
            std::env::set_var("MOUTHY_TEST_FLAG_FALSY", value);
            assert!(!env_flag_enabled("MOUTHY_TEST_FLAG_FALSY"), "{value:?}");
        }
        std::env::remove_var("MOUTHY_TEST_FLAG_FALSY");
    }
}
