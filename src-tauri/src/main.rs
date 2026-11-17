// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use mouthy_app_lib::CliArgs;

fn main() {
    #[cfg(target_os = "linux")]
    {
        // DMABUF renderer causes crashes on various GPU/display server configurations
        // See: https://github.com/tauri-apps/tauri/issues/9394
        // Preserve explicit overrides so affected users can opt out with =0 (#2166).
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        let in_appimage = std::env::var_os("APPIMAGE").is_some_and(|path| !path.is_empty());
        let wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let current_desktop = std::env::var_os("XDG_CURRENT_DESKTOP");
        let requested = std::env::var("MOUTHY_GDK_BACKEND").ok();
        if let Some(backend) = mouthy_app_lib::gdk_backend(
            in_appimage,
            wayland_display.as_deref(),
            current_desktop.as_deref(),
            requested.as_deref(),
        ) {
            std::env::set_var("GDK_BACKEND", backend);
        }
    }

    let cli_args = CliArgs::parse();
    mouthy_app_lib::run(cli_args)
}
