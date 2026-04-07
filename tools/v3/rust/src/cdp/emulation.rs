//! CDP Emulation domain — device metrics, geolocation, user-agent, media, timezone, etc.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenOrientation {
    #[serde(rename = "type")]
    pub type_: String,
    pub angle: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayFeature {
    pub orientation: String,
    pub offset: i32,
    pub mask_length: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaFeature {
    pub name: String,
    pub value: String,
}

/// A device preset for quick emulation setup.
#[derive(Debug, Clone)]
pub struct DevicePreset {
    pub name: &'static str,
    pub width: i32,
    pub height: i32,
    pub device_scale_factor: f64,
    pub mobile: bool,
    pub user_agent: &'static str,
}

// ── Device Presets ─────────────────────────────────────────────────

pub const IPHONE_15_PRO: DevicePreset = DevicePreset {
    name: "iPhone 15 Pro",
    width: 393,
    height: 852,
    device_scale_factor: 3.0,
    mobile: true,
    user_agent: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15",
};

pub const IPAD_PRO: DevicePreset = DevicePreset {
    name: "iPad Pro",
    width: 1024,
    height: 1366,
    device_scale_factor: 2.0,
    mobile: true,
    user_agent: "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15",
};

pub const PIXEL_7: DevicePreset = DevicePreset {
    name: "Pixel 7",
    width: 412,
    height: 915,
    device_scale_factor: 2.625,
    mobile: true,
    user_agent: "Mozilla/5.0 (Linux; Android 13; Pixel 7)",
};

pub const DESKTOP_1080P: DevicePreset = DevicePreset {
    name: "Desktop 1080p",
    width: 1920,
    height: 1080,
    device_scale_factor: 1.0,
    mobile: false,
    user_agent: "",
};

pub const DESKTOP_1440P: DevicePreset = DevicePreset {
    name: "Desktop 1440p",
    width: 2560,
    height: 1440,
    device_scale_factor: 1.0,
    mobile: false,
    user_agent: "",
};

// ── Methods ────────────────────────────────────────────────────────

pub async fn set_device_metrics_override(
    transport: &dyn CdpTransport,
    width: i32,
    height: i32,
    device_scale_factor: f64,
    mobile: bool,
    screen_orientation: Option<ScreenOrientation>,
    display_feature: Option<DisplayFeature>,
) -> CdpResult<()> {
    let mut params = json!({
        "width": width,
        "height": height,
        "deviceScaleFactor": device_scale_factor,
        "mobile": mobile,
    });
    if let Some(ref so) = screen_orientation {
        params["screenOrientation"] = serde_json::to_value(so)?;
    }
    if let Some(ref df) = display_feature {
        params["displayFeature"] = serde_json::to_value(df)?;
    }
    transport
        .send("Emulation.setDeviceMetricsOverride", params)
        .await?;
    Ok(())
}

pub async fn clear_device_metrics_override(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport
        .send("Emulation.clearDeviceMetricsOverride", json!({}))
        .await?;
    Ok(())
}

pub async fn set_geolocation_override(
    transport: &dyn CdpTransport,
    latitude: Option<f64>,
    longitude: Option<f64>,
    accuracy: Option<f64>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(lat) = latitude {
        params["latitude"] = json!(lat);
    }
    if let Some(lng) = longitude {
        params["longitude"] = json!(lng);
    }
    if let Some(acc) = accuracy {
        params["accuracy"] = json!(acc);
    }
    transport
        .send("Emulation.setGeolocationOverride", params)
        .await?;
    Ok(())
}

pub async fn clear_geolocation_override(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport
        .send("Emulation.clearGeolocationOverride", json!({}))
        .await?;
    Ok(())
}

pub async fn set_user_agent_override(
    transport: &dyn CdpTransport,
    user_agent: &str,
    accept_language: Option<&str>,
    platform: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({ "userAgent": user_agent });
    if let Some(lang) = accept_language {
        params["acceptLanguage"] = json!(lang);
    }
    if let Some(plat) = platform {
        params["platform"] = json!(plat);
    }
    transport
        .send("Emulation.setUserAgentOverride", params)
        .await?;
    Ok(())
}

pub async fn set_emulated_media(
    transport: &dyn CdpTransport,
    media: Option<&str>,
    features: Option<Vec<MediaFeature>>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(m) = media {
        params["media"] = json!(m);
    }
    if let Some(ref f) = features {
        params["features"] = serde_json::to_value(f)?;
    }
    transport
        .send("Emulation.setEmulatedMedia", params)
        .await?;
    Ok(())
}

pub async fn set_timezone_override(
    transport: &dyn CdpTransport,
    timezone_id: &str,
) -> CdpResult<()> {
    transport
        .send(
            "Emulation.setTimezoneOverride",
            json!({ "timezoneId": timezone_id }),
        )
        .await?;
    Ok(())
}

pub async fn set_locale_override(
    transport: &dyn CdpTransport,
    locale: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(l) = locale {
        params["locale"] = json!(l);
    }
    transport
        .send("Emulation.setLocaleOverride", params)
        .await?;
    Ok(())
}

pub async fn set_touch_emulation_enabled(
    transport: &dyn CdpTransport,
    enabled: bool,
    max_touch_points: Option<i32>,
) -> CdpResult<()> {
    let mut params = json!({ "enabled": enabled });
    if let Some(mtp) = max_touch_points {
        params["maxTouchPoints"] = json!(mtp);
    }
    transport
        .send("Emulation.setTouchEmulationEnabled", params)
        .await?;
    Ok(())
}

pub async fn set_cpu_throttling_rate(
    transport: &dyn CdpTransport,
    rate: f64,
) -> CdpResult<()> {
    transport
        .send(
            "Emulation.setCPUThrottlingRate",
            json!({ "rate": rate }),
        )
        .await?;
    Ok(())
}

pub async fn set_idle_override(
    transport: &dyn CdpTransport,
    is_user_active: bool,
    is_screen_unlocked: bool,
) -> CdpResult<()> {
    transport
        .send(
            "Emulation.setIdleOverride",
            json!({
                "isUserActive": is_user_active,
                "isScreenUnlocked": is_screen_unlocked,
            }),
        )
        .await?;
    Ok(())
}

pub async fn clear_idle_override(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport
        .send("Emulation.clearIdleOverride", json!({}))
        .await?;
    Ok(())
}

pub async fn set_scrollbars_hidden(
    transport: &dyn CdpTransport,
    hidden: bool,
) -> CdpResult<()> {
    transport
        .send(
            "Emulation.setScrollbarsHidden",
            json!({ "hidden": hidden }),
        )
        .await?;
    Ok(())
}

pub async fn set_auto_dark_mode_override(
    transport: &dyn CdpTransport,
    enabled: Option<bool>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(e) = enabled {
        params["enabled"] = json!(e);
    }
    transport
        .send("Emulation.setAutoDarkModeOverride", params)
        .await?;
    Ok(())
}

pub async fn set_emulated_vision_deficiency(
    transport: &dyn CdpTransport,
    type_: &str,
) -> CdpResult<()> {
    transport
        .send(
            "Emulation.setEmulatedVisionDeficiency",
            json!({ "type": type_ }),
        )
        .await?;
    Ok(())
}

pub async fn set_default_background_color_override(
    transport: &dyn CdpTransport,
    color: Option<Value>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(c) = color {
        params["color"] = c;
    }
    transport
        .send("Emulation.setDefaultBackgroundColorOverride", params)
        .await?;
    Ok(())
}

pub async fn set_script_execution_disabled(
    transport: &dyn CdpTransport,
    disabled: bool,
) -> CdpResult<()> {
    transport
        .send(
            "Emulation.setScriptExecutionDisabled",
            json!({ "value": disabled }),
        )
        .await?;
    Ok(())
}

pub async fn set_document_cookie_disabled(
    transport: &dyn CdpTransport,
    disabled: bool,
) -> CdpResult<()> {
    transport
        .send(
            "Emulation.setDocumentCookieDisabled",
            json!({ "disabled": disabled }),
        )
        .await?;
    Ok(())
}

pub async fn set_focus_emulation_enabled(
    transport: &dyn CdpTransport,
    enabled: bool,
) -> CdpResult<()> {
    transport
        .send(
            "Emulation.setFocusEmulationEnabled",
            json!({ "enabled": enabled }),
        )
        .await?;
    Ok(())
}

pub async fn set_hardware_concurrency_override(
    transport: &dyn CdpTransport,
    hardware_concurrency: i32,
) -> CdpResult<()> {
    transport
        .send(
            "Emulation.setHardwareConcurrencyOverride",
            json!({ "hardwareConcurrency": hardware_concurrency }),
        )
        .await?;
    Ok(())
}

/// Apply a full device preset: metrics + user agent + touch (if mobile).
pub async fn apply_device_preset(
    transport: &dyn CdpTransport,
    preset: &DevicePreset,
) -> CdpResult<()> {
    set_device_metrics_override(
        transport,
        preset.width,
        preset.height,
        preset.device_scale_factor,
        preset.mobile,
        None,
        None,
    )
    .await?;

    if !preset.user_agent.is_empty() {
        set_user_agent_override(transport, preset.user_agent, None, None).await?;
    }

    if preset.mobile {
        set_touch_emulation_enabled(transport, true, Some(5)).await?;
    }

    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;
    use serde_json::json;

    #[tokio::test]
    async fn test_set_device_metrics() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setDeviceMetricsOverride", json!({}))
            .await;

        set_device_metrics_override(&mock, 375, 812, 3.0, true, None, None)
            .await
            .unwrap();

        let params = mock
            .call_params("Emulation.setDeviceMetricsOverride", 0)
            .await
            .unwrap();
        assert_eq!(params["width"], 375);
        assert_eq!(params["height"], 812);
        assert_eq!(params["deviceScaleFactor"], 3.0);
        assert_eq!(params["mobile"], true);
    }

    #[tokio::test]
    async fn test_clear_device_metrics() {
        let mock = MockTransport::new();
        mock.expect("Emulation.clearDeviceMetricsOverride", json!({}))
            .await;

        clear_device_metrics_override(&mock).await.unwrap();
        mock.assert_called_once("Emulation.clearDeviceMetricsOverride")
            .await;
    }

    #[tokio::test]
    async fn test_set_geolocation() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setGeolocationOverride", json!({}))
            .await;

        set_geolocation_override(&mock, Some(40.4168), Some(-3.7038), Some(100.0))
            .await
            .unwrap();

        let params = mock
            .call_params("Emulation.setGeolocationOverride", 0)
            .await
            .unwrap();
        assert_eq!(params["latitude"], 40.4168);
        assert_eq!(params["longitude"], -3.7038);
        assert_eq!(params["accuracy"], 100.0);
    }

    #[tokio::test]
    async fn test_clear_geolocation() {
        let mock = MockTransport::new();
        mock.expect("Emulation.clearGeolocationOverride", json!({}))
            .await;

        clear_geolocation_override(&mock).await.unwrap();
        mock.assert_called_once("Emulation.clearGeolocationOverride")
            .await;
    }

    #[tokio::test]
    async fn test_set_user_agent() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setUserAgentOverride", json!({}))
            .await;

        set_user_agent_override(&mock, "CustomBot/1.0", Some("en-US"), Some("Linux"))
            .await
            .unwrap();

        let params = mock
            .call_params("Emulation.setUserAgentOverride", 0)
            .await
            .unwrap();
        assert_eq!(params["userAgent"], "CustomBot/1.0");
        assert_eq!(params["acceptLanguage"], "en-US");
        assert_eq!(params["platform"], "Linux");
    }

    #[tokio::test]
    async fn test_set_emulated_media_dark() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setEmulatedMedia", json!({})).await;

        set_emulated_media(
            &mock,
            None,
            Some(vec![MediaFeature {
                name: "prefers-color-scheme".into(),
                value: "dark".into(),
            }]),
        )
        .await
        .unwrap();

        let params = mock
            .call_params("Emulation.setEmulatedMedia", 0)
            .await
            .unwrap();
        assert_eq!(params["features"][0]["name"], "prefers-color-scheme");
        assert_eq!(params["features"][0]["value"], "dark");
    }

    #[tokio::test]
    async fn test_set_timezone() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setTimezoneOverride", json!({}))
            .await;

        set_timezone_override(&mock, "Europe/Madrid").await.unwrap();

        let params = mock
            .call_params("Emulation.setTimezoneOverride", 0)
            .await
            .unwrap();
        assert_eq!(params["timezoneId"], "Europe/Madrid");
    }

    #[tokio::test]
    async fn test_set_locale() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setLocaleOverride", json!({})).await;

        set_locale_override(&mock, Some("es-ES")).await.unwrap();

        let params = mock
            .call_params("Emulation.setLocaleOverride", 0)
            .await
            .unwrap();
        assert_eq!(params["locale"], "es-ES");
    }

    #[tokio::test]
    async fn test_set_touch_emulation() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setTouchEmulationEnabled", json!({}))
            .await;

        set_touch_emulation_enabled(&mock, true, Some(5))
            .await
            .unwrap();

        let params = mock
            .call_params("Emulation.setTouchEmulationEnabled", 0)
            .await
            .unwrap();
        assert_eq!(params["enabled"], true);
        assert_eq!(params["maxTouchPoints"], 5);
    }

    #[tokio::test]
    async fn test_set_cpu_throttling() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setCPUThrottlingRate", json!({}))
            .await;

        set_cpu_throttling_rate(&mock, 4.0).await.unwrap();

        let params = mock
            .call_params("Emulation.setCPUThrottlingRate", 0)
            .await
            .unwrap();
        assert_eq!(params["rate"], 4.0);
    }

    #[tokio::test]
    async fn test_set_idle_override() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setIdleOverride", json!({})).await;

        set_idle_override(&mock, true, true).await.unwrap();

        let params = mock
            .call_params("Emulation.setIdleOverride", 0)
            .await
            .unwrap();
        assert_eq!(params["isUserActive"], true);
        assert_eq!(params["isScreenUnlocked"], true);
    }

    #[tokio::test]
    async fn test_set_vision_deficiency() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setEmulatedVisionDeficiency", json!({}))
            .await;

        set_emulated_vision_deficiency(&mock, "deuteranopia")
            .await
            .unwrap();

        let params = mock
            .call_params("Emulation.setEmulatedVisionDeficiency", 0)
            .await
            .unwrap();
        assert_eq!(params["type"], "deuteranopia");
    }

    #[tokio::test]
    async fn test_apply_device_preset_iphone() {
        let mock = MockTransport::new();
        // Expects: setDeviceMetricsOverride + setUserAgentOverride + setTouchEmulationEnabled
        mock.expect("Emulation.setDeviceMetricsOverride", json!({}))
            .await;
        mock.expect("Emulation.setUserAgentOverride", json!({}))
            .await;
        mock.expect("Emulation.setTouchEmulationEnabled", json!({}))
            .await;

        apply_device_preset(&mock, &IPHONE_15_PRO).await.unwrap();

        let metrics = mock
            .call_params("Emulation.setDeviceMetricsOverride", 0)
            .await
            .unwrap();
        assert_eq!(metrics["width"], 393);
        assert_eq!(metrics["height"], 852);
        assert_eq!(metrics["deviceScaleFactor"], 3.0);
        assert_eq!(metrics["mobile"], true);

        let ua = mock
            .call_params("Emulation.setUserAgentOverride", 0)
            .await
            .unwrap();
        assert!(ua["userAgent"]
            .as_str()
            .unwrap()
            .contains("iPhone"));

        let touch = mock
            .call_params("Emulation.setTouchEmulationEnabled", 0)
            .await
            .unwrap();
        assert_eq!(touch["enabled"], true);
        assert_eq!(touch["maxTouchPoints"], 5);
    }

    #[tokio::test]
    async fn test_scrollbars_hidden() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setScrollbarsHidden", json!({}))
            .await;

        set_scrollbars_hidden(&mock, true).await.unwrap();

        let params = mock
            .call_params("Emulation.setScrollbarsHidden", 0)
            .await
            .unwrap();
        assert_eq!(params["hidden"], true);
    }

    #[tokio::test]
    async fn test_set_auto_dark_mode() {
        let mock = MockTransport::new();
        mock.expect("Emulation.setAutoDarkModeOverride", json!({}))
            .await;

        set_auto_dark_mode_override(&mock, Some(true))
            .await
            .unwrap();

        let params = mock
            .call_params("Emulation.setAutoDarkModeOverride", 0)
            .await
            .unwrap();
        assert_eq!(params["enabled"], true);
    }
}
