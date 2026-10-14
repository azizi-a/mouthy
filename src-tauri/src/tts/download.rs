//! Resumable, sha256-checked download of one model file.

use log::info;
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// No bytes for this long means the transfer is wedged, not slow. The partial
/// file stays for the next attempt to resume.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// A client for the model host. Unlike the speech server's, it honours the
/// system proxy and has no overall timeout: a large file on a slow link is fine.
pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(STALL_TIMEOUT)
        .build()
        .map_err(|e| format!("Cannot start a download: {e}"))
}

/// Where a download collects its bytes until they check out. The suffix keeps
/// it out of the voice list and the missing-files check.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

/// Fetches `url` into `dest` through its `.part` file, resuming one an earlier
/// attempt left when the server honours ranges. `dest` appears, by rename, only
/// once the bytes match `size` and `sha256`. `progress` gets the byte count at
/// most every 100 ms and at the end. Dropping the future keeps the partial.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    size: u64,
    sha256: &str,
    progress: &(dyn Fn(u64) + Send + Sync),
) -> Result<(), String> {
    let part = part_path(dest);
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Cannot create the folder {}: {e}", dir.display()))?;
    }
    let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > size {
        remove_if_present(&part)?;
        have = 0;
    }
    // A partial already full-size means an attempt ended between the last
    // byte and the rename; it only needs checking.
    if have < size {
        fetch(client, url, &part, have, size, progress).await?;
    } else {
        progress(size);
    }
    verify(&part, sha256).await?;
    fs::rename(&part, dest).map_err(|e| format!("Cannot move the download into place: {e}"))
}

/// Appends the rest of the file to `part`, which holds `from` bytes.
async fn fetch(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    from: u64,
    size: u64,
    progress: &(dyn Fn(u64) + Send + Sync),
) -> Result<(), String> {
    let mut request = client.get(url);
    if from > 0 {
        info!("Resuming {} from byte {from}", part.display());
        request = request.header(RANGE, format!("bytes={from}-"));
    }
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Cannot reach the download server: {e}"))?;
    let status = response.status();
    let mut from = from;
    if from > 0 {
        match status {
            StatusCode::PARTIAL_CONTENT => {
                let starts_at = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(content_range_start);
                if starts_at != Some(from) {
                    remove_if_present(part)?;
                    return Err(format!(
                        "The server resumed at the wrong place ({starts_at:?}, not {from}); try again"
                    ));
                }
            }
            // The server ignored the range and is sending the whole file.
            StatusCode::OK => from = 0,
            // Partials are always shorter than `size`, so the server's file is too.
            StatusCode::RANGE_NOT_SATISFIABLE => {
                remove_if_present(part)?;
                return Err("The server's file is smaller than expected".into());
            }
            _ => {}
        }
    }
    if !status.is_success() {
        return Err(format!("The download server returned {status}"));
    }
    if let Some(len) = response.content_length() {
        if from + len != size {
            return Err(format!(
                "The server offers {} bytes, expected {size}",
                from + len
            ));
        }
    }

    let mut file = if from > 0 {
        OpenOptions::new().append(true).open(part)
    } else {
        File::create(part)
    }
    .map_err(|e| format!("Cannot write {}: {e}", part.display()))?;
    let mut downloaded = from;
    progress(downloaded);
    let mut reported = Instant::now();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("The download stopped: {e}"))?
    {
        // A server that keeps sending must not fill the disk.
        if downloaded + chunk.len() as u64 > size {
            drop(file);
            remove_if_present(part)?;
            return Err(format!(
                "The server sent more than the expected {size} bytes"
            ));
        }
        file.write_all(&chunk)
            .map_err(|e| format!("Cannot write {}: {e}", part.display()))?;
        downloaded += chunk.len() as u64;
        if reported.elapsed() >= PROGRESS_INTERVAL {
            progress(downloaded);
            reported = Instant::now();
        }
    }
    // On disk before the rename, so a crash cannot leave a named file of zeros.
    file.sync_all()
        .map_err(|e| format!("Cannot write {}: {e}", part.display()))?;
    progress(downloaded);
    if downloaded < size {
        return Err(format!(
            "The download ended early ({downloaded} of {size} bytes); try again to resume"
        ));
    }
    Ok(())
}

