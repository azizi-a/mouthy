use std::time::{Duration, Instant};
use tauri::AppHandle;

#[cfg(target_os = "macos")]
use objc2_app_kit::NSEventModifierFlags;

#[cfg(target_os = "macos")]
fn with_enigo<T>(
    app_handle: &AppHandle,
    f: impl FnOnce(&mut enigo::Enigo) -> Result<T, String>,
) -> Result<T, String> {
    use tauri::Manager;
    let enigo_state = app_handle
        .try_state::<crate::input::EnigoState>()
        .ok_or("Enigo state not initialized")?;
    let mut enigo = enigo_state
        .0
        .lock()
        .map_err(|e| format!("Failed to lock Enigo: {}", e))?;
    f(&mut enigo)
}

/// How long ⌘ stays held after the C click. Apps read the modifier from the
/// C event's own flags; the hold covers the few that poll keyboard state.
#[cfg(target_os = "macos")]
const COPY_HOLD_MS: u64 = 30;
/// How long to wait for the copy to land on the clipboard.
#[cfg(target_os = "macos")]
const COPY_TIMEOUT: Duration = Duration::from_millis(200);
#[cfg(target_os = "macos")]
const COPY_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// How long to wait for the hotkey's ⌥/⌃/⇧ to come up before copying anyway.
/// Held, they can turn the copy into ⌥⌘C, and the hotkey listener takes our
/// synthetic keys for the hotkey's release, so it can fire again.
#[cfg(target_os = "macos")]
const MODIFIER_RELEASE_TIMEOUT: Duration = Duration::from_millis(300);

/// What a selection capture copied.
pub struct Selection {
    /// `None` when nothing was copied or the copy had no text.
    pub text: Option<String>,
    /// When ⌘C went out with ⌥, ⌃ or ⇧ still down. A hotkey listener can take
    /// its synthetic ⌘-up for the release of a hotkey held with them.
    pub copied_while_held_at: Option<Instant>,
}

/// Copies the current selection in the frontmost app, leaving the clipboard
/// as it was.
///
/// Must run on the main thread on macOS (keyboard layout lookup).
#[cfg(target_os = "macos")]
pub fn capture_selection(app_handle: &AppHandle) -> Result<Selection, String> {
    use log::{debug, info, warn};
    use objc2_app_kit::{NSApplication, NSEvent};

    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return Err("Selection capture must run on the main thread".into());
    };
    // Our own window would get the ⌘C only once this returns: too late to be
    // read, in time to overwrite the clipboard.
    if NSApplication::sharedApplication(mtm).isActive() {
        info!("Mouthy is the active app, so there is no selection to copy");
        return Ok(Selection {
            text: None,
            copied_while_held_at: None,
        });
    }

    let waited = Instant::now();
    let modifiers_up =
        wait_for_modifier_release(MODIFIER_RELEASE_TIMEOUT, NSEvent::modifierFlags_class);
    if modifiers_up {
        debug!("Modifier keys up after {:?}", waited.elapsed());
    } else {
        warn!(
            "Modifier keys still down after {:?}; copying anyway",
            waited.elapsed()
        );
    }

    let mut copied_at = None;
    let text = pasteboard::capture(|| {
        copied_at = Some(Instant::now());
        with_enigo(app_handle, |enigo| {
            crate::input::send_copy(enigo, COPY_HOLD_MS)
        })
    })?;
    Ok(Selection {
        text,
        copied_while_held_at: copied_at.filter(|_| !modifiers_up),
    })
}

/// True when none of ⌥, ⌃ and ⇧ is down. ⌘ is part of the copy anyway, and
/// Caps Lock stays on without a key held.
#[cfg(target_os = "macos")]
fn modifiers_clear(flags: NSEventModifierFlags) -> bool {
    !flags.intersects(
        NSEventModifierFlags::Option | NSEventModifierFlags::Control | NSEventModifierFlags::Shift,
    )
}

