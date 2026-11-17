pub mod audio;
pub mod models;

use crate::settings::{get_settings, write_settings, AppSettings, LogLevel, TtsBackend};
use serde::Serialize;
use specta::Type;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
#[specta::specta]
pub fn stop_speaking(app: AppHandle) {
    crate::actions::stop_speaking(&app, "app");
}

/// Reads a fixed sentence in `voice`, stopping any speech first.
#[tauri::command]
#[specta::specta]
pub fn preview_voice(app: AppHandle, voice: String) -> Result<(), String> {
    crate::actions::preview_voice(&app, &voice)
}

/// Voice ids the selected backend offers: the server's list, or the voice
/// files in the models folder.
#[tauri::command]
#[specta::specta]
pub async fn fetch_tts_voices(app: AppHandle) -> Result<Vec<String>, String> {
    let settings = get_settings(&app);
    match settings.tts_backend {
        TtsBackend::Http => {
            crate::tts::http::list_voices(&crate::tts::http::client(), &settings.tts_base_url).await
        }
        TtsBackend::Local => Ok(crate::tts::local::installed_voices(&models_dir(&app)?)),
    }
}

/// Where the espeak-ng that would read unknown words is, if installed.
#[tauri::command]
#[specta::specta]
pub fn find_espeak_ng() -> Option<String> {
    crate::tts::espeak::find().map(|path| path.display().to_string())
}

/// Where wl-paste, which reads the selection and clipboard on Linux, is.
/// Always `None` elsewhere.
#[tauri::command]
#[specta::specta]
pub fn find_wl_paste() -> Option<String> {
    #[cfg(target_os = "linux")]
    return crate::clipboard::find_wl_paste().map(|path| path.display().to_string());
    #[cfg(not(target_os = "linux"))]
    return None;
}

/// The commands to bind in the desktop's shortcut settings.
#[derive(Serialize, Type)]
pub struct ShortcutHelp {
    /// What runs Mouthy, ready to have `--speak-selection` and the like added.
    pub command: String,
}

/// What to bind in the desktop's shortcut settings, when Mouthy's own
/// shortcuts cannot work (Wayland, unless Event Tap is in use).
#[tauri::command]
#[specta::specta]
pub fn shortcut_help(app: AppHandle) -> Option<ShortcutHelp> {
    crate::shortcut::desktop_binds_shortcuts(&app).then(|| ShortcutHelp {
        command: launch_command(
            std::env::var_os("APPIMAGE").as_deref(),
            std::env::current_exe().ok().as_deref(),
        ),
    })
}

/// The AppImage when Mouthy runs from one (its binary sits in a mount that
/// moves on every launch), else the binary's name, as installed on `PATH`.
fn launch_command(appimage: Option<&OsStr>, exe: Option<&Path>) -> String {
    let program = appimage
        .filter(|path| !path.is_empty())
        .or_else(|| exe.and_then(Path::file_name))
        .map(|program| program.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mouthy".into());
    quoted(program)
}

/// `word` as one argument for a shell or a desktop's command field: in double
/// quotes, with `"`, `` ` ``, `$` and `\` escaped, unless it is plain.
fn quoted(word: String) -> String {
    let plain = word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c));
    if plain && !word.is_empty() {
        return word;
    }
    let mut quoted = String::from('"');
    for c in word.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

#[tauri::command]
#[specta::specta]
pub fn get_app_dir_path(app: AppHandle) -> Result<String, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;

    Ok(app_data_dir.to_string_lossy().to_string())
}

/// Where downloaded speech models live.
pub fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("models"))
        .map_err(|e| format!("Failed to get app data directory: {}", e))
}

#[tauri::command]
#[specta::specta]
pub fn get_app_settings(app: AppHandle) -> Result<AppSettings, String> {
    Ok(get_settings(&app))
}

#[tauri::command]
#[specta::specta]
pub fn get_default_settings() -> Result<AppSettings, String> {
    Ok(crate::settings::get_default_settings())
}

#[tauri::command]
#[specta::specta]
pub fn get_log_dir_path(app: AppHandle) -> Result<String, String> {
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("Failed to get log directory: {}", e))?;

    Ok(log_dir.to_string_lossy().to_string())
}