/// Checks `part` against `expected`, deleting it on a mismatch so the next
/// attempt starts clean.
async fn verify(part: &Path, expected: &str) -> Result<(), String> {
    let path = part.to_path_buf();
    let actual = tauri::async_runtime::spawn_blocking(move || sha256_file(&path))
        .await
        .map_err(|e| format!("Checking the download failed: {e}"))?
        .map_err(|e| format!("Cannot read the download: {e}"))?;
    if actual != expected {
        remove_if_present(part)?;
        return Err("The downloaded file is damaged; try again".into());
    }
    Ok(())
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Start offset of a `Content-Range: bytes <start>-<end>/<total>` header.
fn content_range_start(value: &str) -> Option<u64> {
    let range = value.trim().strip_prefix("bytes")?.trim_start();
    range.split('-').next()?.trim().parse().ok()
}

pub fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            Err(format!("Cannot delete {}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::future::{AbortHandle, Abortable};
    use std::io::BufRead;
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    const BODY: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

    fn sha_hex(data: &[u8]) -> String {
        format!("{:x}", Sha256::digest(data))
    }

    fn test_client(stall: Duration) -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .read_timeout(stall)
            .build()
            .unwrap()
    }

    /// One scripted reply: what to write, and whether to then hold the
    /// connection open without sending more.
    struct Reply {
        bytes: Vec<u8>,
        stall: bool,
    }

    fn reply(status: &str, headers: &[String], body: &[u8]) -> Reply {
        let mut bytes = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
        for header in headers {
            bytes.push_str(header);
            bytes.push_str("\r\n");
        }
        bytes.push_str("\r\n");
        let mut bytes = bytes.into_bytes();
        bytes.extend_from_slice(body);
        Reply {
            bytes,
            stall: false,
        }
    }

    fn ok(body: &[u8]) -> Reply {
        reply("200 OK", &[format!("Content-Length: {}", body.len())], body)
    }

    fn read_head(stream: &TcpStream) -> String {
        let mut reader = io::BufReader::new(stream);
        let mut head = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                return head.to_lowercase();
            }
            head.push_str(&line);
        }
    }

    /// Serves one connection per reply, in order. Joins to the request heads.
    fn serve(replies: Vec<Reply>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut heads = Vec::new();
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                heads.push(read_head(&stream));
                let _ = stream.write_all(&reply.bytes);
                if reply.stall {
                    thread::sleep(Duration::from_secs(2));
                }
            }
            heads
        });
        (url, handle)
    }

    fn run(url: &str, dest: &Path, sha256: &str) -> Result<(), String> {
        tauri::async_runtime::block_on(download(
            &test_client(STALL_TIMEOUT),
            url,
            dest,
            BODY.len() as u64,
            sha256,
            &|_| {},
        ))
    }

    fn dest_in(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("voices").join("bm_lewis.bin")
    }

    #[test]
    fn part_name_keeps_downloads_out_of_the_voice_list() {
        assert_eq!(
            part_path(Path::new("/m/voices/bm_lewis.bin")),
            Path::new("/m/voices/bm_lewis.bin.part")
        );
        assert_eq!(
            part_path(Path::new("/m/int8.onnx")),
            Path::new("/m/int8.onnx.part")
        );
    }

    #[test]
    fn content_range_start_parses_the_offset() {
        assert_eq!(content_range_start("bytes 1000-522239/522240"), Some(1000));
        assert_eq!(content_range_start("bytes */522240"), None);
        assert_eq!(content_range_start("items 0-1/2"), None);
    }

    #[test]
    fn fresh_download_is_verified_then_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        let (url, server) = serve(vec![ok(BODY)]);

        run(&url, &dest, &sha_hex(BODY)).unwrap();

        assert_eq!(fs::read(&dest).unwrap(), BODY);
        assert!(!part_path(&dest).exists());
        assert!(!server.join().unwrap()[0].contains("range:"));
    }

    #[test]
    fn resume_asks_for_the_rest_and_appends_it() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(part_path(&dest), &BODY[..10]).unwrap();
        let rest = &BODY[10..];
        let (url, server) = serve(vec![reply(
            "206 Partial Content",
            &[
                format!("Content-Length: {}", rest.len()),
                format!("Content-Range: bytes 10-{}/{}", BODY.len() - 1, BODY.len()),
            ],
            rest,
        )]);

        run(&url, &dest, &sha_hex(BODY)).unwrap();

        assert_eq!(fs::read(&dest).unwrap(), BODY);
        assert!(server.join().unwrap()[0].contains("range: bytes=10-"));
    }

    #[test]
    fn a_server_ignoring_the_range_starts_the_file_over() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(part_path(&dest), b"XXXXX").unwrap();
        let (url, _server) = serve(vec![ok(BODY)]);

        run(&url, &dest, &sha_hex(BODY)).unwrap();

        assert_eq!(fs::read(&dest).unwrap(), BODY);
    }

    #[test]
    fn a_resume_at_the_wrong_offset_discards_the_partial() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(part_path(&dest), &BODY[..10]).unwrap();
        let (url, _server) = serve(vec![reply(
            "206 Partial Content",
            &[
                format!("Content-Length: {}", BODY.len() - 10),
                format!("Content-Range: bytes 0-{}/{}", BODY.len() - 11, BODY.len()),
            ],
            &BODY[..BODY.len() - 10],
        )]);

        assert!(run(&url, &dest, &sha_hex(BODY)).is_err());
        assert!(!part_path(&dest).exists());
        assert!(!dest.exists());
    }

    #[test]
    fn range_not_satisfiable_discards_the_partial() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(part_path(&dest), &BODY[..10]).unwrap();
        let (url, _server) = serve(vec![reply(
            "416 Range Not Satisfiable",
            &["Content-Length: 0".into()],
            b"",
        )]);

        assert!(run(&url, &dest, &sha_hex(BODY)).is_err());
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn a_full_size_partial_is_checked_without_a_request() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(part_path(&dest), BODY).unwrap();

        // Nothing listens on port 9: any request would fail.
        run("http://127.0.0.1:9/file", &dest, &sha_hex(BODY)).unwrap();

        assert_eq!(fs::read(&dest).unwrap(), BODY);
    }

    #[test]
    fn an_oversized_partial_starts_over() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(part_path(&dest), [BODY, BODY].concat()).unwrap();
        let (url, server) = serve(vec![ok(BODY)]);

        run(&url, &dest, &sha_hex(BODY)).unwrap();

        assert_eq!(fs::read(&dest).unwrap(), BODY);
        assert!(!server.join().unwrap()[0].contains("range:"));
    }

    #[test]
    fn a_wrong_checksum_installs_nothing_and_clears_the_partial() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        let tampered = BODY.to_ascii_uppercase();
        let (url, _server) = serve(vec![ok(&tampered)]);

        let error = run(&url, &dest, &sha_hex(BODY)).unwrap_err();

        assert!(error.contains("damaged"), "{error}");
        assert!(!dest.exists());
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn a_wrong_advertised_size_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        let (url, _server) = serve(vec![ok(&BODY[..20])]);

        assert!(run(&url, &dest, &sha_hex(BODY)).is_err());
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn a_server_that_keeps_sending_is_cut_off() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        // No Content-Length: the body runs until the connection closes.
        let (url, _server) = serve(vec![reply("200 OK", &[], &[BODY, BODY].concat())]);

        let error = run(&url, &dest, &sha_hex(BODY)).unwrap_err();

        assert!(error.contains("more than"), "{error}");
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn a_stalled_transfer_times_out_and_keeps_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        let mut first = reply(
            "200 OK",
            &[format!("Content-Length: {}", BODY.len())],
            &BODY[..10],
        );
        first.stall = true;
        let (url, _server) = serve(vec![first]);

        let result = tauri::async_runtime::block_on(download(
            &test_client(Duration::from_millis(300)),
            &url,
            &dest,
            BODY.len() as u64,
            &sha_hex(BODY),
            &|_| {},
        ));

        assert!(result.is_err());
        assert_eq!(fs::read(part_path(&dest)).unwrap(), &BODY[..10]);
        assert!(!dest.exists());
    }

    #[test]
    fn an_aborted_download_keeps_its_partial_for_a_resume() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dest_in(&dir);
        let mut first = reply(
            "200 OK",
            &[format!("Content-Length: {}", BODY.len())],
            &BODY[..10],
        );
        first.stall = true;
        let rest = &BODY[10..];
        let second = reply(
            "206 Partial Content",
            &[
                format!("Content-Length: {}", rest.len()),
                format!("Content-Range: bytes 10-{}/{}", BODY.len() - 1, BODY.len()),
            ],
            rest,
        );
        let (url, _server) = serve(vec![first, second]);
        let client = test_client(STALL_TIMEOUT);

        let (abort, registration) = AbortHandle::new_pair();
        // Cancel once the first bytes are on disk, while the server stalls.
        let canceller = {
            let part = part_path(&dest);
            thread::spawn(move || {
                while fs::metadata(&part).map_or(0, |m| m.len()) < 10 {
                    thread::sleep(Duration::from_millis(5));
                }
                abort.abort();
                Instant::now()
            })
        };
        let result = tauri::async_runtime::block_on(Abortable::new(
            download(
                &client,
                &url,
                &dest,
                BODY.len() as u64,
                &sha_hex(BODY),
                &|_| {},
            ),
            registration,
        ));
        let aborted_at = canceller.join().unwrap();
        assert!(result.is_err(), "aborted");
        assert!(
            aborted_at.elapsed() < Duration::from_millis(500),
            "no wait for a timeout"
        );
        assert_eq!(fs::read(part_path(&dest)).unwrap(), &BODY[..10]);
        assert!(!dest.exists());

        run(&url, &dest, &sha_hex(BODY)).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), BODY);
    }
}
