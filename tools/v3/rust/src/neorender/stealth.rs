//! Stealth configuration — anti-detection fingerprint injection.
//!
//! Generates JS that overrides navigator/screen properties to look like
//! a real Chrome browser. The fingerprint is consistent (not random),
//! because randomness is itself a detection signal.

/// Stealth fingerprint configuration.
pub struct StealthConfig {
    pub screen_width: u32,
    pub screen_height: u32,
    pub languages: Vec<String>,
    pub platform: String,
}

impl StealthConfig {
    /// Default desktop Chrome fingerprint.
    pub fn default_desktop() -> Self {
        Self {
            screen_width: 1920,
            screen_height: 1080,
            languages: vec!["es-ES".into(), "es".into(), "en".into()],
            platform: "MacIntel".into(),
        }
    }

    /// Generate JS code that sets the stealth fingerprint.
    /// This overrides the static js/stealth.js defaults with custom values.
    pub fn to_js(&self) -> String {
        let langs_json: Vec<String> = self.languages.iter().map(|l| format!("'{}'", l)).collect();
        format!(
            r#"
Object.defineProperty(navigator, 'languages', {{ get: () => [{}] }});
Object.defineProperty(navigator, 'platform', {{ get: () => '{}' }});
Object.defineProperty(screen, 'width', {{ get: () => {} }});
Object.defineProperty(screen, 'height', {{ get: () => {} }});
Object.defineProperty(screen, 'availWidth', {{ get: () => {} }});
Object.defineProperty(screen, 'availHeight', {{ get: () => {} }});
"#,
            langs_json.join(", "),
            self.platform,
            self.screen_width,
            self.screen_height,
            self.screen_width,
            self.screen_height.saturating_sub(40), // taskbar offset
        )
    }
}
