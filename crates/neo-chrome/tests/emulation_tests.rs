//! Unit tests for emulation types and CDP param construction.
//!
//! These tests validate the data structures, presets, and serialization
//! used by the emulation module without requiring a live Chrome instance.

use neo_chrome::emulation::{
    ColorScheme, EmulateOptions, NetworkCondition, ViewportConfig,
};

// ─── NetworkCondition preset values ───

#[test]
fn network_offline_params() {
    let (offline, latency, down, up) = NetworkCondition::Offline.params();
    assert!(offline);
    assert_eq!(latency, 0.0);
    assert_eq!(down, -1.0);
    assert_eq!(up, -1.0);
}

#[test]
fn network_slow3g_params() {
    let (offline, latency, down, up) = NetworkCondition::Slow3G.params();
    assert!(!offline);
    assert_eq!(latency, 2000.0);
    assert_eq!(down, 50_000.0);
    assert_eq!(up, 50_000.0);
}

#[test]
fn network_fast3g_params() {
    let (offline, latency, down, up) = NetworkCondition::Fast3G.params();
    assert!(!offline);
    assert_eq!(latency, 562.5);
    assert_eq!(down, 180_000.0);
    assert_eq!(up, 84_375.0);
}

#[test]
fn network_slow4g_params() {
    let (offline, latency, down, up) = NetworkCondition::Slow4G.params();
    assert!(!offline);
    assert_eq!(latency, 170.0);
    assert_eq!(down, 400_000.0);
    assert_eq!(up, 150_000.0);
}

#[test]
fn network_fast4g_params() {
    let (offline, latency, down, up) = NetworkCondition::Fast4G.params();
    assert!(!offline);
    assert_eq!(latency, 40.0);
    assert_eq!(down, 4_000_000.0);
    assert_eq!(up, 3_000_000.0);
}

#[test]
fn network_cdp_params_structure() {
    let params = NetworkCondition::Slow3G.to_cdp_params();
    assert_eq!(params["offline"], false);
    assert_eq!(params["latency"], 2000.0);
    assert_eq!(params["downloadThroughput"], 50_000.0);
    assert_eq!(params["uploadThroughput"], 50_000.0);
}

// ─── ViewportConfig serialization ───

#[test]
fn viewport_default_values() {
    let vp = ViewportConfig::default();
    assert_eq!(vp.width, 1280);
    assert_eq!(vp.height, 720);
    assert_eq!(vp.device_pixel_ratio, 1.0);
    assert!(!vp.mobile);
    assert!(!vp.touch);
    assert!(!vp.landscape);
}

#[test]
fn viewport_cdp_params_basic() {
    let vp = ViewportConfig {
        width: 375,
        height: 812,
        device_pixel_ratio: 3.0,
        mobile: true,
        touch: true,
        landscape: false,
    };
    let params = vp.to_cdp_params();
    assert_eq!(params["width"], 375);
    assert_eq!(params["height"], 812);
    assert_eq!(params["deviceScaleFactor"], 3.0);
    assert_eq!(params["mobile"], true);
}

#[test]
fn viewport_landscape_swaps_dimensions() {
    let vp = ViewportConfig {
        width: 375,
        height: 812,
        device_pixel_ratio: 1.0,
        mobile: true,
        touch: false,
        landscape: true,
    };
    let params = vp.to_cdp_params();
    // height > width, so landscape swaps them
    assert_eq!(params["width"], 812);
    assert_eq!(params["height"], 375);
}

#[test]
fn viewport_landscape_no_swap_when_already_wide() {
    let vp = ViewportConfig {
        width: 1920,
        height: 1080,
        device_pixel_ratio: 1.0,
        mobile: false,
        touch: false,
        landscape: true,
    };
    let params = vp.to_cdp_params();
    // width already > height, no swap needed
    assert_eq!(params["width"], 1920);
    assert_eq!(params["height"], 1080);
}

// ─── EmulateOptions default/builder ───

#[test]
fn emulate_options_default_all_none() {
    let opts = EmulateOptions::default();
    assert!(opts.viewport.is_none());
    assert!(opts.user_agent.is_none());
    assert!(opts.geolocation.is_none());
    assert!(opts.color_scheme.is_none());
    assert!(opts.network_conditions.is_none());
    assert!(opts.cpu_throttling.is_none());
}

#[test]
fn emulate_options_partial_fill() {
    let opts = EmulateOptions {
        user_agent: Some("Mozilla/5.0 NeoBot".to_string()),
        cpu_throttling: Some(4.0),
        ..Default::default()
    };
    assert!(opts.viewport.is_none());
    assert_eq!(opts.user_agent.as_deref(), Some("Mozilla/5.0 NeoBot"));
    assert_eq!(opts.cpu_throttling, Some(4.0));
}

// ─── ColorScheme media feature construction ───

#[test]
fn color_scheme_dark_cdp_params() {
    let params = ColorScheme::Dark.to_cdp_params();
    let features = params["features"].as_array().unwrap();
    assert_eq!(features.len(), 1);
    assert_eq!(features[0]["name"], "prefers-color-scheme");
    assert_eq!(features[0]["value"], "dark");
}

#[test]
fn color_scheme_light_cdp_params() {
    let params = ColorScheme::Light.to_cdp_params();
    let features = params["features"].as_array().unwrap();
    assert_eq!(features.len(), 1);
    assert_eq!(features[0]["name"], "prefers-color-scheme");
    assert_eq!(features[0]["value"], "light");
}

#[test]
fn color_scheme_auto_resets_features() {
    let params = ColorScheme::Auto.to_cdp_params();
    let features = params["features"].as_array().unwrap();
    assert!(features.is_empty(), "Auto should send empty features to reset");
}
