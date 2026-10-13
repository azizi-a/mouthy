use log::{debug, warn};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;
use std::collections::HashMap;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

// Custom deserializer to handle both old numeric format (1-5) and new string format ("trace", "debug", etc.)
impl<'de> Deserialize<'de> for LogLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LogLevelVisitor;

        impl<'de> Visitor<'de> for LogLevelVisitor {
            type Value = LogLevel;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string or integer representing log level")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<LogLevel, E> {
                match value.to_lowercase().as_str() {
                    "trace" => Ok(LogLevel::Trace),
                    "debug" => Ok(LogLevel::Debug),
                    "info" => Ok(LogLevel::Info),
                    "warn" => Ok(LogLevel::Warn),
                    "error" => Ok(LogLevel::Error),
                    _ => Err(E::unknown_variant(
                        value,
                        &["trace", "debug", "info", "warn", "error"],
                    )),
                }
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<LogLevel, E> {
                match value {
                    1 => Ok(LogLevel::Trace),
                    2 => Ok(LogLevel::Debug),
                    3 => Ok(LogLevel::Info),
                    4 => Ok(LogLevel::Warn),
                    5 => Ok(LogLevel::Error),
                    _ => Err(E::invalid_value(de::Unexpected::Unsigned(value), &"1-5")),
                }
            }
        }

        deserializer.deserialize_any(LogLevelVisitor)
    }
}

impl From<LogLevel> for tauri_plugin_log::LogLevel {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => tauri_plugin_log::LogLevel::Trace,
            LogLevel::Debug => tauri_plugin_log::LogLevel::Debug,
            LogLevel::Info => tauri_plugin_log::LogLevel::Info,
            LogLevel::Warn => tauri_plugin_log::LogLevel::Warn,
            LogLevel::Error => tauri_plugin_log::LogLevel::Error,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ShortcutBinding {
    pub id: String,
    pub name: String,
    pub description: String,
    pub default_binding: String,
    pub current_binding: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition {
    Top,
    // `none` is retired: overlay visibility is owned by `OverlayStyle` now. The
    // alias keeps legacy stores (`"overlay_position": "none"`) deserializing
    // instead of failing the whole load.
    #[serde(alias = "none")]
    Bottom,
}

/// Which overlay to display: `Minimal` is the compact pill, `None` hides it.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum OverlayStyle {
    None,
    // `live` (a streaming-text panel) is retired; the alias keeps stores that
    // still hold it deserializing instead of failing the whole load.
    #[serde(alias = "live")]
    Minimal,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardImplementation {
    Tauri,
    HandyKeys,
}

impl Default for KeyboardImplementation {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        return KeyboardImplementation::Tauri;
        #[cfg(not(target_os = "linux"))]
        return KeyboardImplementation::HandyKeys;
    }
}

/// UI appearance mode. `System` follows the OS `prefers-color-scheme`; `Light`
/// and `Dark` force one of the two palettes the app ships.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}

/// Where speech is synthesised: an OpenAI-compatible server at
/// `tts_base_url`, or Kokoro in-process from the files in the models folder.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum TtsBackend {
    Http,
    Local,
}

/// Which Kokoro export the local backend loads. int8 is about twice as fast
/// as fp32 on the CPU.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum ModelVariant {
    Int8,
    Fp16,
    Fp32,
}

