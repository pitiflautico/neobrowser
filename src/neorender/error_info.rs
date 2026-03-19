//! NeoRender — Error Intelligence
//!
//! When something fails, give the AI context instead of cryptic errors.
//! Detects WAF blocks, auth requirements, rate limits, paywalls, captchas, etc.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ErrorInfo {
    pub blocked: bool,
    pub reason: String,
    pub error_type: String,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ErrorType {
    WafBlocked(String),
    AuthRequired,
    NotFound,
    ServerError(u16),
    Timeout,
    RateLimited,
    ConsentRequired,
    PaywallDetected,
    CaptchaRequired,
}

impl ErrorType {
    fn label(&self) -> String {
        match self {
            Self::WafBlocked(name) => format!("waf_blocked:{}", name.to_lowercase()),
            Self::AuthRequired => "auth_required".to_string(),
            Self::NotFound => "not_found".to_string(),
            Self::ServerError(code) => format!("server_error:{code}"),
            Self::Timeout => "timeout".to_string(),
            Self::RateLimited => "rate_limited".to_string(),
            Self::ConsentRequired => "consent_required".to_string(),
            Self::PaywallDetected => "paywall".to_string(),
            Self::CaptchaRequired => "captcha_required".to_string(),
        }
    }
}

impl ErrorInfo {
    pub fn from_response(status: u16, html: &str, url: &str) -> Option<Self> {
        // Only inspect first 10KB for performance
        let sample = &html[..html.len().min(10_000)];

        // 403 — WAF or auth
        if status == 403 {
            if sample.contains("cf-browser-verification") || sample.contains("_cf_chl_opt") || sample.contains("cf_chl_managed") {
                return Some(Self::waf("Cloudflare"));
            }
            if sample.contains("AwsWafIntegration") || sample.contains("aws-waf-token") {
                return Some(Self::waf("AWS WAF"));
            }
            if sample.contains("akamai") && sample.contains("challenge") {
                return Some(Self::waf("Akamai"));
            }
            if sample.contains("_dd_s") && sample.contains("challenge") {
                return Some(Self::waf("DataDome"));
            }
            if sample.contains("px-captcha") || sample.contains("_pxhd") {
                return Some(Self::waf("PerimeterX"));
            }
            return Some(Self::auth_required(url));
        }

        // 401
        if status == 401 {
            return Some(Self::auth_required(url));
        }

        // 429
        if status == 429 {
            return Some(Self::rate_limited());
        }

        // 404
        if status == 404 {
            return Some(Self::not_found(url));
        }

        // 500+
        if status >= 500 {
            return Some(Self::server_error(status));
        }

        // Content-level detection (even on 200)
        if sample.contains("g-recaptcha") || sample.contains("h-captcha") || sample.contains("cf-turnstile") {
            return Some(Self::captcha());
        }

        let lower = sample.to_lowercase();
        if lower.contains("paywall") || lower.contains("subscribe to continue")
            || lower.contains("premium content") || lower.contains("subscribe to read")
        {
            return Some(Self::paywall());
        }

        None
    }

    // ─── Constructors ───

    fn waf(name: &str) -> Self {
        let et = ErrorType::WafBlocked(name.to_string());
        Self {
            blocked: true,
            reason: format!("{name} WAF challenge detected"),
            error_type: et.label(),
            suggestions: vec![
                "Inject cookies from a real Chrome session".to_string(),
                "Switch to CDP/Chrome mode for this domain".to_string(),
                format!("This site uses {name} bot protection"),
            ],
        }
    }

    fn auth_required(url: &str) -> Self {
        let domain = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        let et = ErrorType::AuthRequired;
        Self {
            blocked: true,
            reason: format!("Authentication required for {domain}"),
            error_type: et.label(),
            suggestions: vec![
                "Login via Chrome mode first, then reuse cookies".to_string(),
                format!("Inject session cookies for {domain}"),
                "Check if there's an API endpoint that doesn't require auth".to_string(),
            ],
        }
    }

    fn rate_limited() -> Self {
        let et = ErrorType::RateLimited;
        Self {
            blocked: true,
            reason: "Rate limited (429 Too Many Requests)".to_string(),
            error_type: et.label(),
            suggestions: vec![
                "Wait before retrying (exponential backoff)".to_string(),
                "Reduce request frequency for this domain".to_string(),
                "Consider using a different IP or proxy".to_string(),
            ],
        }
    }

    fn not_found(url: &str) -> Self {
        let et = ErrorType::NotFound;
        Self {
            blocked: false,
            reason: format!("Page not found: {url}"),
            error_type: et.label(),
            suggestions: vec![
                "Check the URL for typos".to_string(),
                "The page may have been moved or deleted".to_string(),
                "Try searching the site for the content".to_string(),
            ],
        }
    }

    fn server_error(status: u16) -> Self {
        let et = ErrorType::ServerError(status);
        Self {
            blocked: false,
            reason: format!("Server error ({status})"),
            error_type: et.label(),
            suggestions: vec![
                "Retry after a short delay — may be temporary".to_string(),
                "The server may be overloaded or under maintenance".to_string(),
            ],
        }
    }

    fn captcha() -> Self {
        let et = ErrorType::CaptchaRequired;
        Self {
            blocked: true,
            reason: "CAPTCHA challenge detected on page".to_string(),
            error_type: et.label(),
            suggestions: vec![
                "Switch to Chrome mode for manual CAPTCHA solve".to_string(),
                "Inject pre-authenticated cookies to bypass".to_string(),
                "Try accessing the site's API directly".to_string(),
            ],
        }
    }

    fn paywall() -> Self {
        let et = ErrorType::PaywallDetected;
        Self {
            blocked: true,
            reason: "Content behind paywall".to_string(),
            error_type: et.label(),
            suggestions: vec![
                "Check for cached/archive version (web.archive.org)".to_string(),
                "Login with a subscribed account".to_string(),
                "Look for the same content on a free source".to_string(),
            ],
        }
    }
}