/// Waits up to `timeout` for `flags` to report ⌥, ⌃ and ⇧ all up. False if
/// they were still down when it gave up.
#[cfg(target_os = "macos")]
fn wait_for_modifier_release(
    timeout: Duration,
    mut flags: impl FnMut() -> NSEventModifierFlags,
) -> bool {
    poll(timeout, COPY_POLL_INTERVAL, || {
        modifiers_clear(flags()).then_some(())
    })
    .is_some()
}

/// The PRIMARY selection: the text last highlighted in any app.
///
/// Never call it on the main thread: wl-paste waits for the selection's
/// owner, which may be Mouthy itself.
#[cfg(target_os = "linux")]
pub fn capture_selection(_app_handle: &AppHandle) -> Result<Selection, String> {
    wayland::read_primary().map(|text| Selection {
        text,
        copied_while_held_at: None,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn capture_selection(_app_handle: &AppHandle) -> Result<Selection, String> {
    Err("selection capture is macOS-only".into())
}

/// What the clipboard holds, as far as reading it aloud goes.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
pub enum ClipboardText {
    Text(String),
    /// No plain text.
    Empty,
    /// Text a password manager marked as secret, so it is not read.
    Secret,
}

/// The clipboard's plain text, read without changing the clipboard.
#[cfg(target_os = "macos")]
pub fn read_text() -> Result<ClipboardText, String> {
    Ok(pasteboard::read_text())
}

/// Never call it on the main thread: wl-paste waits for the clipboard's
/// owner, which may be Mouthy itself.
#[cfg(target_os = "linux")]
pub fn read_text() -> Result<ClipboardText, String> {
    wayland::read_clipboard()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn read_text() -> Result<ClipboardText, String> {
    Err("clipboard reading is macOS-only".into())
}

/// Where wl-paste is, if wl-clipboard is installed.
#[cfg(target_os = "linux")]
pub fn find_wl_paste() -> Option<std::path::PathBuf> {
    wayland::find()
}

/// The selection and clipboard on Wayland, read by wl-clipboard's `wl-paste`,
/// a separately installed program.
#[cfg(any(target_os = "linux", test))]
mod wayland {
    use super::ClipboardText;
    use crate::program::{self, Output};
    use log::{debug, info};
    use std::path::Path;
    use std::time::Duration;

    /// wl-paste waits for the app that owns the text; one that hangs must not
    /// hold up the press for long.
    const TIMEOUT: Duration = Duration::from_millis(500);
    /// Enough for every character read aloud: at most four bytes each.
    const TEXT_BYTES: usize = crate::actions::TEXT_CHAR_CAP * 4;
    const TYPES_BYTES: usize = 64 * 1024;
    /// Plain-text types, best first. The first one offered is read, so a
    /// browser's `text/html` never is.
    const PLAIN_TEXT: [&str; 5] = [
        "text/plain;charset=utf-8",
        "UTF8_STRING",
        "text/plain",
        "STRING",
        "TEXT",
    ];
    /// What KDE's password managers add to a secret they copy. Many others,
    /// and apps copying through XWayland, add nothing.
    const PASSWORD_HINT: &str = "x-kde-passwordManagerHint";
    /// How wl-paste says there is nothing to read, in its old and new
    /// wording. Anything else it prints is an error.
    const NOTHING: [&str; 4] = [
        "Nothing is copied",
        "No selection",
        "No suitable type",
        "Clipboard content is not available as",
    ];

    #[cfg(target_os = "linux")]
    pub fn find() -> Option<std::path::PathBuf> {
        program::find(
            &[
                "/usr/bin/wl-paste",
                "/home/linuxbrew/.linuxbrew/bin/wl-paste",
            ],
            "wl-paste",
            std::env::var_os("PATH"),
        )
    }

    #[cfg(target_os = "linux")]
    fn wl_paste(input: &str) -> Result<std::path::PathBuf, String> {
        if !crate::utils::is_wayland() {
            return Err(format!("Reading the {input} needs a Wayland session"));
        }
        find().ok_or_else(|| {
            "Install wl-clipboard (package manager, or brew install wl-clipboard)".to_string()
        })
    }

    #[cfg(target_os = "linux")]
    pub fn read_primary() -> Result<Option<String>, String> {
        read(&wl_paste("selection")?, true, TIMEOUT)
    }

    #[cfg(target_os = "linux")]
    pub fn read_clipboard() -> Result<ClipboardText, String> {
        read_clipboard_with(&wl_paste("clipboard")?, TIMEOUT)
    }

    /// The selection's (PRIMARY's) or the clipboard's plain text, `None` if
    /// it has none.
    fn read(wl_paste: &Path, primary: bool, timeout: Duration) -> Result<Option<String>, String> {
        match types(wl_paste, primary, timeout)? {
            Some(types) => read_as(wl_paste, primary, &types, timeout),
            None => Ok(None),
        }
    }

    fn read_clipboard_with(wl_paste: &Path, timeout: Duration) -> Result<ClipboardText, String> {
        let Some(types) = types(wl_paste, false, timeout)? else {
            return Ok(ClipboardText::Empty);
        };
        if names_secret(&types) {
            info!("The clipboard holds a password manager's secret; not reading it");
            return Ok(ClipboardText::Secret);
        }
        Ok(match read_as(wl_paste, false, &types, timeout)? {
            Some(text) => ClipboardText::Text(text),
            None => ClipboardText::Empty,
        })
    }

    /// The types offered, one a line; `None` when nothing is.
    fn types(wl_paste: &Path, primary: bool, timeout: Duration) -> Result<Option<String>, String> {
        let args: &[&str] = if primary {
            &["--primary", "--list-types"]
        } else {
            &["--list-types"]
        };
        pasted(program::run(wl_paste, args, timeout, TYPES_BYTES))
    }

    /// The text in the best plain-text type among `types`; `None` when none
    /// is offered or it is empty.
    fn read_as(
        wl_paste: &Path,
        primary: bool,
        types: &str,
        timeout: Duration,
    ) -> Result<Option<String>, String> {
        let Some(mime) = plain_text_type(types) else {
            debug!("No plain text among {} types", types.lines().count());
            return Ok(None);
        };
        let mut args = vec!["--no-newline", "--type", mime];
        if primary {
            args.insert(0, "--primary");
        }
        let text = pasted(program::run(wl_paste, &args, timeout, TEXT_BYTES))?;
        Ok(text.filter(|text| !text.is_empty()))
    }

    fn plain_text_type(types: &str) -> Option<&str> {
        PLAIN_TEXT.iter().find_map(|wanted| {
            types
                .lines()
                .map(str::trim)
                .find(|offered| offered.eq_ignore_ascii_case(wanted))
        })
    }

    fn names_secret(types: &str) -> bool {
        types.lines().any(|name| name.trim() == PASSWORD_HINT)
    }

    /// wl-paste's output as text (invalid UTF-8 replaced, as from an X11 app's
    /// Latin-1), `None` when it said there was nothing to read, or why it failed.
    fn pasted(run: Result<Output, program::Error>) -> Result<Option<String>, String> {
        let output = run.map_err(|e| format!("wl-paste: {e}"))?;
        // No status: stopped once it had printed all that is read.
        if output.status.is_none_or(|status| status.success()) {
            return Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if NOTHING.iter().any(|nothing| stderr.contains(nothing)) {
            return Ok(None);
        }
        let reason = stderr
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string)
            .or_else(|| output.status.map(|status| status.to_string()))
            .unwrap_or_default();
        Err(format!("wl-paste failed: {reason}"))
    }

    #[cfg(all(test, unix))]
    mod tests {
        use super::*;
        use std::path::PathBuf;
        use std::process::Command;

        /// A stand-in wl-paste running `body` as a shell script. A child
        /// process writes it: had this one, a test forking meanwhile could
        /// hold the file open for writing, failing its exec with "Text file busy".
        fn fake(dir: &tempfile::TempDir, body: &str) -> PathBuf {
            let path = dir.path().join("wl-paste");
            let written = Command::new("/bin/sh")
                .args(["-c", r#"printf '%s\n' "$1" > "$2" && chmod 755 "$2""#, "sh"])
                .arg(format!("#!/bin/sh\n{body}"))
                .arg(&path)
                .status()
                .unwrap();
            assert!(written.success());
            path
        }

        /// Long enough for a busy test machine; only the hung owner waits it out.
        const SLOW: Duration = Duration::from_secs(10);

        fn primary(body: &str) -> Result<Option<String>, String> {
            let dir = tempfile::tempdir().unwrap();
            read(&fake(&dir, body), true, SLOW)
        }

        fn clipboard(body: &str) -> Result<ClipboardText, String> {
            let dir = tempfile::tempdir().unwrap();
            read_clipboard_with(&fake(&dir, body), SLOW)
        }

        /// A stand-in offering `types` on PRIMARY and running `text` for the
        /// first of them.
        fn offering(types: &str, text: &str) -> String {
            let first = types.lines().next().unwrap_or_default();
            format!(
                r#"case "$*" in
"--primary --list-types") printf '{types}\n' ;;
"--primary --no-newline --type {first}") {text} ;;
*) exit 9 ;;
esac"#
            )
        }

        #[test]
        fn the_selection_is_primary_as_plain_text_without_a_newline() {
            let read = primary(
                r#"case "$*" in
"--primary --list-types") printf 'text/html\ntext/plain;charset=utf-8\nUTF8_STRING\n' ;;
"--primary --no-newline --type text/plain;charset=utf-8") printf 'Read this.\nAnd this.' ;;
*) exit 9 ;;
esac"#,
            );
            assert_eq!(read, Ok(Some("Read this.\nAnd this.".into())));
        }

        #[test]
        fn html_alone_is_not_read() {
            assert_eq!(primary("printf 'text/html\\n'"), Ok(None));
        }

        #[test]
        fn the_best_plain_text_type_offered_is_chosen() {
            assert_eq!(
                plain_text_type("text/html\nSTRING\nUTF8_STRING\n"),
                Some("UTF8_STRING")
            );
            assert_eq!(
                plain_text_type("text/plain;charset=UTF-8\ntext/plain\n"),
                Some("text/plain;charset=UTF-8")
            );
            assert_eq!(plain_text_type("TEXT\n"), Some("TEXT"));
            assert_eq!(plain_text_type("image/png\ntext/html\n"), None);
        }

        #[test]
        fn wl_paste_saying_there_is_nothing_reads_as_empty() {
            for message in [
                "Nothing is copied",
                "No selection",
                "No suitable type of content copied",
                r#"Clipboard content is not available as requested type "text". Use "wl-paste --list-types" to view available types."#,
            ] {
                let read = primary(&format!("echo '{message}' >&2; exit 1"));
                assert_eq!(read, Ok(None), "{message}");
            }
            assert_eq!(primary("exit 0"), Ok(None), "an empty selection");
        }

        #[test]
        fn any_other_failure_is_an_error_naming_it() {
            assert_eq!(
                primary(
                    "echo 'The compositor does not seem to support primary selection' >&2; exit 1"
                ),
                Err(
                    "wl-paste failed: The compositor does not seem to support primary selection"
                        .into()
                )
            );
            assert_eq!(
                primary("exit 3"),
                Err("wl-paste failed: exit status: 3".into())
            );
        }

        #[test]
        fn a_hung_owner_is_given_up_on() {
            let dir = tempfile::tempdir().unwrap();
            assert_eq!(
                read(&fake(&dir, "exec sleep 10"), true, TIMEOUT),
                Err("wl-paste: no answer in time".into())
            );
        }

        #[test]
        fn invalid_utf8_is_read_with_replacements() {
            assert_eq!(
                primary(&offering("STRING", r"printf 'caf\351'")),
                Ok(Some("caf\u{fffd}".into()))
            );
        }

        #[test]
        fn a_long_selection_is_read_up_to_the_cap() {
            let read = primary(&offering(
                "UTF8_STRING",
                "head -c 204800 /dev/zero | tr '\\0' a",
            ))
            .unwrap()
            .unwrap();
            assert_eq!(read.len(), TEXT_BYTES);
        }

        const PLAIN_TYPES: &str = r#"printf 'text/plain;charset=utf-8\nUTF8_STRING\nCOMPOUND_TEXT\nTEXT\ntext/plain\nSTRING\nSAVE_TARGETS\n'"#;

        #[test]
        fn the_clipboard_is_read_as_text_after_its_types() {
            let read = clipboard(&format!(
                r#"case "$*" in
--list-types) {PLAIN_TYPES} ;;
"--no-newline --type text/plain;charset=utf-8") printf 'Copied text' ;;
*) exit 9 ;;
esac"#
            ));
            assert_eq!(read, Ok(ClipboardText::Text("Copied text".into())));
        }

        #[test]
        fn a_secret_from_a_password_manager_is_not_read() {
            let read = clipboard(
                r#"case "$*" in
--list-types) printf 'text/plain\nx-kde-passwordManagerHint\n' ;;
*) printf 'hunter2' ;;
esac"#,
            );
            assert_eq!(read, Ok(ClipboardText::Secret));
        }

        #[test]
        fn an_empty_clipboard_is_empty() {
            assert_eq!(
                clipboard("echo 'Nothing is copied' >&2; exit 1"),
                Ok(ClipboardText::Empty)
            );
            let image_only = r#"case "$*" in
--list-types) printf 'image/png\n' ;;
*) echo 'Clipboard content is not available as requested type "text"' >&2; exit 1 ;;
esac"#;
            assert_eq!(clipboard(image_only), Ok(ClipboardText::Empty));
        }

        #[test]
        fn only_the_kde_hint_marks_a_secret() {
            assert!(names_secret("text/plain\nx-kde-passwordManagerHint\n"));
            assert!(!names_secret("text/plain\nUTF8_STRING\n"));
            assert!(!names_secret("x-kde-passwordManagerHint-other\n"));
        }
    }
}

