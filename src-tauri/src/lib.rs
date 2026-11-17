mod actions;
mod audio_out;
pub mod audio_toolkit;
mod autostart;
pub mod cli;
mod clipboard;
mod commands;
mod input;
mod overlay;
mod program;
mod secure_input;
mod settings;
mod shortcut;
#[cfg(unix)]
mod signal_handle;
mod tray;
mod tray_i18n;
mod tts;
mod utils;

pub use cli::CliArgs;
#[cfg(debug_assertions)]
use specta_typescript::{BigIntExportBehavior, Typescript};
use tauri_specta::{collect_commands, Builder};
#[cfg(target_os = "linux")]
pub use utils::gdk_backend;

use env_filter::Builder as EnvFilterBuilder;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use tauri::image::Image;

use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};

use crate::settings::get_settings;

// Global atomic to store the file log level filter
// We use u8 to store the log::LevelFilter as a number
pub static FILE_LOG_LEVEL: AtomicU8 = AtomicU8::new(log::LevelFilter::Debug as u8);

/// When `true`, log records are also forwarded to the webview via the
/// `log://log` event for the debug panel's live log viewer. Gated on debug
/// mode — the live log viewer is its only consumer and only exists in debug
/// mode — so normal runs never broadcast log records (which can include file
/// paths or selected text) onto the frontend event bus. Synced at startup
/// and whenever debug mode is toggled (see `shortcut::change_debug_mode_setting`).
pub static WEBVIEW_LOG_STREAMING: AtomicBool = AtomicBool::new(false);

fn level_filter_from_u8(value: u8) -> log::LevelFilter {
    match value {
        0 => log::LevelFilter::Off,
        1 => log::LevelFilter::Error,
        2 => log::LevelFilter::Warn,
        3 => log::LevelFilter::Info,
        4 => log::LevelFilter::Debug,
        5 => log::LevelFilter::Trace,
        _ => log::LevelFilter::Trace,
    }
}

fn build_console_filter() -> env_filter::Filter {
    let mut builder = EnvFilterBuilder::new();

    match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => {
            if let Err(err) = builder.try_parse(&spec) {
                log::warn!(
                    "Ignoring invalid RUST_LOG value '{}': {}. Falling back to info-level console logging",
                    spec,
                    err
                );
                builder.filter_level(log::LevelFilter::Info);
            }
        }
        _ => {
            builder.filter_level(log::LevelFilter::Info);
        }
    }

    builder.build()
}

fn show_main_window(app: &AppHandle) {
    if let Some(main_window) = app.get_webview_window("main") {
        if let Err(e) = main_window.unminimize() {
            log::error!("Failed to unminimize webview window: {}", e);
        }
        if let Err(e) = main_window.show() {
            log::error!("Failed to show webview window: {}", e);
        }
        if let Err(e) = main_window.set_focus() {
            log::error!("Failed to focus webview window: {}", e);
        }
        #[cfg(target_os = "macos")]
        {
            if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
                log::error!("Failed to set activation policy to Regular: {}", e);
            }
        }
        return;
    }

    let webview_labels = app.webview_windows().keys().cloned().collect::<Vec<_>>();
    log::error!(
        "Main window not found. Webview labels: {:?}",
        webview_labels
    );
}

/// Choose the macOS activation policy the process *launches* with.
///
/// Must run between `build()` and `run()`: that is the only point where
/// `App::set_activation_policy` sets tao's initial policy, which
/// `applicationDidFinishLaunching` then applies directly. Calling the
/// `AppHandle` variant from `setup` (which Tauri runs on `RunEvent::Ready`,
/// i.e. after launch) is instead a runtime Regular → Accessory demotion of an
/// already-activated foreground app — the transition Apple documents as
/// unreliable, and what left a Dock icon behind for start-hidden and
/// login-item launches on macOS 26+ (#1787). Launching as Accessory avoids the
/// transition entirely; showing the window later promotes to Regular, which is
/// the supported direction.
///
/// Mirrors the show-window decision in `setup`: the app launches without a
/// Dock icon only when it will start hidden (setting or `--start-hidden`) AND a
/// tray icon is available (setting and not `--no-tray`). With no tray the Dock
/// icon stays as the only way back into the app (#903).
#[cfg(target_os = "macos")]
fn apply_startup_activation_policy(app: &mut tauri::App) {
    let cli_args = app.state::<CliArgs>().inner().clone();
    let settings = settings::get_settings(app.handle());

    let should_hide = settings.start_hidden || cli_args.start_hidden;
    let tray_available = settings.show_tray_icon && !cli_args.no_tray;

    if should_hide && tray_available {
        log::info!("Starting hidden with tray available: launching as Accessory (no Dock icon)");
        app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    }
}

