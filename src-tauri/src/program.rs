//! Separately installed command-line programs: finding one, and running it
//! with a deadline.

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

/// Error output kept; the rest is read and dropped, so the program never
/// blocks on a full pipe.
const STDERR_BYTES: usize = 4096;
const POLL: Duration = Duration::from_millis(2);

/// Path variables an AppImage's launcher points into the image. A host
/// program started with them would load the image's libraries (its
/// `libwayland-client`, say) and data instead of its own.
const IMAGE_PATHS: &[&str] = &[
    "PATH",
    "LD_LIBRARY_PATH",
    "XDG_DATA_DIRS",
    "PYTHONPATH",
    "PYTHONHOME",
    "PERLLIB",
    "GSETTINGS_SCHEMA_DIR",
    "QT_PLUGIN_PATH",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
    "GTK_DATA_PREFIX",
    "GTK_EXE_PREFIX",
    "GTK_PATH",
    "GTK_IM_MODULE_FILE",
    "GDK_PIXBUF_MODULE_FILE",
    "GI_TYPELIB_PATH",
    "GIO_MODULE_DIR",
    "GIO_EXTRA_MODULES",
];
/// Settings the AppImage's launcher makes for Mouthy's own GTK and Python.
const IMAGE_SETTINGS: &[&str] = &["PYTHONDONTWRITEBYTECODE", "GTK_THEME", "GDK_BACKEND"];