/// Pasteboard types that password managers add to a secret they copy
/// (nspasteboard.org). Clipboard text carrying either is never read aloud.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SECRET_MARKERS: [&str; 2] = [
    "org.nspasteboard.ConcealedType",
    "org.nspasteboard.TransientType",
];

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn marks_secret(types: &[String]) -> bool {
    types.iter().any(|t| SECRET_MARKERS.contains(&t.as_str()))
}

/// Calls `probe` every `interval` until it returns a value, giving up once
/// `timeout` has passed. Always probes at least once.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn poll<T>(
    timeout: Duration,
    interval: Duration,
    mut probe: impl FnMut() -> Option<T>,
) -> Option<T> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(value) = probe() {
            return Some(value);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(interval);
    }
}

/// What to do with the saved clipboard once the copy attempt is over.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum RestoreAction {
    /// The copy never happened, so the clipboard is still the user's own.
    NoCopy,
    Restore,
    /// Something wrote after our read; restoring would overwrite it.
    LeaveAlone,
}

/// `before` is the `changeCount` ahead of the copy, `observed_copy` the count
/// seen when the text was read (if any) and `now` the count after that.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn restore_decision(before: isize, observed_copy: Option<isize>, now: isize) -> RestoreAction {
    match observed_copy {
        Some(copied_at) if copied_at != before => {
            if now == copied_at {
                RestoreAction::Restore
            } else {
                RestoreAction::LeaveAlone
            }
        }
        _ => RestoreAction::NoCopy,
    }
}

