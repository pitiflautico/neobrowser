//! Session video recording via CDP screencast.
//!
//! Captures frames from the current tab and assembles them into an MP4 with
//! ffmpeg. Useful for demos, debugging, and verifying that an action actually
//! had a visible effect.

use std::time::{Duration, Instant};

use serde_json::json;

use crate::cdp::CdpClient;

/// Record the current tab for `seconds` and return the MP4 as base64.
pub async fn record_video_base64(client: &CdpClient, seconds: u64) -> Result<String, String> {
    // Start screencast.
    client
        .send(
            "Page.startScreencast",
            json!({
                "format": "png",
                "quality": 80,
                "maxWidth": 1280,
                "maxHeight": 720,
                "everyNthFrame": 1,
            }),
        )
        .await
        .map_err(|e| format!("startScreencast failed: {e}"))?;

    let frames_dir = std::env::temp_dir().join(format!("neobrowser-video-{}", uuid_simple()));
    std::fs::create_dir_all(&frames_dir).map_err(|e| format!("mkdir failed: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut frame_count = 0u32;

    // Listen for screencast frames.
    while Instant::now() < deadline {
        // We poll for frames by sending a no-op and checking for events.
        // A real implementation would use the event listener; this is a
        // simplified version that captures at a fixed rate.
        if let Ok(data) = client
            .send("Page.captureScreenshot", json!({"format": "png"}))
            .await
        {
            if let Some(b64) = data.get("data").and_then(|d| d.as_str()) {
                let frame_path = frames_dir.join(format!("frame_{frame_count:04}.png"));
                if let Ok(bytes) = base64_decode(b64) {
                    let _ = std::fs::write(&frame_path, bytes);
                    frame_count += 1;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Stop screencast.
    let _ = client.send("Page.stopScreencast", json!({})).await;

    if frame_count == 0 {
        return Err("no frames captured".into());
    }

    // Assemble with ffmpeg. Scale to even dimensions because libx264 requires
    // width and height divisible by 2; a screenshot of an odd-sized viewport
    // fails with "height not divisible by 2".
    let out_path = std::env::temp_dir().join(format!("neobrowser-video-{}.mp4", uuid_simple()));
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-framerate",
            "10",
            "-i",
            &format!("{}/frame_%04d.png", frames_dir.display()),
            "-vf",
            "scale=trunc(iw/2)*2:trunc(ih/2)*2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "faststart",
            out_path.to_str().unwrap_or(""),
        ])
        .status()
        .map_err(|e| format!("ffmpeg failed: {e}"))?;

    if !status.success() {
        return Err(format!("ffmpeg exited with {status}"));
    }

    let mp4_bytes = std::fs::read(&out_path).map_err(|e| format!("read mp4 failed: {e}"))?;
    let _ = std::fs::remove_dir_all(&frames_dir);
    let _ = std::fs::remove_file(&out_path);

    Ok(base64_encode(&mp4_bytes))
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:x}")
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| e.to_string())
}

fn base64_encode(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}