fn initialize_core_logic(app_handle: &AppHandle) {
    // Note: Enigo (keyboard/mouse simulation) is NOT initialized here.
    // The frontend is responsible for calling the `initialize_enigo` command
    // after onboarding completes. This avoids triggering permission dialogs
    // on macOS before the user is ready.

    app_handle.manage(tray::TrayState::new());
    app_handle.manage(actions::TtsState::default());

    // Note: Shortcuts are NOT initialized here.
    // The frontend is responsible for calling the `initialize_shortcuts` command
    // after permissions are confirmed (on macOS) or after onboarding completes.
    // This matches the pattern used for Enigo initialization.

    // Signals speak and stop. On Linux, SIGUSR1 is deliberately not handled —
    // it belongs to WebKitGTK's garbage collector — see signal_handle.rs.
    #[cfg(unix)]
    signal_handle::setup_signal_handler(app_handle.clone());

    // The macOS activation policy for a start-hidden launch is applied before
    // the event loop runs (see `apply_startup_activation_policy`), not here:
    // by the time `setup` runs the app has already launched as a Regular
    // (Dock) app, and demoting it at runtime is unreliable (#1787).

    // Get the current theme to set the appropriate initial icon
    let initial_theme = tray::get_current_theme(app_handle);

    // Choose the appropriate initial icon based on theme
    let initial_icon_path = tray::get_icon_path(initial_theme, tray::TrayIconState::Idle, false);

    let mut tray_builder = TrayIconBuilder::new()
        .icon(
            Image::from_path(
                app_handle
                    .path()
                    .resolve(initial_icon_path, tauri::path::BaseDirectory::Resource)
                    .unwrap(),
            )
            .unwrap(),
        )
        .tooltip(tray::tray_tooltip())
        .icon_as_template(true);

    // Windows notification-area convention: left click opens the app, right click
    // shows the menu. Elsewhere (macOS menu bar, Linux) the menu stays on left click.
    #[cfg(target_os = "windows")]
    {
        tray_builder = tray_builder
            .show_menu_on_left_click(false)
            .on_tray_icon_event(|tray, event| {
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
                let opens_window = matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                );
                if opens_window {
                    show_main_window(tray.app_handle());
                }
            });
    }
    #[cfg(not(target_os = "windows"))]
    {
        tray_builder = tray_builder.show_menu_on_left_click(true);
    }

    let tray_builder = tray_builder.on_menu_event(|app, event| match event.id.as_ref() {
        "settings" => {
            show_main_window(app);
        }
        "secure_input_warning" => {
            // Full explanation lives in the settings-window banner
            show_main_window(app);
        }
        "speak_selection" => {
            actions::speak_selection(app, "tray");
        }
        "stop" => {
            actions::stop_speaking(app, "tray");
        }
        "quit" => {
            app.exit(0);
        }
        _ => {}
    });
    // Building it panics when no appindicator library loads, as on a Linux
    // desktop without one. Mouthy runs on without a tray; launching it again
    // raises Settings.
    match panic::catch_unwind(AssertUnwindSafe(|| tray_builder.build(app_handle))) {
        Ok(Ok(tray)) => {
            app_handle.manage(tray);
        }
        Ok(Err(e)) => log::error!("Running without a tray icon: {e}"),
        Err(payload) => {
            let reason = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("it could not be built");
            log::error!("Running without a tray icon: {reason}");
        }
    }

    // Initialize tray menu with idle state
    tray::update_tray_menu(app_handle);

    // Apply show_tray_icon setting
    let settings = settings::get_settings(app_handle);
    if !settings.show_tray_icon {
        tray::set_tray_visibility(app_handle, false);
    }

    // Apply the autostart preference (SMAppService login item on macOS,
    // tauri-plugin-autostart elsewhere)
    autostart::apply_autostart(app_handle, settings.autostart_enabled);

    // Create the overlay window (hidden by default)
    utils::create_overlay(app_handle);
}

/// Makes `dir` the working directory, returning the one to go back to.
#[cfg(target_os = "linux")]
fn enter_dir(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let back = std::env::current_dir().ok()?;
    std::env::set_current_dir(dir).ok()?;
    Some(back)
}