/// Selection capture against `NSPasteboard`. Copy detection uses
/// `changeCount`, so the clipboard is never cleared beforehand and a copy of
/// text identical to the clipboard still counts.
#[cfg(target_os = "macos")]
mod pasteboard {
    use super::{
        marks_secret, poll, restore_decision, ClipboardText, RestoreAction, COPY_POLL_INTERVAL,
        COPY_TIMEOUT,
    };
    use log::{debug, info, warn};
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{
        NSPasteboard, NSPasteboardItem, NSPasteboardType, NSPasteboardTypeString,
        NSPasteboardWriting,
    };
    use objc2_foundation::{NSArray, NSData};
    use std::time::Instant;

    /// Every pasteboard item with the data of each of its types. Restoring all
    /// types keeps rich text and files, and the concealed/transient markers
    /// that keep clipboard managers from recording passwords.
    type Snapshot = Vec<Vec<(Retained<NSPasteboardType>, Retained<NSData>)>>;

    /// Most clipboard data held in memory for the restore. A bigger clipboard
    /// is not restored: the copied selection stays on it.
    const SNAPSHOT_CAP_BYTES: usize = 16 * 1024 * 1024;
    /// A clipboard written to while it is read may come back torn, so it is
    /// read once more before giving up on restoring it.
    const SNAPSHOT_ATTEMPTS: usize = 2;

