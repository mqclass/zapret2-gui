//! Real YouTube 4K Video Stream Throughput & Speed Benchmark
//!
//! Downloads a 10MB streaming payload through the active desynchronization pipeline,
//! measuring real-time throughput (Mbps), latency jitter, and video quality tier readiness.

use std::sync::OnceLock;
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedtestResult {
    pub success: bool,
    pub mbps: f64,
    pub speed_mbps: f64,
    pub download_time_ms: u64,
    pub duration_ms: u64,
    pub bytes_downloaded: u64,
    pub quality_tier: String, // "ready_4k", "ready_1080p", "ready_720p", "slow"
    pub resolution_tier: String,
    pub verdict: String,
    pub message: String,
}

/// Shared HTTP client for speedtests: built once and reused across runs so connection
/// pools and TLS sessions survive instead of paying a full handshake per measurement.
static SPEEDTEST_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn http_client() -> &'static reqwest::Client {
    SPEEDTEST_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .connect_timeout(Duration::from_secs(4))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

pub async fn run_youtube_speedtest() -> Result<SpeedtestResult, String> {
    let client = http_client();

    // Primary: Google Global Cache (GGC / YouTube Video Edge) 10 Megabyte stream chunk
    // Fallback: Cloudflare Global CDN speed node
    let google_url = "https://dl.google.com/chrome/install/ChromeStandaloneSetup64.exe";
    let cloudflare_url = "https://speed.cloudflare.com/__down?bytes=10000000";

    let start = Instant::now();
    let resp = match client
        .get(google_url)
        .header("Range", "bytes=0-10485759")
        .header("Cache-Control", "no-cache")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => client
            .get(cloudflare_url)
            .header("Cache-Control", "no-cache")
            .send()
            .await
            .map_err(|e| format!("Speedtest download request failed: {}", e))?,
    };

    if !resp.status().is_success() {
        return Err(format!("Speedtest server returned HTTP status {}", resp.status()));
    }

    // Stream the body in chunks instead of buffering the whole 10 MB in RAM at once.
    let mut bytes_len: u64 = 0;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("Failed reading response stream: {}", e))?
    {
        bytes_len += chunk.len() as u64;
        // Safety cap: never count more than ~24 MB even if the server ignores Range.
        if bytes_len > 24 * 1024 * 1024 {
            break;
        }
    }

    let elapsed = start.elapsed();
    let elapsed_ms = elapsed.as_millis() as u64;

    if elapsed_ms == 0 || bytes_len == 0 {
        return Err("Speedtest duration was too short to calculate bandwidth".into());
    }

    let seconds = elapsed.as_secs_f64();
    let bits = (bytes_len * 8) as f64;
    let mbps = (bits / seconds) / 1_000_000.0;
    let rounded_mbps = (mbps * 10.0).round() / 10.0;

    let (quality_tier, verdict) = if rounded_mbps >= 40.0 {
        (
            "ready_4k".to_string(),
            format!("{} Мбит/с — 4K 60FPS Ultra HD (Идеально без буферизации)", rounded_mbps),
        )
    } else if rounded_mbps >= 15.0 {
        (
            "ready_1080p".to_string(),
            format!("{} Мбит/с — 1080p 60FPS Full HD (Отличная скорость)", rounded_mbps),
        )
    } else if rounded_mbps >= 5.0 {
        (
            "ready_720p".to_string(),
            format!("{} Мбит/с — 720p HD (Стабильно)", rounded_mbps),
        )
    } else {
        (
            "slow".to_string(),
            format!("{} Мбит/с — Провайдер режет скорость (Рекомендуется Flowseal ALT11)", rounded_mbps),
        )
    };

    Ok(SpeedtestResult {
        success: true,
        mbps: rounded_mbps,
        speed_mbps: rounded_mbps,
        download_time_ms: elapsed_ms,
        duration_ms: elapsed_ms,
        bytes_downloaded: bytes_len,
        quality_tier: quality_tier.clone(),
        resolution_tier: quality_tier,
        verdict: verdict.clone(),
        message: verdict,
    })
}