/// The container-level `serde(default)` (backed by the `Default` impl below)
/// guarantees every field — including ones added in the future — falls back to
/// its `get_default_settings()` value when missing from a stored settings
/// object, so a partial store can never fail the whole load (#1619).
/// Field-level defaults below take precedence where present.
#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(default)]
pub struct AppSettings {
    /// Defaults to empty on partial stores; the load path syncs the ids with
    /// the default bindings (see `sync_bindings_with_defaults`) before the
    /// settings are used.
    #[serde(default)]
    pub bindings: HashMap<String, ShortcutBinding>,
    #[serde(default = "default_start_hidden")]
    pub start_hidden: bool,
    #[serde(default = "default_autostart_enabled")]
    pub autostart_enabled: bool,
    #[serde(default)]
    pub onboarding_completed: bool,
    #[serde(default)]
    pub selected_output_device: Option<String>,
    #[serde(default = "default_overlay_position")]
    pub overlay_position: OverlayPosition,
    #[serde(default = "default_debug_mode")]
    pub debug_mode: bool,
    #[serde(default = "default_log_level")]
    pub log_level: LogLevel,
    #[serde(default = "default_app_language")]
    pub app_language: String,
    #[serde(default = "default_theme")]
    pub theme: Theme,
    #[serde(default)]
    pub experimental_enabled: bool,
    #[serde(default)]
    pub keyboard_implementation: KeyboardImplementation,
    #[serde(default = "default_show_tray_icon")]
    pub show_tray_icon: bool,
    #[serde(default = "default_overlay_style")]
    pub overlay_style: OverlayStyle,
    #[serde(default = "default_tts_base_url")]
    pub tts_base_url: String,
    #[serde(default = "default_tts_voice")]
    pub tts_voice: String,
    /// Sent to the server; changing playback rate locally would shift pitch.
    #[serde(default = "default_tts_speed")]
    pub tts_speed: f32,
    #[serde(default = "default_tts_volume")]
    pub tts_volume: f32,
    #[serde(default = "default_tts_backend")]
    pub tts_backend: TtsBackend,
    #[serde(default = "default_tts_model_variant")]
    pub tts_model_variant: ModelVariant,
}

fn default_start_hidden() -> bool {
    false
}

fn default_autostart_enabled() -> bool {
    false
}

fn default_overlay_position() -> OverlayPosition {
    // Position only matters when the overlay is shown; whether it shows at all is
    // `overlay_style` (Linux defaults that to None). So a single default suffices.
    OverlayPosition::Bottom
}

fn default_overlay_style() -> OverlayStyle {
    // Linux hides the overlay by default; other platforms show the pill.
    // Position is independent and only selects top vs. bottom placement.
    #[cfg(target_os = "linux")]
    return OverlayStyle::None;
    #[cfg(not(target_os = "linux"))]
    return OverlayStyle::Minimal;
}

fn default_debug_mode() -> bool {
    false
}

fn default_log_level() -> LogLevel {
    LogLevel::Debug
}

fn default_theme() -> Theme {
    Theme::System
}

fn default_app_language() -> String {
    tauri_plugin_os::locale()
        .map(|l| l.replace('_', "-"))
        .unwrap_or_else(|| "en".to_string())
}

fn default_show_tray_icon() -> bool {
    true
}

fn default_tts_base_url() -> String {
    "http://127.0.0.1:8880".to_string()
}

fn default_tts_voice() -> String {
    "bm_lewis".to_string()
}

fn default_tts_speed() -> f32 {
    1.0
}

fn default_tts_volume() -> f32 {
    1.0
}

fn default_tts_backend() -> TtsBackend {
    TtsBackend::Http
}

fn default_tts_model_variant() -> ModelVariant {
    ModelVariant::Int8
}

pub const SETTINGS_STORE_PATH: &str = "settings_store.json";

pub fn get_default_settings() -> AppSettings {
    // Matches macOS's own "Speak selection" shortcut.
    #[cfg(target_os = "macos")]
    let speak_shortcut = "option+escape";
    #[cfg(not(target_os = "macos"))]
    let speak_shortcut = "alt+escape";

    let mut bindings = HashMap::new();
    bindings.insert(
        "speak".to_string(),
        ShortcutBinding {
            id: "speak".to_string(),
            name: "Speak Selection".to_string(),
            description: "Reads the selected text aloud; press again to stop.".to_string(),
            default_binding: speak_shortcut.to_string(),
            current_binding: speak_shortcut.to_string(),
        },
    );

    AppSettings {
        bindings,
        start_hidden: default_start_hidden(),
        autostart_enabled: default_autostart_enabled(),
        onboarding_completed: false,
        selected_output_device: None,
        overlay_position: default_overlay_position(),
        debug_mode: false,
        log_level: default_log_level(),
        app_language: default_app_language(),
        theme: default_theme(),
        experimental_enabled: false,
        keyboard_implementation: KeyboardImplementation::default(),
        show_tray_icon: default_show_tray_icon(),
        overlay_style: default_overlay_style(),
        tts_base_url: default_tts_base_url(),
        tts_voice: default_tts_voice(),
        tts_speed: default_tts_speed(),
        tts_volume: default_tts_volume(),
        tts_backend: default_tts_backend(),
        tts_model_variant: default_tts_model_variant(),
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        get_default_settings()
    }
}