fn log_plugin(console_filter: env_filter::Filter) -> tauri::plugin::TauriPlugin<tauri::Wry> {
    LogBuilder::new()
        .level(log::LevelFilter::Trace) // Set to most verbose level globally
        // misaki-rs logs every sentence it reads at debug; keep selected text out of the logs.
        .level_for("misaki_rs", log::LevelFilter::Warn)
        .max_file_size(500_000)
        .rotation_strategy(RotationStrategy::KeepOne)
        .clear_targets()
        .targets([
            // Console output respects RUST_LOG environment variable
            Target::new(TargetKind::Stdout)
                .filter(move |metadata| console_filter.enabled(metadata)),
            // File logs respect the user's settings (stored in FILE_LOG_LEVEL atomic)
            Target::new(TargetKind::LogDir {
                file_name: Some("mouthy".into()),
            })
            .filter(|metadata| {
                let file_level = FILE_LOG_LEVEL.load(Ordering::Relaxed);
                metadata.level() <= level_filter_from_u8(file_level)
            }),
            // Stream logs to the webview (via the `log://log` event) so the
            // debug panel's live log viewer can show them in real time. Only
            // active while debug mode is on (its sole consumer), and shares the
            // file log level so the "Log Level" setting controls verbosity.
            Target::new(TargetKind::Webview).filter(|metadata| {
                WEBVIEW_LOG_STREAMING.load(Ordering::Relaxed)
                    && metadata.level()
                        <= level_filter_from_u8(FILE_LOG_LEVEL.load(Ordering::Relaxed))
            }),
        ])
        .build()
}

/// Whether a tray icon shows: built, turned on in Settings, and no `--no-tray`.
fn tray_available(app: &AppHandle) -> bool {
    get_settings(app).show_tray_icon
        && !app.try_state::<CliArgs>().is_some_and(|args| args.no_tray)
        && app.try_state::<TrayIcon>().is_some()
}

