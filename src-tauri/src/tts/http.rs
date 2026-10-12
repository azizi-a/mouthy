//! OpenAI-compatible speech server client (verified against Kokoro-FastAPI).

use log::debug;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

/// Rate of the headerless s16le mono audio returned for `response_format: "pcm"`.
pub const SAMPLE_RATE: u32 = 24_000;

/// From the 400 Kokoro-FastAPI returns for input with nothing to say, such as "...".
const NO_SPEAKABLE_TEXT: &str = "no speakable text";

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        // System proxies can swallow loopback traffic, and the server is local.
        .no_proxy()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(60))
        .build()
        .expect("failed to build HTTP client")
}

/// PCM rather than WAV: the server streams WAV headers with placeholder sizes.
pub async fn synthesize(
    client: &reqwest::Client,
    base_url: &str,
    voice: &str,
    speed: f32,
    text: &str,
) -> Result<Vec<i16>, String> {
    let request = client
        .post(endpoint(base_url, "/v1/audio/speech"))
        .json(&json!({
            // The OpenAI schema requires `model`; Kokoro-FastAPI accepts any value.
            "model": "kokoro",
            "input": text,
            "voice": voice,
            "response_format": "pcm",
            "speed": speed,
            "stream": false,
        }));
    let body = send(request, base_url).await?;
    pcm_bytes_to_i16(&body)
}

pub async fn list_voices(client: &reqwest::Client, base_url: &str) -> Result<Vec<String>, String> {
    let body = send(client.get(endpoint(base_url, "/v1/audio/voices")), base_url).await?;
    parse_voices(&body)
}

async fn send(request: reqwest::RequestBuilder, base_url: &str) -> Result<Vec<u8>, String> {
    let base_url = trim_base_url(base_url);
    let response = request
        .send()
        .await
        .map_err(|e| request_error(base_url, e))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        debug!("TTS server error body: {}", body);
        return Err(match error_message(&body) {
            Some(message) => format!("TTS server returned {}: {}", status, message),
            None => format!("TTS server returned {}", status),
        });
    }
    let body = response
        .bytes()
        .await
        .map_err(|e| request_error(base_url, e))?;
    Ok(body.to_vec())
}

/// True for the error a sentence such as "..." gets; skip it rather than fail.
pub fn is_unspeakable(error: &str) -> bool {
    error.starts_with("TTS server returned 400") && error.contains(NO_SPEAKABLE_TEXT)
}

#[derive(Deserialize)]
struct ErrorBody {
    detail: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}

/// Kokoro-FastAPI errors look like `{"detail": {"message": "…", …}}`.
fn error_message(body: &str) -> Option<String> {
    serde_json::from_str::<ErrorBody>(body)
        .ok()
        .map(|body| body.detail.message)
}

fn request_error(base_url: &str, error: reqwest::Error) -> String {
    debug!("TTS request to {} failed: {:?}", base_url, error);
    if error.is_builder() {
        format!("Invalid TTS server URL: {}", base_url)
    } else if error.is_connect() {
        format!("Cannot reach TTS server at {}", base_url)
    } else if error.is_timeout() {
        format!("TTS server at {} timed out", base_url)
    } else {
        format!("TTS request failed: {}", error)
    }
}

/// Accepts the base URL with or without the `/v1` that OpenAI clients expect.
fn trim_base_url(base_url: &str) -> &str {
    let base_url = base_url.trim().trim_end_matches('/');
    base_url.strip_suffix("/v1").unwrap_or(base_url)
}

fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}{}", trim_base_url(base_url), path)
}

fn pcm_bytes_to_i16(bytes: &[u8]) -> Result<Vec<i16>, String> {
    let (pairs, remainder) = bytes.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(format!(
            "TTS server returned malformed audio ({} bytes)",
            bytes.len()
        ));
    }
    Ok(pairs.iter().map(|&pair| i16::from_le_bytes(pair)).collect())
}

#[derive(Deserialize)]
struct VoiceList {
    voices: Vec<Voice>,
}

#[derive(Deserialize)]
struct Voice {
    id: String,
}