/// Startup entry point. Same load-or-create/salvage behavior as
/// `get_settings`; kept as a named alias for call-site clarity, plus a
/// one-time debug dump of the loaded settings.
pub fn load_or_create_app_settings(app: &AppHandle) -> AppSettings {
    let settings = get_settings(app);
    debug!("Loaded settings: {:?}", settings);
    settings
}

pub fn get_settings(app: &AppHandle) -> AppSettings {
    let store = app
        .store(SETTINGS_STORE_PATH)
        .expect("Failed to initialize store");

    // Settings reads also persist salvage and binding sync. Both are
    // idempotent, so this converges after the first read of an older store.
    if let Some(settings_value) = store.get("settings") {
        let (mut settings, mut updated) =
            match serde_json::from_value::<AppSettings>(settings_value.clone()) {
                Ok(settings) => (settings, false),
                Err(e) => {
                    warn!("Failed to parse stored settings ({e}); salvaging valid fields");
                    (salvage_settings(&settings_value), true)
                }
            };

        if sync_bindings_with_defaults(&mut settings.bindings) {
            updated = true;
        }

        if updated {
            store.set("settings", serde_json::to_value(&settings).unwrap());
        }

        settings
    } else {
        let default_settings = get_default_settings();
        store.set("settings", serde_json::to_value(&default_settings).unwrap());
        default_settings
    }
}

/// Rebuilds settings from a store value that failed to deserialize as a whole.
/// Every stored field that is individually valid is kept; only broken values
/// (e.g. an enum variant written by a newer or older version) fall back to
/// their default. This means one bad field can never reset the rest of the
/// user's configuration (#1619).
fn salvage_settings(stored: &serde_json::Value) -> AppSettings {
    let Some(stored_map) = stored.as_object() else {
        warn!("Stored settings are not a JSON object; falling back to defaults");
        return get_default_settings();
    };

    let mut merged = serde_json::to_value(get_default_settings())
        .expect("default settings serialize to a JSON object");

    for (key, value) in stored_map {
        let previous = merged
            .as_object_mut()
            .expect("merged settings stay an object")
            .insert(key.clone(), value.clone());
        if serde_json::from_value::<AppSettings>(merged.clone()).is_err() {
            // Log only the key: values may hold secrets (e.g. API keys).
            warn!("Dropping invalid settings field '{key}', keeping its default");
            let map = merged
                .as_object_mut()
                .expect("merged settings stay an object");
            match previous {
                Some(previous) => map.insert(key.clone(), previous),
                None => map.remove(key),
            };
        }
    }

    serde_json::from_value(merged).unwrap_or_else(|e| {
        warn!("Failed to reassemble salvaged settings ({e}); falling back to defaults");
        get_default_settings()
    })
}

/// Adds default bindings missing from a stored map and drops stored ids that
/// are no longer defaults, so retired bindings never register or get shadowed.
/// Returns whether the map changed.
fn sync_bindings_with_defaults(bindings: &mut HashMap<String, ShortcutBinding>) -> bool {
    let defaults = get_default_settings().bindings;
    let mut changed = false;

    bindings.retain(|id, _| {
        let keep = defaults.contains_key(id);
        if !keep {
            debug!("Dropping binding no longer in defaults: {}", id);
            changed = true;
        }
        keep
    });

    for (key, value) in defaults {
        if let std::collections::hash_map::Entry::Vacant(entry) = bindings.entry(key) {
            debug!("Adding missing binding: {}", entry.key());
            entry.insert(value);
            changed = true;
        }
    }

    changed
}

pub fn write_settings(app: &AppHandle, settings: AppSettings) {
    let store = app
        .store(SETTINGS_STORE_PATH)
        .expect("Failed to initialize store");

    store.set("settings", serde_json::to_value(&settings).unwrap());
}

pub fn get_bindings(app: &AppHandle) -> HashMap<String, ShortcutBinding> {
    let settings = get_settings(app);

    settings.bindings
}