    /// `total + len`, or `None` once that passes the cap.
    pub(super) fn add_within_cap(total: usize, len: usize) -> Option<usize> {
        total
            .checked_add(len)
            .filter(|&sum| sum <= SNAPSHOT_CAP_BYTES)
    }

    /// The snapshot and its size in bytes, or `None` as soon as it passes the cap.
    fn snapshot(pasteboard: &NSPasteboard) -> Option<(Snapshot, usize)> {
        let Some(items) = pasteboard.pasteboardItems() else {
            return Some((Vec::new(), 0));
        };
        let mut total = 0;
        let mut saved = Vec::with_capacity(items.len());
        for item in items.iter() {
            let mut types = Vec::new();
            for data_type in item.types().iter() {
                if let Some(data) = item.dataForType(&data_type) {
                    total = add_within_cap(total, data.len())?;
                    types.push((data_type, data));
                }
            }
            saved.push(types);
        }
        Some((saved, total))
    }

    /// Snapshots the clipboard and returns the `changeCount` the copy is
    /// measured from. `Err` says why there is nothing to restore; the copy
    /// goes ahead either way.
    fn save(pasteboard: &NSPasteboard) -> (isize, Result<Snapshot, String>) {
        let mut count = pasteboard.changeCount();
        for _ in 0..SNAPSHOT_ATTEMPTS {
            let started = Instant::now();
            let saved = snapshot(pasteboard);
            let elapsed = started.elapsed();
            match &saved {
                Some((items, bytes)) => debug!(
                    "Saved the clipboard: {} items, {bytes} bytes in {elapsed:?}",
                    items.len()
                ),
                None => debug!(
                    "Stopped saving the clipboard after {elapsed:?}: over {SNAPSHOT_CAP_BYTES} bytes"
                ),
            }
            let after = pasteboard.changeCount();
            if after == count {
                let saved = saved.map(|(items, _)| items).ok_or_else(|| {
                    format!("it holds over {} MB", SNAPSHOT_CAP_BYTES / (1024 * 1024))
                });
                return (count, saved);
            }
            debug!("Clipboard changed while it was being saved");
            count = after;
        }
        (count, Err("it kept changing while being saved".into()))
    }