fn parse_voices(body: &[u8]) -> Result<Vec<String>, String> {
    let list: VoiceList = serde_json::from_slice(body)
        .map_err(|e| format!("TTS server returned an unexpected voice list: {}", e))?;
    let mut ids: Vec<String> = list.voices.into_iter().map(|voice| voice.id).collect();
    ids.sort();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Instant;

    #[test]
    fn pcm_bytes_decode_little_endian() {
        assert_eq!(
            pcm_bytes_to_i16(&[0x01, 0x00, 0xff, 0xff, 0x00, 0x80, 0xff, 0x7f]),
            Ok(vec![1, -1, i16::MIN, i16::MAX])
        );
        assert_eq!(pcm_bytes_to_i16(&[]), Ok(vec![]));
    }

    #[test]
    fn pcm_bytes_reject_odd_length() {
        assert!(pcm_bytes_to_i16(&[0x01, 0x00, 0x02]).is_err());
    }

    #[test]
    fn endpoint_joins_with_or_without_trailing_slash() {
        let expected = "http://127.0.0.1:8880/v1/audio/speech";
        assert_eq!(
            endpoint("http://127.0.0.1:8880", "/v1/audio/speech"),
            expected
        );
        assert_eq!(
            endpoint("http://127.0.0.1:8880/", "/v1/audio/speech"),
            expected
        );
        assert_eq!(
            endpoint(" http://127.0.0.1:8880// ", "/v1/audio/speech"),
            expected
        );
        assert_eq!(
            endpoint("http://127.0.0.1:8880/v1/", "/v1/audio/speech"),
            expected
        );
    }

    #[test]
    fn error_message_reads_detail_message() {
        let body = r#"{"detail":{"error":"validation_error","message":"Input contains no speakable text","type":"invalid_request_error"}}"#;
        assert_eq!(
            error_message(body),
            Some("Input contains no speakable text".to_string())
        );
        // FastAPI's own 422s carry a list in `detail`.
        assert_eq!(
            error_message(r#"{"detail":[{"msg":"field required"}]}"#),
            None
        );
        assert_eq!(error_message("Internal Server Error"), None);
    }

    #[test]
    fn unspeakable_only_for_a_400_with_that_message() {
        assert!(is_unspeakable(
            "TTS server returned 400 Bad Request: Input contains no speakable text"
        ));
        assert!(!is_unspeakable(
            "TTS server returned 400 Bad Request: Voice 'x' not found"
        ));
        assert!(!is_unspeakable(
            "TTS server returned 500 Internal Server Error: no speakable text"
        ));
    }

    #[test]
    fn voices_parse_to_sorted_ids() {
        let body = br#"{"voices":[{"id":"bm_lewis","name":"bm_lewis"},{"id":"af_alloy","target_quality":"B"}]}"#;
        assert_eq!(
            parse_voices(body),
            Ok(vec!["af_alloy".to_string(), "bm_lewis".to_string()])
        );
    }

    #[test]
    fn voices_reject_unexpected_shape() {
        assert!(parse_voices(br#"["af_alloy","bm_lewis"]"#).is_err());
        assert!(parse_voices(b"not json").is_err());
    }

    #[test]
    fn unreachable_server_reports_base_url() {
        // A port the OS just handed out and released is closed, so the connect fails fast.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let base_url = format!("http://127.0.0.1:{}", port);
        let result = tauri::async_runtime::block_on(synthesize(
            &client(),
            &format!("{}/", base_url),
            "bm_lewis",
            1.0,
            "Hello.",
        ));
        assert_eq!(
            result,
            Err(format!("Cannot reach TTS server at {}", base_url))
        );
    }

    #[test]
    #[ignore = "needs Kokoro-FastAPI at 127.0.0.1:8880"]
    fn synthesize_against_local_server() {
        const BASE_URL: &str = "http://127.0.0.1:8880";
        let client = client();

        let voices = tauri::async_runtime::block_on(list_voices(&client, BASE_URL)).unwrap();
        assert!(voices.iter().any(|voice| voice == "bm_lewis"));

        let started = Instant::now();
        let samples = tauri::async_runtime::block_on(synthesize(
            &client,
            BASE_URL,
            "bm_lewis",
            1.0,
            "Hello from Mouthy, this is a short test sentence.",
        ))
        .unwrap();
        let seconds = samples.len() as f32 / SAMPLE_RATE as f32;
        println!(
            "{} samples ({:.2} s of audio) in {:?}",
            samples.len(),
            seconds,
            started.elapsed()
        );
        // About 3 s of speech; a wrong rate or format would land far outside.
        assert!((1.0..8.0).contains(&seconds), "{} s of audio", seconds);
        assert!(samples.iter().any(|&s| s != 0));
    }
}