#[tauri::command]
#[specta::specta]
fn show_main_window_command(app: AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(cli_args: CliArgs) {
    // Parse console logging directives from RUST_LOG, falling back to info-level logging
    // when the variable is unset
    let console_filter = build_console_filter();

    let specta_builder = Builder::<tauri::Wry>::new().commands(collect_commands![
        shortcut::change_binding,
        shortcut::reset_binding,
        shortcut::change_theme_setting,
        shortcut::change_start_hidden_setting,
        shortcut::change_autostart_setting,
        shortcut::change_overlay_position_setting,
        shortcut::change_overlay_style_setting,
        shortcut::change_debug_mode_setting,
        shortcut::change_experimental_enabled_setting,
        shortcut::suspend_all_bindings,
        shortcut::resume_all_bindings,
        shortcut::change_app_language_setting,
        shortcut::change_keyboard_implementation_setting,
        shortcut::get_keyboard_implementation,
        shortcut::change_show_tray_icon_setting,
        shortcut::change_onboarding_completed_setting,
        shortcut::change_tts_base_url_setting,
        shortcut::change_tts_voice_setting,
        shortcut::change_tts_speed_setting,
        shortcut::change_tts_volume_setting,
        shortcut::change_tts_backend_setting,
        shortcut::change_tts_model_variant_setting,
        shortcut::change_tts_espeak_fallback_setting,
        shortcut::change_tts_language_setting,
        shortcut::handy_keys::start_handy_keys_recording,
        shortcut::handy_keys::stop_handy_keys_recording,
        secure_input::get_secure_input_status,
        secure_input::run_keyboard_diagnostic,
        show_main_window_command,
        commands::stop_speaking,
        commands::preview_voice,
        commands::fetch_tts_voices,
        commands::find_espeak_ng,
        commands::find_wl_paste,
        commands::shortcut_help,
        commands::get_app_dir_path,
        commands::get_app_settings,
        commands::get_default_settings,
        commands::get_log_dir_path,
        commands::set_log_level,
        commands::open_log_dir,
        commands::open_app_data_dir,
        commands::open_third_party_notices,
        commands::initialize_enigo,
        commands::initialize_shortcuts,
        commands::audio::get_available_output_devices,
        commands::audio::set_selected_output_device,
        commands::audio::get_selected_output_device,
        commands::models::list_model_files,
        commands::models::download_model_file,
        commands::models::cancel_model_download,
        commands::models::delete_model_file,
        commands::models::open_models_dir,
    ]);

    #[cfg(debug_assertions)] // <- Only export on non-release builds
    specta_builder
        .export(
            Typescript::default().bigint(BigIntExportBehavior::Number),
            "../src/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    let invoke_handler = specta_builder.invoke_handler();

    #[allow(unused_mut)]
    let mut builder =
        tauri::Builder::default().device_event_filter(tauri::DeviceEventFilter::Always);

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    // An AppImage starts Mouthy in the image's `usr` folder, which WebKit finds
    // its helper programs from. The single-instance plugin hands the working
    // directory to a running Mouthy, so the caller's is set while plugins
    // start, before any window opens, and the image's put back after.
    let launch_dir = utils::launch_dir(
        std::env::var_os("APPIMAGE").as_deref(),
        std::env::var_os("OWD").as_deref(),
    );
    #[cfg(target_os = "linux")]
    let image_dir = launch_dir.as_deref().and_then(enter_dir);

    // Single-instance forwards CLI args to an already-running Mouthy and exits.
    // The callback runs on an async runtime worker: hand off, never block.
    // Registered before the log plugin, so a second process exits before it
    // opens, or rotates away, the running app's log file.
    #[allow(unused_mut)]
    let mut app = builder
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            if args.iter().any(|a| a == "--speak-selection") {
                actions::speak_selection(app, "CLI");
            } else if args.iter().any(|a| a == "--speak-clipboard") {
                actions::speak_clipboard(app, "CLI");
            } else if let Some(request) = CliArgs::handed_over_speak_file(&args, &cwd) {
                actions::speak_file(app, &request.path, request.voice, request.language, "CLI");
            } else if args.iter().any(|a| a == "--stop") {
                actions::stop_speaking(app, "CLI");
            } else {
                // A second process was launched without remote-control flags
                // (e.g. the binary run from a shell). On macOS, relaunching the
                // bundle from Spotlight/Finder/Dock does not start a process —
                // it arrives as RunEvent::Reopen below — but treat this the
                // same way: raise the window and recreate a possibly vanished
                // tray icon (#1948).
                #[cfg(target_os = "macos")]
                tray::recreate_tray_icon(app);
                show_main_window(app);
            }
        }))
        .plugin(log_plugin(console_filter))
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_macos_permissions::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(cli_args.clone())
        .manage(commands::models::ModelDownloads::default())
        .setup(move |app| {
            // Only used on Windows, to disable WebView2 browser accelerators.
            #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
            let main_window =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("/".into()))
                    .title("Mouthy")
                    .inner_size(680.0, 570.0)
                    .min_inner_size(680.0, 570.0)
                    .resizable(true)
                    .maximizable(true)
                    .visible(false)
                    .build()?;

            // Disable WebView2 browser accelerators (F5, F6, Ctrl+F, F12, ...).
            // A settings window has no use for them, and pressing F6 while
            // recording a shortcut was reported to turn the whole window white
            // (cjpais/Handy#1940), likely by triggering WebView2 focus cycling.
            // DevTools stays enabled; only the F12 accelerator is lost.
            #[cfg(target_os = "windows")]
            {
                let _ = main_window.with_webview(|webview| unsafe {
                    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
                    use windows::core::Interface;

                    let result = webview
                        .controller()
                        .CoreWebView2()
                        .and_then(|core| core.Settings())
                        .and_then(|settings| settings.cast::<ICoreWebView2Settings3>())
                        .and_then(|settings| settings.SetAreBrowserAcceleratorKeysEnabled(false));

                    if let Err(error) = result {
                        log::warn!("Failed to disable WebView2 browser accelerators: {error}");
                    }
                });
            }

            #[cfg(target_os = "linux")]
            let new_store = !settings::is_stored(app.handle());
            let mut settings = get_settings(app.handle());

            // Apply the persisted appearance theme to the native title bar before
            // the window is shown, so it matches the in-app palette without a flash
            // of the wrong theme. See `apply_window_theme` for what this does per
            // platform.
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            shortcut::apply_window_theme(app.handle(), settings.theme);

            // CLI --debug flag overrides debug_mode and log level (runtime-only, not persisted)
            if cli_args.debug {
                settings.debug_mode = true;
                settings.log_level = settings::LogLevel::Trace;
            }

            let tauri_log_level: tauri_plugin_log::LogLevel = settings.log_level.into();
            let file_log_level: log::Level = tauri_log_level.into();
            // Store the file log level in the atomic for the filter to use
            FILE_LOG_LEVEL.store(file_log_level.to_level_filter() as u8, Ordering::Relaxed);
            // Only forward logs to the webview while debug mode is on (the live log
            // viewer is the sole consumer and only exists in debug mode). This also
            // honors the runtime `--debug` override applied to `settings` above.
            WEBVIEW_LOG_STREAMING.store(settings.debug_mode, Ordering::Relaxed);
            let app_handle = app.handle().clone();

            initialize_core_logic(&app_handle);

            #[cfg(target_os = "linux")]
            {
                use gtk::glib::prelude::ObjectExt;
                // GDK_BACKEND may list several backends, so name the one GTK opened.
                log::info!(
                    "Display backend: {} (GDK_BACKEND {})",
                    gtk::gdk::Display::default()
                        .map_or("none".into(), |display| display.type_().name().to_string()),
                    std::env::var("GDK_BACKEND").unwrap_or_else(|_| "not set".into())
                );
                if matches!(
                    std::env::var("MOUTHY_GDK_BACKEND"),
                    Err(std::env::VarError::NotUnicode(_))
                ) {
                    log::debug!("MOUTHY_GDK_BACKEND is not valid UTF-8, so it was ignored");
                }
                // Decided once the overlay window exists, and only for a new store.
                if new_store {
                    utils::choose_first_overlay_style(&app_handle);
                }
            }

            // Load the local voice now rather than on the first request.
            actions::preload_local_voice(&app_handle);

            // Secure Input monitor (macOS): detects stuck secure input that
            // silently blocks keyed shortcuts, warns the user, and activates
            // the Carbon fallback. See secure_input.rs and issue #1578.
            secure_input::init(&app_handle);

            // Hide tray icon if --no-tray was passed
            if cli_args.no_tray {
                tray::set_tray_visibility(&app_handle, false);
            }

            // Show main window only if not starting hidden.
            // CLI --start-hidden flag overrides the setting.
            let should_hide = settings.start_hidden || cli_args.start_hidden;

            // If start_hidden but tray is disabled, we must show the window
            // anyway. Without a tray icon, the dock is the only way back in.
            // Keep in sync with `apply_startup_activation_policy` (macOS).
            if !should_hide || !tray_available(&app_handle) {
                show_main_window(&app_handle);
            }

            // The other speak flags only reach a running instance, but a script
            // handing over text expects it spoken even when it launched Mouthy.
            let cwd = launch_dir
                .clone()
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_default();
            if let Some(request) = cli_args.speak_file_in(&cwd) {
                actions::speak_launch_file(
                    &app_handle,
                    &request.path,
                    request.voice,
                    request.language,
                );
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _res = window.hide();

                // Without a tray there would be no way back, and no Quit.
                #[cfg(not(target_os = "macos"))]
                if window.label() == "main" && !tray_available(window.app_handle()) {
                    window.app_handle().exit(0);
                }

                #[cfg(target_os = "macos")]
                {
                    let settings = get_settings(window.app_handle());
                    let tray_visible =
                        settings.show_tray_icon && !window.app_handle().state::<CliArgs>().no_tray;
                    if tray_visible {
                        // Tray is available: hide the dock icon, app lives in the tray
                        let res = window
                            .app_handle()
                            .set_activation_policy(tauri::ActivationPolicy::Accessory);
                        if let Err(e) = res {
                            log::error!("Failed to set activation policy: {}", e);
                        }
                    }
                    // No tray: keep the dock icon visible so the user can reopen
                }
            }
            tauri::WindowEvent::ThemeChanged(theme) => {
                log::info!("Theme changed to: {:?}", theme);
                // Re-apply the current tray state with the new theme's icon set
                utils::refresh_tray_icon(window.app_handle());
            }
            _ => {}
        })
        .invoke_handler(invoke_handler)
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    #[cfg(target_os = "linux")]
    if let Some(image_dir) = image_dir {
        if let Err(e) = std::env::set_current_dir(&image_dir) {
            log::error!("Cannot return to {}: {e}", image_dir.display());
        }
    }

    // Must sit between build() and run(): see the doc comment.
    #[cfg(target_os = "macos")]
    apply_startup_activation_policy(&mut app);

    // `app` is only used by the macOS arm.
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    app.run(|app, event| match &event {
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            // Fired when the already-running bundle is launched again from
            // Spotlight/Finder or the Dock icon is clicked. If the settings
            // window is hidden, the user is likely looking for a tray icon
            // that vanished (#1948): recreate it. When the window is
            // already visible this is just a focus request and the tray is
            // left alone.
            let window_visible = app
                .get_webview_window("main")
                .and_then(|w| w.is_visible().ok())
                .unwrap_or(false);
            if !window_visible {
                tray::recreate_tray_icon(app);
            }
            show_main_window(app);
        }
        _ => {}
    });
}