    fn restore(pasteboard: &NSPasteboard, snapshot: &Snapshot) {
        pasteboard.clearContents();
        let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = snapshot
            .iter()
            .filter(|types| !types.is_empty())
            .map(|types| {
                let item = NSPasteboardItem::new();
                for (data_type, data) in types {
                    item.setData_forType(data, data_type);
                }
                ProtocolObject::from_retained(item)
            })
            .collect();
        if !items.is_empty() && !pasteboard.writeObjects(&NSArray::from_retained_slice(&items)) {
            warn!("Failed to restore the clipboard after copying the selection");
        }
    }

    pub(super) fn read_text() -> ClipboardText {
        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            let mut types = Vec::new();
            if let Some(items) = pasteboard.pasteboardItems() {
                for item in items.iter() {
                    types.extend(item.types().iter().map(|t| t.to_string()));
                }
            }
            if marks_secret(&types) {
                info!("The clipboard holds a password manager's secret; not reading it");
                return ClipboardText::Secret;
            }
            // SAFETY: NSPasteboardTypeString is a read-only framework static.
            match pasteboard.stringForType(unsafe { NSPasteboardTypeString }) {
                Some(text) => ClipboardText::Text(text.to_string()),
                None => ClipboardText::Empty,
            }
        })
    }

    pub(super) fn capture(
        send_copy: impl FnOnce() -> Result<(), String>,
    ) -> Result<Option<String>, String> {
        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            let (before, saved) = save(&pasteboard);

            send_copy()?;

            // The copy may bump changeCount (clearContents) a moment before the
            // text is written, so keep polling until the text is readable.
            let mut copied_at = None;
            let text = poll(COPY_TIMEOUT, COPY_POLL_INTERVAL, || {
                let count = pasteboard.changeCount();
                if count == before {
                    return None;
                }
                copied_at = Some(count);
                // SAFETY: NSPasteboardTypeString is a read-only framework static.
                let text = pasteboard.stringForType(unsafe { NSPasteboardTypeString })?;
                Some(text.to_string())
            });

            // A later write (the app finishing its copy, or the user copying)
            // wins over our restore.
            match restore_decision(before, copied_at, pasteboard.changeCount()) {
                RestoreAction::NoCopy => {
                    debug!("Copy left the clipboard unchanged: nothing selected");
                    return Ok(None);
                }
                RestoreAction::Restore => match &saved {
                    Ok(snapshot) => restore(&pasteboard, snapshot),
                    Err(reason) => warn!(
                        "Not restoring the clipboard: {reason}. The copied selection stays on it."
                    ),
                },
                RestoreAction::LeaveAlone => {
                    info!("Clipboard changed again after the copy; leaving it as is");
                }
            }
            if text.is_none() {
                debug!("Copied selection has no plain text");
            }
            Ok(text)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn poll_returns_the_first_value_without_waiting_out_the_timeout() {
        let calls = Cell::new(0);
        let value = poll(Duration::from_secs(5), Duration::from_millis(1), || {
            calls.set(calls.get() + 1);
            (calls.get() == 3).then_some("copied")
        });
        assert_eq!(value, Some("copied"));
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn poll_gives_up_after_the_timeout() {
        let calls = Cell::new(0);
        let started = std::time::Instant::now();
        let value: Option<()> = poll(Duration::from_millis(50), Duration::from_millis(10), || {
            calls.set(calls.get() + 1);
            None
        });
        assert_eq!(value, None);
        assert!(started.elapsed() >= Duration::from_millis(50));
        // One probe up front, then at most one per interval; a slow machine only probes less.
        assert!((2..=7).contains(&calls.get()), "{} probes", calls.get());
    }

    #[test]
    fn poll_probes_once_with_a_zero_timeout() {
        let calls = Cell::new(0);
        let value: Option<()> = poll(Duration::ZERO, Duration::from_millis(10), || {
            calls.set(calls.get() + 1);
            None
        });
        assert_eq!(value, None);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn restore_decision_never_restores_when_no_copy_was_observed() {
        assert_eq!(restore_decision(7, None, 7), RestoreAction::NoCopy);
        // Another app wrote during our wait; that is not our copy to undo.
        assert_eq!(restore_decision(7, None, 9), RestoreAction::NoCopy);
    }

    #[test]
    fn restore_decision_ignores_an_observation_equal_to_the_starting_count() {
        assert_eq!(restore_decision(7, Some(7), 7), RestoreAction::NoCopy);
    }

    #[test]
    fn restore_decision_restores_only_when_the_count_has_not_moved_since_our_read() {
        assert_eq!(restore_decision(7, Some(8), 8), RestoreAction::Restore);
        assert_eq!(restore_decision(7, Some(8), 9), RestoreAction::LeaveAlone);
    }

    #[test]
    fn concealed_or_transient_clipboard_text_is_secret() {
        let types = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert!(!marks_secret(&types(&["public.utf8-plain-text"])));
        assert!(!marks_secret(&[]));
        assert!(marks_secret(&types(&[
            "public.utf8-plain-text",
            "org.nspasteboard.ConcealedType"
        ])));
        assert!(marks_secret(&types(&["org.nspasteboard.TransientType"])));
    }

    /// Writes to the real clipboard, so it runs only on request:
    /// `cargo test read_text_ -- --ignored --test-threads=1`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn read_text_reads_plain_text_and_refuses_a_concealed_secret() {
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
        use objc2_foundation::{NSData, NSString};

        let pasteboard = NSPasteboard::generalPasteboard();
        // SAFETY: NSPasteboardTypeString is a read-only framework static.
        let string_type = unsafe { NSPasteboardTypeString };
        let write = |text: &str, concealed: bool| {
            pasteboard.clearContents();
            pasteboard.setString_forType(&NSString::from_str(text), string_type);
            if concealed {
                let concealed_type = NSString::from_str("org.nspasteboard.ConcealedType");
                pasteboard.setData_forType(Some(&NSData::new()), &concealed_type);
            }
        };

        write("The clipboard hotkey works.", false);
        assert_eq!(
            read_text(),
            Ok(ClipboardText::Text(
                "The clipboard hotkey works.".to_string()
            ))
        );
        write("hunter2", true);
        assert_eq!(read_text(), Ok(ClipboardText::Secret));
        pasteboard.clearContents();
        assert_eq!(read_text(), Ok(ClipboardText::Empty));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn only_option_control_and_shift_hold_up_the_copy() {
        use NSEventModifierFlags as Flags;
        assert!(modifiers_clear(Flags::empty()));
        for held in [Flags::Option, Flags::Control, Flags::Shift] {
            assert!(!modifiers_clear(held), "{held:?}");
            assert!(!modifiers_clear(held | Flags::Command), "{held:?} with ⌘");
        }
        assert!(modifiers_clear(Flags::Command));
        assert!(modifiers_clear(Flags::CapsLock));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn modifier_wait_reads_once_when_nothing_is_held() {
        let reads = Cell::new(0);
        let released = wait_for_modifier_release(Duration::from_secs(5), || {
            reads.set(reads.get() + 1);
            NSEventModifierFlags::CapsLock
        });
        assert!(released);
        assert_eq!(reads.get(), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn modifier_wait_ends_as_soon_as_the_keys_are_up() {
        let reads = Cell::new(0);
        let started = std::time::Instant::now();
        let released = wait_for_modifier_release(Duration::from_secs(5), || {
            reads.set(reads.get() + 1);
            if reads.get() < 3 {
                NSEventModifierFlags::Option
            } else {
                NSEventModifierFlags::empty()
            }
        });
        assert!(released);
        assert_eq!(reads.get(), 3);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn modifier_wait_gives_up_after_the_timeout() {
        let reads = Cell::new(0);
        let started = std::time::Instant::now();
        let released = wait_for_modifier_release(Duration::from_millis(50), || {
            reads.set(reads.get() + 1);
            NSEventModifierFlags::Option
        });
        assert!(!released);
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!((2..=7).contains(&reads.get()), "{} reads", reads.get());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn snapshot_size_is_capped_at_16_mb() {
        let cap = 16 * 1024 * 1024;
        assert_eq!(pasteboard::add_within_cap(0, cap), Some(cap));
        assert_eq!(pasteboard::add_within_cap(cap - 10, 10), Some(cap));
        assert_eq!(pasteboard::add_within_cap(cap, 1), None);
        assert_eq!(pasteboard::add_within_cap(1, usize::MAX), None);
    }
}