#[specta::specta]
#[tauri::command]
pub fn set_log_level(app: AppHandle, level: LogLevel) -> Result<(), String> {
    let tauri_log_level: tauri_plugin_log::LogLevel = level.into();
    let log_level: log::Level = tauri_log_level.into();
    // Update the file log level atomic so the filter picks up the new level
    crate::FILE_LOG_LEVEL.store(
        log_level.to_level_filter() as u8,
        std::sync::atomic::Ordering::Relaxed,
    );

    let mut settings = get_settings(&app);
    settings.log_level = level;
    write_settings(&app, settings);

    Ok(())
}

#[specta::specta]
#[tauri::command]
pub fn open_log_dir(app: AppHandle) -> Result<(), String> {
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("Failed to get log directory: {}", e))?;

    let path = log_dir.to_string_lossy().as_ref().to_string();
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|e| format!("Failed to open log directory: {}", e))?;

    Ok(())
}

#[specta::specta]
#[tauri::command]
pub fn open_app_data_dir(app: AppHandle) -> Result<(), String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;

    let path = app_data_dir.to_string_lossy().as_ref().to_string();
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|e| format!("Failed to open app data directory: {}", e))?;

    Ok(())
}

#[specta::specta]
#[tauri::command]
pub fn open_third_party_notices(app: AppHandle) -> Result<(), String> {
    let notices = app
        .path()
        .resolve(
            "resources/THIRD_PARTY_NOTICES.txt",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| format!("Failed to find third-party notices: {}", e))?;

    app.opener()
        .open_path(notices.to_string_lossy(), None::<String>)
        .map_err(|e| format!("Failed to open third-party notices: {}", e))?;

    Ok(())
}

/// Try to initialize Enigo (keyboard/mouse simulation).
/// On macOS, this will return an error if accessibility permissions are not granted.
#[specta::specta]
#[tauri::command]
pub fn initialize_enigo(app: AppHandle) -> Result<(), String> {
    use crate::input::EnigoState;

    // Check if already initialized
    if app.try_state::<EnigoState>().is_some() {
        log::debug!("Enigo already initialized");
        return Ok(());
    }

    // Try to initialize
    match EnigoState::new() {
        Ok(enigo_state) => {
            app.manage(enigo_state);
            log::info!("Enigo initialized successfully after permission grant");
            Ok(())
        }
        Err(e) => {
            if cfg!(target_os = "macos") {
                log::warn!(
                    "Failed to initialize Enigo: {} (accessibility permissions may not be granted)",
                    e
                );
            } else {
                log::warn!("Failed to initialize Enigo: {}", e);
            }
            Err(format!("Failed to initialize input system: {}", e))
        }
    }
}

/// Marker state to track if shortcuts have been initialized.
pub struct ShortcutsInitialized;

/// Initialize keyboard shortcuts.
/// On macOS, this should be called after accessibility permissions are granted.
/// This is idempotent - calling it multiple times is safe.
#[specta::specta]
#[tauri::command]
pub fn initialize_shortcuts(app: AppHandle) -> Result<(), String> {
    // Check if already initialized
    if app.try_state::<ShortcutsInitialized>().is_some() {
        log::debug!("Shortcuts already initialized");
        return Ok(());
    }

    // Initialize shortcuts
    crate::shortcut::init_shortcuts(&app);

    // Mark as initialized before reconciling the macOS Secure Input fallback.
    app.manage(ShortcutsInitialized);
    crate::secure_input::reconcile_fallback(&app);

    log::info!("Shortcuts initialized successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_appimage_is_what_a_shortcut_runs() {
        let exe = Path::new("/tmp/.mount_MouthyX1/usr/bin/mouthy");
        assert_eq!(
            launch_command(
                Some(OsStr::new("/home/me/Applications/Mouthy.AppImage")),
                Some(exe)
            ),
            "/home/me/Applications/Mouthy.AppImage"
        );
        assert_eq!(launch_command(None, Some(exe)), "mouthy");
        assert_eq!(launch_command(Some(OsStr::new("")), Some(exe)), "mouthy");
        assert_eq!(launch_command(None, None), "mouthy");
    }

    #[test]
    fn a_path_with_spaces_or_shell_characters_is_quoted() {
        assert_eq!(
            launch_command(Some(OsStr::new("/home/me/My Apps/Mouthy.AppImage")), None),
            "\"/home/me/My Apps/Mouthy.AppImage\""
        );
        assert_eq!(quoted("/a/$b\"c`d\\e".into()), "\"/a/\\$b\\\"c\\`d\\\\e\"");
        assert_eq!(
            quoted("Mouthy_0.6.0-x86_64.AppImage".into()),
            "Mouthy_0.6.0-x86_64.AppImage"
        );
    }
}
