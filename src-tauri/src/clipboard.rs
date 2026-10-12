use std::time::Duration;
use tauri::AppHandle;

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
const COPY_TIMEOUT: Duration = Duration::from_millis(200);
const COPY_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Copies the current selection in the frontmost app and returns its text,
/// leaving the clipboard as it was. `Ok(None)` when nothing was copied or the
/// copy had no text.
///
/// Must run on the main thread on macOS (keyboard layout lookup).
#[cfg(target_os = "macos")]
pub fn capture_selection(app_handle: &AppHandle) -> Result<Option<String>, String> {
    pasteboard::capture(|| {
        with_enigo(app_handle, |enigo| {
            crate::input::send_copy(enigo, COPY_HOLD_MS)
        })
    })
}

#[cfg(not(target_os = "macos"))]
pub fn capture_selection(_app_handle: &AppHandle) -> Result<Option<String>, String> {
    Err("selection capture is macOS-only".into())
}

/// Calls `probe` every `interval` until it returns a value, giving up once
/// `timeout` has passed. Always probes at least once.
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
    use super::{poll, restore_decision, RestoreAction, COPY_POLL_INTERVAL, COPY_TIMEOUT};
    use log::{debug, info, warn};
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::ProtocolObject;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSPasteboard, NSPasteboardItem, NSPasteboardType, NSPasteboardTypeString,
        NSPasteboardWriting,
    };
    use objc2_foundation::{NSArray, NSData};

    /// Every pasteboard item with the data of each of its types. Restoring all
    /// types keeps rich text and files, and the concealed/transient markers
    /// that keep clipboard managers from recording passwords.
    type Snapshot = Vec<Vec<(Retained<NSPasteboardType>, Retained<NSData>)>>;

    fn snapshot(pasteboard: &NSPasteboard) -> Snapshot {
        let Some(items) = pasteboard.pasteboardItems() else {
            return Vec::new();
        };
        items
            .iter()
            .map(|item| {
                item.types()
                    .iter()
                    .filter_map(|data_type| {
                        let data = item.dataForType(&data_type)?;
                        Some((data_type, data))
                    })
                    .collect()
            })
            .collect()
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

    pub(super) fn capture(
        send_copy: impl FnOnce() -> Result<(), String>,
    ) -> Result<Option<String>, String> {
        if MainThreadMarker::new().is_none() {
            return Err("Selection capture must run on the main thread".into());
        }

        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            let before = pasteboard.changeCount();
            let saved = snapshot(&pasteboard);

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
                RestoreAction::Restore => restore(&pasteboard, &saved),
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
}
