//! CDP emulation tools — device metrics, user agent, geolocation, network, CPU.
//!
//! Wraps Chrome DevTools Protocol emulation commands into ergonomic methods
//! on `ChromeSession`. All commands are sent to the page session target.

use serde_json::json;

use crate::Result;

// ─── Types ───

/// Viewport configuration for device metrics emulation.
#[derive(Debug, Clone)]
pub struct ViewportConfig {
    pub width: u32,
    pub height: u32,
    pub device_pixel_ratio: f64,
    pub mobile: bool,
    pub touch: bool,
    pub landscape: bool,
}

impl Default for ViewportConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            device_pixel_ratio: 1.0,
            mobile: false,
            touch: false,
            landscape: false,
        }
    }
}

impl ViewportConfig {
    /// Build the CDP params for `Emulation.setDeviceMetricsOverride`.
    pub fn to_cdp_params(&self) -> serde_json::Value {
        let (w, h) = if self.landscape && self.height > self.width {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        json!({
            "width": w,
            "height": h,
            "deviceScaleFactor": self.device_pixel_ratio,
            "mobile": self.mobile,
        })
    }
}

/// Preferred color scheme for `Emulation.setEmulatedMedia`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    Dark,
    Light,
    /// Reset to browser default.
    Auto,
}

impl ColorScheme {
    /// Build the CDP params for `Emulation.setEmulatedMedia`.
    pub fn to_cdp_params(self) -> serde_json::Value {
        match self {
            Self::Dark => json!({
                "features": [{ "name": "prefers-color-scheme", "value": "dark" }]
            }),
            Self::Light => json!({
                "features": [{ "name": "prefers-color-scheme", "value": "light" }]
            }),
            Self::Auto => json!({
                "features": []
            }),
        }
    }
}

/// Network condition presets for `Network.emulateNetworkConditions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkCondition {
    Offline,
    Slow3G,
    Fast3G,
    Slow4G,
    Fast4G,
}

impl NetworkCondition {
    /// Returns (offline, latency_ms, download_throughput, upload_throughput).
    pub fn params(self) -> (bool, f64, f64, f64) {
        match self {
            Self::Offline => (true, 0.0, -1.0, -1.0),
            Self::Slow3G => (false, 2000.0, 50_000.0, 50_000.0),
            Self::Fast3G => (false, 562.5, 180_000.0, 84_375.0),
            Self::Slow4G => (false, 170.0, 400_000.0, 150_000.0),
            Self::Fast4G => (false, 40.0, 4_000_000.0, 3_000_000.0),
        }
    }

    /// Build the CDP params for `Network.emulateNetworkConditions`.
    pub fn to_cdp_params(self) -> serde_json::Value {
        let (offline, latency, down, up) = self.params();
        json!({
            "offline": offline,
            "latency": latency,
            "downloadThroughput": down,
            "uploadThroughput": up,
        })
    }
}

/// All emulation options bundled. Each `None` field is skipped.
#[derive(Debug, Clone, Default)]
pub struct EmulateOptions {
    pub viewport: Option<ViewportConfig>,
    pub user_agent: Option<String>,
    pub geolocation: Option<(f64, f64)>,
    pub color_scheme: Option<ColorScheme>,
    pub network_conditions: Option<NetworkCondition>,
    pub cpu_throttling: Option<f64>,
}

// ─── ChromeSession methods ───

use crate::session::ChromeSession;

impl ChromeSession {
    /// Apply multiple emulation settings in one call.
    ///
    /// Each `Some` field in `options` sends the corresponding CDP command.
    /// Fields set to `None` are skipped (not reset).
    pub async fn emulate(&self, options: &EmulateOptions) -> Result<()> {
        if let Some(ref vp) = options.viewport {
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Emulation.setDeviceMetricsOverride",
                    Some(vp.to_cdp_params()),
                )
                .await?;

            if vp.touch {
                self.cdp
                    .send_to(
                        &self.page_session_id,
                        "Emulation.setTouchEmulationEnabled",
                        Some(json!({ "enabled": true })),
                    )
                    .await?;
            }
        }

        if let Some(ref ua) = options.user_agent {
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Emulation.setUserAgentOverride",
                    Some(json!({ "userAgent": ua })),
                )
                .await?;
        }

        if let Some((lat, lon)) = options.geolocation {
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Emulation.setGeolocationOverride",
                    Some(json!({
                        "latitude": lat,
                        "longitude": lon,
                        "accuracy": 1,
                    })),
                )
                .await?;
        }

        if let Some(cs) = options.color_scheme {
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Emulation.setEmulatedMedia",
                    Some(cs.to_cdp_params()),
                )
                .await?;
        }

        if let Some(nc) = options.network_conditions {
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Network.emulateNetworkConditions",
                    Some(nc.to_cdp_params()),
                )
                .await?;
        }

        if let Some(rate) = options.cpu_throttling {
            let rate = rate.clamp(1.0, 20.0);
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Emulation.setCPUThrottlingRate",
                    Some(json!({ "rate": rate })),
                )
                .await?;
        }

        Ok(())
    }

    /// Resize the page viewport to the given dimensions.
    ///
    /// Uses `Emulation.setDeviceMetricsOverride` with scale=1, mobile=false.
    pub async fn resize_page(&self, width: u32, height: u32) -> Result<()> {
        self.cdp
            .send_to(
                &self.page_session_id,
                "Emulation.setDeviceMetricsOverride",
                Some(json!({
                    "width": width,
                    "height": height,
                    "deviceScaleFactor": 1,
                    "mobile": false,
                })),
            )
            .await?;
        Ok(())
    }
}