/// The first executable `program` in `known` (absolute paths, checked first:
/// an app opened from a desktop menu has a shorter `PATH`), then on `path_var`.
pub fn find(known: &[&str], program: &str, path_var: Option<OsString>) -> Option<PathBuf> {
    known
        .iter()
        .map(PathBuf::from)
        .chain(
            path_var
                .iter()
                .flat_map(std::env::split_paths)
                .map(|dir| dir.join(program)),
        )
        .find(|path| is_executable(path))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// What a program printed.
#[derive(Debug)]
pub struct Output {
    /// `None` when it was stopped after printing more than the bytes asked for.
    pub status: Option<ExitStatus>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs `program` without a shell or stdin and returns its output, killing it
/// once `timeout` has passed. Output is read while it runs, since a program
/// that fills a pipe's buffer (64 KiB on Linux) waits for it to be read. Past
/// `max_bytes` of output the program is stopped and the first `max_bytes`
/// returned. Only `program` itself is killed: a helper it started that holds
/// the output open can outlive it. From an AppImage, it runs with the host's
/// environment, not the image's.
pub fn run(
    program: &Path,
    args: &[&str],
    timeout: Duration,
    max_bytes: usize,
) -> Result<Output, String> {
    let deadline = Instant::now() + timeout;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(appdir) = std::env::var_os("APPDIR").filter(|dir| !dir.is_empty()) {
        for (name, value) in host_environment(Path::new(&appdir), |name| std::env::var_os(name)) {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", program.display()))?;
    let output = collect(&mut child, deadline, max_bytes);
    if output.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    output.map_err(|e| match e {
        Failure::Late => "no answer in time".to_string(),
        Failure::Other(e) => e,
    })
}

/// What a host program's environment needs changed when Mouthy runs from an
/// AppImage mounted at `appdir`, given how `var` reads Mouthy's own: each
/// path variable without its entries in the image (`None`, removed, when none
/// is left) and the launcher's settings removed.
fn host_environment(
    appdir: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> Vec<(&'static str, Option<OsString>)> {
    // The mount may be reached through a symlink, as a symlinked TMPDIR is.
    let real_appdir = std::fs::canonicalize(appdir).ok();
    let in_image = |entry: &Path| {
        entry.starts_with(appdir)
            || real_appdir
                .as_deref()
                .is_some_and(|real| entry.starts_with(real))
    };
    let mut changes = Vec::new();
    for &name in IMAGE_PATHS {
        let Some(value) = var(name) else {
            continue;
        };
        let entries: Vec<PathBuf> = std::env::split_paths(&value).collect();
        let kept: Vec<&PathBuf> = entries
            .iter()
            .filter(|entry| !entry.as_os_str().is_empty() && !in_image(entry))
            .collect();
        if kept.len() == entries.len() {
            continue;
        }
        let value = (!kept.is_empty())
            .then(|| std::env::join_paths(kept).ok())
            .flatten();
        changes.push((name, value));
    }
    for &name in IMAGE_SETTINGS {
        if var(name).is_some() {
            changes.push((name, None));
        }
    }
    changes
}

enum Failure {
    Late,
    Other(String),
}

fn collect(child: &mut Child, deadline: Instant, max_bytes: usize) -> Result<Output, Failure> {
    let stdout = child
        .stdout
        .take()
        .map(|out| read_up_to(out, max_bytes, false));
    let stderr = child
        .stderr
        .take()
        .map(|err| read_up_to(err, STDERR_BYTES, true));
    let (stdout, cut_short) = received(stdout, deadline)?;
    if cut_short {
        let _ = child.kill();
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) => return Err(Failure::Late),
            Err(e) => return Err(Failure::Other(format!("cannot wait for it: {e}"))),
        }
    };
    // Output cut short is what was asked for: a helper still holding the
    // error output open must not turn it into a timeout.
    let stderr = if cut_short {
        received_so_far(stderr)
    } else {
        received(stderr, deadline)?.0
    };
    Ok(Output {
        status: (!cut_short).then_some(status),
        stdout,
        stderr,
    })
}

type Reading = Receiver<io::Result<(Vec<u8>, bool)>>;

/// What a reader has finished reading, without waiting for it.
fn received_so_far(reading: Option<io::Result<Reading>>) -> Vec<u8> {
    reading
        .and_then(Result::ok)
        .and_then(|reading| reading.try_recv().ok())
        .and_then(Result::ok)
        .map(|(bytes, _)| bytes)
        .unwrap_or_default()
}

/// Reads `pipe` on its own thread: up to `max_bytes`, saying whether there
/// was more, then to the end only if `drain`.
fn read_up_to(
    mut pipe: impl Read + Send + 'static,
    max_bytes: usize,
    drain: bool,
) -> io::Result<Reading> {
    let (send, reading) = mpsc::channel();
    thread::Builder::new()
        .name("program-output".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let read = pipe
                .by_ref()
                .take(max_bytes as u64 + 1)
                .read_to_end(&mut bytes)
                .and_then(|_| {
                    let more = bytes.len() > max_bytes;
                    bytes.truncate(max_bytes);
                    if more && drain {
                        io::copy(&mut pipe, &mut io::sink())?;
                    }
                    Ok((bytes, more))
                });
            let _ = send.send(read);
        })?;
    Ok(reading)
}

/// The bytes a reader got and whether it stopped short, waiting no later
/// than `deadline`. A pipe that was never opened reads as empty.
fn received(
    reading: Option<io::Result<Reading>>,
    deadline: Instant,
) -> Result<(Vec<u8>, bool), Failure> {
    let reading = match reading {
        None => return Ok((Vec::new(), false)),
        Some(reading) => {
            reading.map_err(|e| Failure::Other(format!("cannot read its output: {e}")))?
        }
    };
    match reading.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(read) => read.map_err(|e| Failure::Other(format!("cannot read its output: {e}"))),
        Err(RecvTimeoutError::Timeout) => Err(Failure::Late),
        Err(RecvTimeoutError::Disconnected) => {
            Err(Failure::Other("its output reader stopped".into()))
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn sh(script: &str, timeout: Duration, max_bytes: usize) -> Result<Output, String> {
        run(Path::new("/bin/sh"), &["-c", script], timeout, max_bytes)
    }

    #[test]
    fn output_past_a_pipe_buffer_is_read_while_the_program_runs() {
        // 200 KiB: more than a pipe holds, so reading only after the exit
        // would hang until the deadline.
        let started = Instant::now();
        let output = sh(
            "head -c 204800 /dev/zero | tr '\\0' a",
            Duration::from_secs(10),
            1 << 20,
        )
        .unwrap();
        assert!(output.status.unwrap().success());
        assert_eq!(output.stdout.len(), 204_800);
        assert!(output.stdout.iter().all(|&b| b == b'a'));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn output_past_the_cap_stops_the_program() {
        let started = Instant::now();
        let output = sh("yes 0123456789", Duration::from_secs(10), 100_000).unwrap();
        assert_eq!(output.status, None, "stopped, not exited");
        assert_eq!(output.stdout.len(), 100_000);
        assert!(output.stdout.starts_with(b"0123456789\n0123456789\n"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn output_cut_short_is_kept_while_a_helper_holds_the_pipes() {
        // The background sleep keeps stdout and stderr open after `yes` is
        // stopped, so waiting for either to close would run into the deadline.
        let started = Instant::now();
        let output = sh(
            "sleep 5 & exec yes 0123456789",
            Duration::from_secs(10),
            100_000,
        )
        .unwrap();
        assert_eq!(output.status, None);
        assert_eq!(output.stdout.len(), 100_000);
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "did not wait for the helper"
        );
    }

    #[test]
    fn output_of_exactly_the_cap_is_whole() {
        let output = sh("printf abcd", Duration::from_secs(10), 4).unwrap();
        assert!(output.status.unwrap().success());
        assert_eq!(output.stdout, b"abcd");
    }

    #[test]
    fn a_program_that_never_answers_is_killed_at_the_deadline() {
        let started = Instant::now();
        let error = run(
            Path::new("/bin/sleep"),
            &["10"],
            Duration::from_millis(200),
            1024,
        )
        .unwrap_err();
        assert_eq!(error, "no answer in time");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn exit_status_and_error_output_are_kept() {
        let output = sh(
            "printf partial; echo 'Nothing is copied' >&2; exit 1",
            Duration::from_secs(10),
            1024,
        )
        .unwrap();
        assert_eq!(output.status.unwrap().code(), Some(1));
        assert_eq!(output.stdout, b"partial");
        assert_eq!(output.stderr, b"Nothing is copied\n");
    }

    #[test]
    fn long_error_output_is_cut_without_blocking_the_program() {
        let output = sh(
            "head -c 204800 /dev/zero >&2; printf done",
            Duration::from_secs(10),
            1024,
        )
        .unwrap();
        assert!(output.status.unwrap().success());
        assert_eq!(output.stdout, b"done");
        assert_eq!(output.stderr.len(), STDERR_BYTES);
    }

    #[test]
    fn a_missing_program_is_an_error() {
        let error = run(
            Path::new("/nonexistent/wl-paste"),
            &[],
            Duration::from_secs(1),
            1024,
        )
        .unwrap_err();
        assert!(
            error.starts_with("cannot start /nonexistent/wl-paste"),
            "{error}"
        );
    }

    #[test]
    fn a_host_program_gets_no_appimage_paths_or_settings() {
        let appdir = Path::new("/tmp/.mount_Mouthy1");
        let vars = [
            (
                "PATH",
                "/tmp/.mount_Mouthy1/usr/bin/:/tmp/.mount_Mouthy1/usr/sbin/:/usr/local/bin:/usr/bin",
            ),
            (
                "LD_LIBRARY_PATH",
                "/tmp/.mount_Mouthy1/usr/lib/:/tmp/.mount_Mouthy1/usr/lib/x86_64-linux-gnu/:",
            ),
            (
                "XDG_DATA_DIRS",
                "/tmp/.mount_Mouthy1/usr/share:/usr/share:/home/me/.local/share/flatpak/exports/share",
            ),
            ("GTK_PATH", "/tmp/.mount_Mouthy1/usr/lib/gtk-3.0"),
            ("GI_TYPELIB_PATH", "/opt/mine/typelibs"),
            ("QT_PLUGIN_PATH", "/tmp/.mount_Mouthy10/plugins"),
            ("GDK_BACKEND", "x11"),
            ("PYTHONDONTWRITEBYTECODE", "1"),
            ("WAYLAND_DISPLAY", "wayland-0"),
        ];
        let var = |name: &str| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        };
        let changes = host_environment(appdir, var);
        let set = |value: &str| Some(OsString::from(value));
        assert_eq!(
            changes,
            vec![
                ("PATH", set("/usr/local/bin:/usr/bin")),
                ("LD_LIBRARY_PATH", None),
                (
                    "XDG_DATA_DIRS",
                    set("/usr/share:/home/me/.local/share/flatpak/exports/share")
                ),
                ("GTK_PATH", None),
                ("PYTHONDONTWRITEBYTECODE", None),
                ("GDK_BACKEND", None),
            ],
            "the user's own paths, another mount's and WAYLAND_DISPLAY stay"
        );
        assert_eq!(host_environment(appdir, |_| None), vec![]);
    }

    #[test]
    fn an_appdir_reached_through_a_symlink_is_recognised() {
        let dir = tempfile::tempdir().unwrap();
        let mount = dir.path().join("mount");
        fs::create_dir(&mount).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&mount, &link).unwrap();
        let real_bin = fs::canonicalize(&mount).unwrap().join("usr/bin");
        let path = std::env::join_paths([real_bin.as_path(), Path::new("/usr/bin")]).unwrap();
        let changes = host_environment(&link, |name| (name == "PATH").then(|| path.clone()));
        assert_eq!(changes, vec![("PATH", Some(OsString::from("/usr/bin")))]);
    }

    #[test]
    fn find_checks_known_paths_then_path_and_skips_what_cannot_run() {
        let dir = tempfile::tempdir().unwrap();
        let make = |name: &str, mode: u32| {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "#!/bin/sh\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            path
        };
        let known = make("known/wl-paste", 0o755);
        let on_path = make("bin/wl-paste", 0o755);
        make("plain/wl-paste", 0o644);
        let path_var =
            std::env::join_paths([dir.path().join("plain"), dir.path().join("bin")]).ok();
        let known_str = known.to_str().unwrap();
        let missing = dir.path().join("missing/wl-paste");

        assert_eq!(
            find(&[known_str], "wl-paste", path_var.clone()),
            Some(known.clone())
        );
        assert_eq!(
            find(&[missing.to_str().unwrap()], "wl-paste", path_var.clone()),
            Some(on_path),
            "not executable on PATH is skipped"
        );
        assert_eq!(find(&[], "wl-paste", None), None);
    }
}
