use std::ffi::OsStr;
use std::path::PathBuf;

// Re-export all utility modules for easy access
pub use crate::overlay::*;
pub use crate::tray::*;

/// Whether this is a Linux Wayland session, whatever display backend GTK uses.
pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && is_wayland_session(
            std::env::var_os("WAYLAND_DISPLAY").as_deref(),
            std::env::var_os("XDG_SESSION_TYPE").as_deref(),
        )
}

/// The directory Mouthy was started from when an AppImage's launcher moved
/// it into the image: the AppImage runtime leaves the caller's in `OWD`.
pub fn launch_dir(appimage: Option<&OsStr>, owd: Option<&OsStr>) -> Option<PathBuf> {
    appimage.filter(|path| !path.is_empty())?;
    owd.map(PathBuf::from).filter(|dir| dir.is_absolute())
}

/// A Wayland session from its `WAYLAND_DISPLAY` and `XDG_SESSION_TYPE`.
fn is_wayland_session(wayland_display: Option<&OsStr>, session_type: Option<&OsStr>) -> bool {
    wayland_display.is_some_and(|display| !display.is_empty())
        || session_type.is_some_and(|kind| kind.eq_ignore_ascii_case("wayland"))
}

/// The `GDK_BACKEND` to set before GTK starts, if any. Only the AppImage's
/// own value is replaced: its launch hook exports `GDK_BACKEND=x11` over the
/// user's, so `MOUTHY_GDK_BACKEND` is the way to choose there. Without it, a
/// Wayland session gets native Wayland, where the overlay can use the GTK
/// layer shell, with X11 as GDK's fallback should the Wayland socket be gone.
/// GNOME has no layer shell, and its overlay can be kept from taking focus
/// only under XWayland, so GNOME keeps the hook's `x11`.
#[cfg(any(target_os = "linux", test))]
pub fn gdk_backend<'a>(
    in_appimage: bool,
    wayland_display: Option<&OsStr>,
    current_desktop: Option<&OsStr>,
    requested: Option<&'a str>,
) -> Option<&'a str> {
    if !in_appimage {
        return None;
    }
    let wayland_session = wayland_display.is_some_and(|display| !display.is_empty());
    requested
        .map(str::trim)
        .filter(|backend| !backend.is_empty())
        .or((wayland_session && !is_gnome(current_desktop)).then_some("wayland,x11"))
}

/// Whether `XDG_CURRENT_DESKTOP`, a colon-separated list, names GNOME. Desktops
/// built on GNOME Shell list it after their own name (`ubuntu:GNOME`).
#[cfg(any(target_os = "linux", test))]
fn is_gnome(current_desktop: Option<&OsStr>) -> bool {
    current_desktop
        .and_then(OsStr::to_str)
        .is_some_and(|desktops| {
            desktops
                .split(':')
                .any(|desktop| desktop.trim().eq_ignore_ascii_case("gnome"))
        })
}

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

    #[test]
    fn a_wayland_display_or_session_type_means_wayland() {
        let os = |s: &'static str| Some(OsStr::new(s));
        assert!(is_wayland_session(os("wayland-0"), None));
        assert!(is_wayland_session(None, os("wayland")));
        assert!(is_wayland_session(None, os("Wayland")));
        assert!(is_wayland_session(os("wayland-0"), os("x11")));
        assert!(!is_wayland_session(None, os("x11")));
        assert!(!is_wayland_session(os(""), os("tty")));
        assert!(!is_wayland_session(None, None));
    }

    #[test]
    fn an_appimage_was_started_from_owd() {
        let os = |s: &'static str| Some(OsStr::new(s));
        let image = os("/home/me/Applications/Mouthy.AppImage");
        assert_eq!(
            launch_dir(image, os("/home/me/notes")),
            Some(PathBuf::from("/home/me/notes"))
        );
        assert_eq!(launch_dir(image, None), None);
        assert_eq!(launch_dir(image, os("notes")), None, "not absolute");
        assert_eq!(launch_dir(None, os("/home/me/notes")), None, "no AppImage");
        assert_eq!(launch_dir(os(""), os("/home/me/notes")), None);
    }

    #[test]
    fn the_appimage_runs_on_wayland_unless_told_otherwise() {
        let os = |s: &'static str| Some(OsStr::new(s));
        let wayland = os("wayland-0");
        // The hook's GDK_BACKEND=x11 is always replaced inside the image, so
        // it is not an input.
        for desktop in [None, os("KDE"), os("niri"), os("GNOMEish")] {
            assert_eq!(
                gdk_backend(true, wayland, desktop, None),
                Some("wayland,x11")
            );
            assert_eq!(
                gdk_backend(true, wayland, desktop, Some("  ")),
                Some("wayland,x11")
            );
        }
        assert_eq!(
            gdk_backend(true, wayland, os("KDE"), Some(" x11 ")),
            Some("x11")
        );
        assert_eq!(
            gdk_backend(true, wayland, os("KDE"), Some("wayland")),
            Some("wayland")
        );
        // GNOME keeps the hook's x11 unless asked.
        for gnome in [os("GNOME"), os("ubuntu:GNOME"), os("Pop:gnome")] {
            assert_eq!(gdk_backend(true, wayland, gnome, None), None);
            assert_eq!(
                gdk_backend(true, wayland, gnome, Some("wayland")),
                Some("wayland")
            );
        }
        // An X11 session keeps the hook's x11 unless asked.
        assert_eq!(gdk_backend(true, None, os("KDE"), None), None);
        assert_eq!(gdk_backend(true, os(""), os("KDE"), None), None);
        assert_eq!(
            gdk_backend(true, None, None, Some("wayland")),
            Some("wayland")
        );
        // A deb or rpm user's own GDK_BACKEND is never touched.
        assert_eq!(gdk_backend(false, wayland, os("KDE"), None), None);
        assert_eq!(gdk_backend(false, wayland, os("GNOME"), Some("x11")), None);
        assert_eq!(gdk_backend(false, None, None, Some("wayland")), None);
    }
}