pub fn get_stored_binding(settings: &AppSettings, id: &str) -> Result<ShortcutBinding, String> {
    settings
        .bindings
        .get(id)
        .cloned()
        .ok_or_else(|| format!("Binding with id '{}' not found", id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_binding_returns_the_requested_binding() {
        let settings = get_default_settings();

        let result = get_stored_binding(&settings, "speak");

        assert_eq!(result.unwrap().id, "speak");
    }

    #[test]
    fn unknown_stored_binding_returns_an_error() {
        let settings = get_default_settings();

        let result = get_stored_binding(&settings, "unknown");

        assert_eq!(result.unwrap_err(), "Binding with id 'unknown' not found");
    }

    fn default_settings_json() -> serde_json::Value {
        serde_json::to_value(get_default_settings()).unwrap()
    }

    /// Every field must survive a partial store: a missing key must never fail
    /// the whole-settings parse (#1619). `json!({})` is the extreme case.
    #[test]
    fn empty_store_parses_with_defaults() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({}))
            .expect("all AppSettings fields need serde defaults");
        assert_eq!(settings.theme, default_theme());
        // Bindings default to empty; the load path merges the real defaults in.
        assert!(settings.bindings.is_empty());
    }

    #[test]
    fn salvage_preserves_valid_fields_when_one_value_is_invalid() {
        let mut stored = default_settings_json();
        let map = stored.as_object_mut().unwrap();
        map.insert("app_language".into(), serde_json::json!("fr"));
        map.insert("onboarding_completed".into(), serde_json::json!(true));
        // An enum variant this build doesn't know, e.g. written by a newer
        // version before a downgrade.
        map.insert("theme".into(), serde_json::json!("sepia"));
        stored["bindings"]["speak"]["current_binding"] = serde_json::json!("f13");

        // Precondition: this is exactly the whole-store parse failure from
        // #1619 that used to reset everything to defaults.
        assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

        let salvaged = salvage_settings(&stored);
        assert_eq!(salvaged.app_language, "fr");
        assert!(salvaged.onboarding_completed);
        assert_eq!(salvaged.bindings["speak"].current_binding, "f13");
        assert_eq!(salvaged.theme, default_theme());
    }

    #[test]
    fn salvage_drops_only_wrong_typed_fields() {
        let mut stored = default_settings_json();
        let map = stored.as_object_mut().unwrap();
        map.insert("tts_speed".into(), serde_json::json!("fast"));
        map.insert("theme".into(), serde_json::json!(42));
        map.insert("app_language".into(), serde_json::json!("de"));

        assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

        let salvaged = salvage_settings(&stored);
        assert_eq!(salvaged.tts_speed, default_tts_speed());
        assert_eq!(salvaged.theme, default_theme());
        assert_eq!(salvaged.app_language, "de");
    }

    #[test]
    fn salvage_of_poisoned_bindings_keeps_other_fields() {
        let mut stored = default_settings_json();
        let map = stored.as_object_mut().unwrap();
        // One malformed entry poisons the whole bindings map, but must not
        // take the rest of the settings down with it.
        map.insert(
            "bindings".into(),
            serde_json::json!({ "speak": { "id": 42 } }),
        );
        map.insert("app_language".into(), serde_json::json!("de"));

        assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

        let salvaged = salvage_settings(&stored);
        assert_eq!(salvaged.app_language, "de");
        let defaults = get_default_settings();
        assert_eq!(
            salvaged.bindings["speak"].current_binding,
            defaults.bindings["speak"].current_binding
        );
    }

    #[test]
    fn salvage_tolerates_unknown_keys() {
        let mut stored = default_settings_json();
        let map = stored.as_object_mut().unwrap();
        map.insert(
            "field_from_the_future".into(),
            serde_json::json!({ "nested": true }),
        );
        map.insert("app_language".into(), serde_json::json!("kept"));
        map.insert("theme".into(), serde_json::json!("sepia"));

        let salvaged = salvage_settings(&stored);
        assert_eq!(salvaged.app_language, "kept");
        assert_eq!(salvaged.theme, default_theme());
    }

    #[test]
    fn salvage_of_non_object_store_falls_back_to_defaults() {
        for stored in [
            serde_json::json!("corrupt"),
            serde_json::json!(null),
            serde_json::json!([1, 2, 3]),
        ] {
            let salvaged = salvage_settings(&stored);
            assert_eq!(
                serde_json::to_value(&salvaged).unwrap(),
                default_settings_json()
            );
        }
    }

    fn binding(id: &str, current: &str) -> ShortcutBinding {
        ShortcutBinding {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            default_binding: current.to_string(),
            current_binding: current.to_string(),
        }
    }

    fn current_bindings(bindings: &HashMap<String, ShortcutBinding>) -> Vec<(String, String)> {
        let mut pairs: Vec<_> = bindings
            .iter()
            .map(|(id, b)| (id.clone(), b.current_binding.clone()))
            .collect();
        pairs.sort();
        pairs
    }

    #[test]
    fn binding_sync_drops_ids_not_in_defaults_and_keeps_customizations() {
        let mut bindings = HashMap::from([
            (
                "transcribe".to_string(),
                binding("transcribe", "option+space"),
            ),
            ("cancel".to_string(), binding("cancel", "escape")),
            ("speak".to_string(), binding("speak", "f13")),
        ]);

        assert!(sync_bindings_with_defaults(&mut bindings));

        assert!(!bindings.contains_key("transcribe"));
        assert!(!bindings.contains_key("cancel"));
        assert_eq!(bindings["speak"].current_binding, "f13");
        let mut ids: Vec<_> = bindings.keys().cloned().collect();
        let mut default_ids: Vec<_> = get_default_settings().bindings.into_keys().collect();
        ids.sort();
        default_ids.sort();
        assert_eq!(ids, default_ids);
    }

    #[test]
    fn binding_sync_fills_an_empty_map_from_defaults() {
        let mut bindings = HashMap::new();

        assert!(sync_bindings_with_defaults(&mut bindings));
        assert_eq!(
            current_bindings(&bindings),
            current_bindings(&get_default_settings().bindings)
        );
    }

    #[test]
    fn binding_sync_leaves_current_bindings_unchanged() {
        let mut bindings = get_default_settings().bindings;

        assert!(!sync_bindings_with_defaults(&mut bindings));
        assert_eq!(
            current_bindings(&bindings),
            current_bindings(&get_default_settings().bindings)
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn default_overlay_style_is_minimal_when_overlay_defaults_on() {
        let settings = get_default_settings();
        assert_eq!(settings.overlay_style, OverlayStyle::Minimal);
    }

    #[test]
    fn speech_backend_settings_default_to_http_and_int8() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(settings.tts_backend, TtsBackend::Http);
        assert_eq!(settings.tts_model_variant, ModelVariant::Int8);
    }

    #[test]
    fn speech_backend_settings_store_as_lowercase_names() {
        let mut settings = get_default_settings();
        settings.tts_backend = TtsBackend::Local;
        settings.tts_model_variant = ModelVariant::Fp16;
        let stored = serde_json::to_value(&settings).unwrap();
        assert_eq!(stored["tts_backend"], "local");
        assert_eq!(stored["tts_model_variant"], "fp16");
        for (name, variant) in [
            ("int8", ModelVariant::Int8),
            ("fp16", ModelVariant::Fp16),
            ("fp32", ModelVariant::Fp32),
        ] {
            assert_eq!(
                serde_json::from_value::<ModelVariant>(name.into()).unwrap(),
                variant
            );
        }
    }

    #[test]
    fn legacy_live_overlay_style_deserializes_to_minimal() {
        let style: OverlayStyle = serde_json::from_value(serde_json::json!("live"))
            .expect("legacy \"live\" should deserialize, not error");
        assert_eq!(style, OverlayStyle::Minimal);
    }

    #[test]
    fn legacy_none_overlay_position_deserializes_to_bottom() {
        // A persisted "none" must not fail the whole settings load; the serde
        // alias folds it onto Bottom (visibility is owned by overlay_style).
        let raw = serde_json::json!({ "overlay_position": "none" });
        let position: OverlayPosition =
            serde_json::from_value(raw.get("overlay_position").unwrap().clone())
                .expect("legacy \"none\" should deserialize, not error");
        assert_eq!(position, OverlayPosition::Bottom);
    }
}
