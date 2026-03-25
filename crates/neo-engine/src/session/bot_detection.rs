//! Bot protection detection — Cloudflare, AWS WAF, Akamai, and generic challenges.
//!
//! Extracted from browser_impl.rs for testability and separation of concerns.

use std::collections::HashMap;

/// Bot protection detection result.
#[derive(Debug, Clone, PartialEq)]
pub enum BotProtection {
    /// No bot protection detected.
    None,
    /// Cloudflare (CDN + WAF). May have Turnstile challenge.
    Cloudflare { has_challenge: bool },
    /// AWS WAF / Shield.
    AwsWaf,
    /// Akamai Bot Manager.
    Akamai,
    /// Generic bot detection (unknown provider).
    Generic(String),
}

impl BotProtection {
    pub fn is_protected(&self) -> bool {
        !matches!(self, BotProtection::None)
    }

    pub fn needs_chrome_transport(&self) -> bool {
        matches!(self, BotProtection::Cloudflare { .. } | BotProtection::Akamai)
    }
}

/// Detect bot protection from HTTP response headers.
pub fn detect_bot_protection(headers: &HashMap<String, String>) -> BotProtection {
    // Cloudflare
    if headers.contains_key("cf-ray") {
        let has_challenge = headers.get("cf-mitigated").is_some()
            || headers.get("server").map(|s| s.contains("challenge")).unwrap_or(false);
        return BotProtection::Cloudflare { has_challenge };
    }
    if let Some(server) = headers.get("server") {
        if server.eq_ignore_ascii_case("cloudflare") {
            return BotProtection::Cloudflare { has_challenge: false };
        }
    }
    if headers.contains_key("cf-cache-status")
        || headers.contains_key("cf-request-id")
        || headers.contains_key("cf-mitigated")
    {
        let has_challenge = headers.contains_key("cf-mitigated");
        return BotProtection::Cloudflare { has_challenge };
    }

    // AWS WAF
    if headers.contains_key("x-amzn-waf-action") || headers.contains_key("x-amzn-requestid") {
        return BotProtection::AwsWaf;
    }

    // Akamai
    if headers.contains_key("x-akamai-transformed")
        || headers.get("server").map(|s| s.contains("AkamaiGHost")).unwrap_or(false)
    {
        return BotProtection::Akamai;
    }

    // Generic — check for common bot challenge indicators
    if let Some(server) = headers.get("server") {
        if server.contains("ddos-guard") || server.contains("DDoS") {
            return BotProtection::Generic("ddos-guard".into());
        }
    }

    BotProtection::None
}

/// Detect bot protection from HTML body content (post-parse).
/// Call after HTML is loaded to catch JS-based challenges.
pub fn detect_bot_challenge_in_body(body: &str) -> Option<BotProtection> {
    // Cloudflare Turnstile / Challenge
    if body.contains("challenges.cloudflare.com")
        || body.contains("cf-turnstile")
        || body.contains("cf_chl_opt")
        || body.contains("cf-challenge-running")
    {
        return Some(BotProtection::Cloudflare { has_challenge: true });
    }

    // Cloudflare "Just a moment" / "Checking your browser"
    if body.contains("Just a moment") && body.contains("cloudflare") {
        return Some(BotProtection::Cloudflare { has_challenge: true });
    }

    // reCAPTCHA
    if body.contains("recaptcha") && (body.contains("google.com/recaptcha") || body.contains("g-recaptcha")) {
        return Some(BotProtection::Generic("recaptcha".into()));
    }

    // hCaptcha
    if body.contains("hcaptcha.com") || body.contains("h-captcha") {
        return Some(BotProtection::Generic("hcaptcha".into()));
    }

    // PerimeterX / Human Security
    if body.contains("px-captcha") || body.contains("perimeterx") {
        return Some(BotProtection::Generic("perimeterx".into()));
    }

    // DataDome
    if body.contains("datadome") && body.contains("captcha") {
        return Some(BotProtection::Generic("datadome".into()));
    }

    // Kasada
    if body.contains("ips.js") && body.contains("_ips") {
        return Some(BotProtection::Generic("kasada".into()));
    }

    None
}

