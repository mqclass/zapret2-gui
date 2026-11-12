//! Clash API client for the sing-box sidecar (WARP mode, stage 3).
//!
//! The engine exposes its Clash API on a random loopback port with a
//! random secret (both chosen by the config generator, stage 2, and
//! passed here - never re-read from the config file). This client is
//! the manager's only view into the engine's health:
//!
//! - `alive()` - `GET /version`: the engine process is up and its
//!   HTTP stack answers. This is the readiness signal: the manager
//!   waits for it after spawn instead of sleeping a fixed delay
//!   (NETZapret's `ClashApi.AliveAsync`).
//! - `measure_exit()` - `GET /proxies/{tag}/delay`: latency of one
//!   exit through the WARP outbound. A working exit answers with
//!   `{"delay": <ms>}`; a dead exit times out or errors; an
//!   unregistered engine answers with "tunnel not initialized".
//!   The warmup and the watchdog (stage 6) judge the exit from this.
//!
//! The selector (`PUT /proxies/{tag}`) and mode switch
//! (`PATCH /configs`) from the design's stage 6 are not needed yet:
//! the v1 config has no selector group, and the tunnel scope is fixed
//! at generation time. They are added when stage 6 needs them.
//!
//! The client never touches the network outside localhost: the base
//! URL is always `http://127.0.0.1:<port>`.

use std::time::Duration;

use thiserror::Error;

/// Errors from the Clash API client.
#[derive(Debug, Error)]
pub enum ClashApiError {
    /// The HTTP request itself failed (connection refused, timeout,
    /// DNS - for a localhost API this almost always means the engine
    /// is not up yet or died).
    #[error("clash api request failed: {0}")]
    Transport(#[from] reqwest::Error),

    /// The engine answered with a non-2xx status: the exit is down,
    /// the proxy does not exist, or the tunnel is not initialized.
    /// The message is the engine's own `message` field when present.
    #[error("exit measurement failed: {0}")]
    ExitFailed(String),

    /// The engine answered 2xx but the body is not the expected JSON
    /// (`{"delay": <ms>}`).
    #[error("unexpected clash api response: {0}")]
    BadResponse(String),
}

/// Client for one sing-box Clash API endpoint.
///
/// Built per engine start: the port and secret come from the generated
/// config (`GeneratedSingBoxConfig`), so a regenerated config (stage 3
/// TOCTOU retry) gets a fresh client.
#[derive(Debug, Clone)]
pub struct ClashApiClient {
    base: String,
    secret: String,
    client: reqwest::Client,
}

impl ClashApiClient {
    /// Build a client for `http://127.0.0.1:<port>` with the given
    /// Bearer secret. No request is made here.
    pub fn new(port: u16, secret: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base: format!("http://127.0.0.1:{port}"),
            secret: secret.to_string(),
            client,
        }
    }

    /// Is the engine up? `GET /version` with a short per-request
    /// timeout - the readiness loop calls this every 250 ms, so a
    /// single call must never hang the loop.
    pub async fn alive(&self) -> bool {
        match self
            .client
            .get(format!("{}/version", self.base))
            .header("Authorization", format!("Bearer {}", self.secret))
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }

    /// Measure the latency of one exit (`GET /proxies/{tag}/delay`).
    ///
    /// `timeout` is the engine-side measurement deadline (the `timeout`
    /// query parameter); the HTTP request itself gets `timeout` plus a
    /// 5 s grace so a slow engine answer is not cut off by our side.
    /// `url` is the probe target (a tiny 204 endpoint).
    pub async fn measure_exit(
        &self,
        tag: &str,
        timeout: Duration,
        url: &str,
    ) -> Result<u64, ClashApiError> {
        let request_url = delay_url(&self.base, tag, timeout, url);
        let response = self
            .client
            .get(request_url)
            .header("Authorization", format!("Bearer {}", self.secret))
            .timeout(timeout + Duration::from_secs(5))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if status.is_success() {
            parse_delay_response(&body)
        } else {
            let message = extract_error_message(&body).unwrap_or_else(|| format!("HTTP {status}"));
            Err(ClashApiError::ExitFailed(message))
        }
    }
}

/// Build the delay-measurement URL. The proxy tag goes into the path
/// and the probe URL into the query, both percent-encoded: the tag
/// contains a space (`Cloudflare WARP`) and the probe URL contains
/// `:` and `/`.
fn delay_url(base: &str, tag: &str, timeout: Duration, url: &str) -> String {
    format!(
        "{}/proxies/{}/delay?timeout={}&url={}",
        base,
        percent_encode_segment(tag),
        timeout.as_millis(),
        percent_encode_segment(url)
    )
}

/// Percent-encode one path segment or query value. Everything outside
/// the unreserved set is encoded (stricter than necessary for query
/// values, but correct for both).
fn percent_encode_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Parse a delay response: `{"delay": <ms>}`.
fn parse_delay_response(body: &str) -> Result<u64, ClashApiError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| ClashApiError::BadResponse(format!("invalid JSON: {error}")))?;
    value["delay"]
        .as_u64()
        .ok_or_else(|| ClashApiError::BadResponse(format!("missing numeric 'delay' in: {body}")))
}

/// Pull the engine's `message` field out of an error body
/// (`{"message": "..."}`), if present.
fn extract_error_message(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value["message"].as_str().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding_keeps_unreserved_chars() {
        assert_eq!(percent_encode_segment("abc-XYZ_019.~"), "abc-XYZ_019.~");
    }

    #[test]
    fn percent_encoding_encodes_space_in_tag() {
        assert_eq!(
            percent_encode_segment("Cloudflare WARP"),
            "Cloudflare%20WARP"
        );
    }

    #[test]
    fn percent_encoding_encodes_url_chars() {
        assert_eq!(
            percent_encode_segment("https://www.google.com/generate_204"),
            "https%3A%2F%2Fwww.google.com%2Fgenerate_204"
        );
    }

    #[test]
    fn delay_url_builds_full_request_url() {
        assert_eq!(
            delay_url(
                "http://127.0.0.1:54321",
                "Cloudflare WARP",
                Duration::from_secs(10),
                "https://www.google.com/generate_204",
            ),
            "http://127.0.0.1:54321/proxies/Cloudflare%20WARP/delay?timeout=10000&url=https%3A%2F%2Fwww.google.com%2Fgenerate_204",
        );
    }

    #[test]
    fn parse_delay_response_reads_delay() {
        assert_eq!(parse_delay_response(r#"{"delay": 123}"#).unwrap(), 123);
    }

    #[test]
    fn parse_delay_response_rejects_missing_delay() {
        assert!(matches!(
            parse_delay_response(r#"{"ok": true}"#),
            Err(ClashApiError::BadResponse(_))
        ));
    }

    #[test]
    fn parse_delay_response_rejects_invalid_json() {
        assert!(matches!(
            parse_delay_response("not json"),
            Err(ClashApiError::BadResponse(_))
        ));
    }

    #[test]
    fn extract_error_message_reads_message_field() {
        assert_eq!(
            extract_error_message(r#"{"message": "tunnel not initialized"}"#).as_deref(),
            Some("tunnel not initialized")
        );
        assert_eq!(extract_error_message(r#"{"delay": 1}"#), None);
        assert_eq!(extract_error_message("garbage"), None);
    }
}