/// Legacy compat — still used by navigate() for domain marking.
pub fn detect_cloudflare(headers: &HashMap<String, String>) -> bool {
    matches!(detect_bot_protection(headers), BotProtection::Cloudflare { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_cloudflare_cf_ray() {
        let mut h = HashMap::new();
        h.insert("cf-ray".to_string(), "abc123-MAD".to_string());
        assert!(detect_cloudflare(&h));
    }

    #[test]
    fn test_detect_cloudflare_server_header() {
        let mut h = HashMap::new();
        h.insert("server".to_string(), "cloudflare".to_string());
        assert!(detect_cloudflare(&h));
    }

    #[test]
    fn test_detect_cloudflare_server_case_insensitive() {
        let mut h = HashMap::new();
        h.insert("server".to_string(), "Cloudflare".to_string());
        assert!(detect_cloudflare(&h));
    }

    #[test]
    fn test_detect_cloudflare_cache_status() {
        let mut h = HashMap::new();
        h.insert("cf-cache-status".to_string(), "DYNAMIC".to_string());
        assert!(detect_cloudflare(&h));
    }

    #[test]
    fn test_detect_cloudflare_mitigated() {
        let mut h = HashMap::new();
        h.insert("cf-mitigated".to_string(), "challenge".to_string());
        assert!(detect_cloudflare(&h));
    }

    #[test]
    fn test_detect_cloudflare_request_id() {
        let mut h = HashMap::new();
        h.insert("cf-request-id".to_string(), "abc".to_string());
        assert!(detect_cloudflare(&h));
    }

    #[test]
    fn test_detect_not_cloudflare_nginx() {
        let mut h = HashMap::new();
        h.insert("server".to_string(), "nginx".to_string());
        assert!(!detect_cloudflare(&h));
    }

    #[test]
    fn test_detect_not_cloudflare_empty() {
        let h = HashMap::new();
        assert!(!detect_cloudflare(&h));
    }

    #[test]
    fn test_detect_not_cloudflare_apache() {
        let mut h = HashMap::new();
        h.insert("server".to_string(), "Apache/2.4".to_string());
        h.insert("x-powered-by".to_string(), "PHP/8.1".to_string());
        assert!(!detect_cloudflare(&h));
    }

    #[test]
    fn test_detect_cloudflare_mixed_headers() {
        let mut h = HashMap::new();
        h.insert("server".to_string(), "nginx".to_string());
        h.insert("cf-ray".to_string(), "xyz-SIN".to_string());
        // cf-ray present → Cloudflare even if server says nginx
        assert!(detect_cloudflare(&h));
    }

    #[test]
    fn test_detect_cloudflare_multiple_indicators() {
        let mut h = HashMap::new();
        h.insert("cf-ray".to_string(), "abc".to_string());
        h.insert("server".to_string(), "cloudflare".to_string());
        h.insert("cf-cache-status".to_string(), "HIT".to_string());
        assert!(detect_cloudflare(&h));
    }

    // === Bot protection detection tests ===

    #[test]
    fn test_bot_protection_cloudflare() {
        let mut h = HashMap::new();
        h.insert("cf-ray".to_string(), "abc".to_string());
        assert_eq!(detect_bot_protection(&h), BotProtection::Cloudflare { has_challenge: false });
    }

    #[test]
    fn test_bot_protection_cloudflare_with_challenge() {
        let mut h = HashMap::new();
        h.insert("cf-ray".to_string(), "abc".to_string());
        h.insert("cf-mitigated".to_string(), "challenge".to_string());
        assert_eq!(detect_bot_protection(&h), BotProtection::Cloudflare { has_challenge: true });
    }

    #[test]
    fn test_bot_protection_aws_waf() {
        let mut h = HashMap::new();
        h.insert("x-amzn-waf-action".to_string(), "block".to_string());
        assert_eq!(detect_bot_protection(&h), BotProtection::AwsWaf);
    }

    #[test]
    fn test_bot_protection_akamai() {
        let mut h = HashMap::new();
        h.insert("x-akamai-transformed".to_string(), "9".to_string());
        assert_eq!(detect_bot_protection(&h), BotProtection::Akamai);
    }

    #[test]
    fn test_bot_protection_none() {
        let mut h = HashMap::new();
        h.insert("server".to_string(), "nginx".to_string());
        assert_eq!(detect_bot_protection(&h), BotProtection::None);
        assert!(!BotProtection::None.is_protected());
        assert!(!BotProtection::None.needs_chrome_transport());
    }

    #[test]
    fn test_bot_protection_needs_chrome() {
        assert!(BotProtection::Cloudflare { has_challenge: false }.needs_chrome_transport());
        assert!(BotProtection::Akamai.needs_chrome_transport());
        assert!(!BotProtection::AwsWaf.needs_chrome_transport());
        assert!(!BotProtection::None.needs_chrome_transport());
    }

    // === Body-based challenge detection ===

    #[test]
    fn test_body_challenge_turnstile() {
        let body = "<script src=\"https://challenges.cloudflare.com/turnstile\"></script>";
        assert!(matches!(detect_bot_challenge_in_body(body), Some(BotProtection::Cloudflare { has_challenge: true })));
    }

    #[test]
    fn test_body_challenge_recaptcha() {
        let body = "<div class=\"g-recaptcha\" data-sitekey=\"abc\"></div><script src=\"https://www.google.com/recaptcha/api.js\"></script>";
        assert!(matches!(detect_bot_challenge_in_body(body), Some(BotProtection::Generic(ref s)) if s == "recaptcha"));
    }

    #[test]
    fn test_body_challenge_hcaptcha() {
        let body = "<div class=\"h-captcha\"></div><script src=\"https://hcaptcha.com/1/api.js\"></script>";
        assert!(matches!(detect_bot_challenge_in_body(body), Some(BotProtection::Generic(ref s)) if s == "hcaptcha"));
    }

    #[test]
    fn test_body_no_challenge() {
        let body = "<html><body><h1>Hello World</h1></body></html>";
        assert!(detect_bot_challenge_in_body(body).is_none());
    }

    #[test]
    fn test_body_challenge_cloudflare_just_a_moment() {
        let body = "<title>Just a moment...</title><div>cloudflare protection</div>";
        assert!(matches!(detect_bot_challenge_in_body(body), Some(BotProtection::Cloudflare { has_challenge: true })));
    }

    #[test]
    fn test_body_challenge_perimeterx() {
        let body = "<div id=\"px-captcha\">Please verify you are human</div>";
        assert!(matches!(detect_bot_challenge_in_body(body), Some(BotProtection::Generic(ref s)) if s == "perimeterx"));
    }
}
