//! General bot-wall / challenge / consent detection.
//!
//! Anti-bot friction is a *class* of problem we hit across many sites, not a
//! per-site quirk. This module detects it generically from the URL + rendered page
//! so the rest of the system can react (skip a walled search provider, surface a
//! wall on `navigate`, decide to retry with a real profile, etc.).

use serde::Serialize;

use crate::cdp::CdpClient;
use crate::page;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Wall {
    /// A hard bot wall (Google /sorry/, "unusual traffic").
    BotWall,
    /// An interactive challenge (Cloudflare Turnstile, reCAPTCHA, "verify you are human").
    Captcha,
    /// A cookie/consent gate blocking content.
    Consent,
    /// Rate limiting ("too many requests", 429).
    RateLimited,
    /// A login gate.
    LoginRequired,
    /// A generic access/server error page.
    Error,
}

impl Wall {
    pub fn as_str(self) -> &'static str {
        match self {
            Wall::BotWall => "bot_wall",
            Wall::Captcha => "captcha",
            Wall::Consent => "consent",
            Wall::RateLimited => "rate_limited",
            Wall::LoginRequired => "login_required",
            Wall::Error => "error",
        }
    }

    /// Which action status this wall implies.
    ///
    /// The distinction that matters: a captcha or a login gate needs *a person*,
    /// while a bot wall or a rate limit needs a different approach (another source,
    /// a warmed profile, backing off). Collapsing both into "failed" is what makes a
    /// model retry the same blocked request in a loop.
    pub fn action_status(self) -> crate::action::ActionStatus {
        match self {
            Wall::Captcha | Wall::LoginRequired => crate::action::ActionStatus::NeedsHuman,
            Wall::BotWall | Wall::RateLimited | Wall::Consent | Wall::Error => {
                crate::action::ActionStatus::Blocked
            }
        }
    }

    /// A short, actionable hint for the model.
    pub fn hint(self) -> &'static str {
        match self {
            Wall::BotWall => "site served a bot wall; retry with NEOBROWSER_REAL_PROFILE (a warmed, logged-in profile) or a different source",
            Wall::Captcha => "an interactive challenge is present; a real profile or human handoff may be needed",
            Wall::Consent => "a cookie/consent gate is blocking content; try dismiss_overlay",
            Wall::RateLimited => "the site is rate-limiting; slow down or retry later",
            Wall::LoginRequired => "this page requires login; use the login tool or a real profile",
            Wall::Error => "the page returned an access/server error",
        }
    }
}

/// Signals gathered cheaply from the live page in one JS round-trip.
struct Signals {
    url: String,
    title: String,
    text: String,
    has_captcha_frame: bool,
    has_password_field: bool,
    has_cf_challenge: bool,
}

async fn gather(client: &CdpClient) -> Signals {
    // One evaluation collects everything, so detection is a single round-trip.
    let v = page::eval_body(client, &crate::js::wall_signals().returning())
        .await
        .ok();
    let obj = v.and_then(|v| match v {
        serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(&s).ok(),
        other => Some(other),
    });
    let get_str = |k: &str| {
        obj.as_ref()
            .and_then(|o| o.get(k))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    let get_bool = |k: &str| {
        obj.as_ref()
            .and_then(|o| o.get(k))
            .and_then(|x| x.as_bool())
            .unwrap_or(false)
    };
    Signals {
        url: get_str("url"),
        title: get_str("title"),
        text: get_str("text"),
        has_captcha_frame: get_bool("captcha"),
        has_password_field: get_bool("password"),
        has_cf_challenge: get_bool("cf"),
    }
}

/// Detect a wall on the current page, or `None` if the content looks reachable.
pub async fn detect(client: &CdpClient) -> Option<Wall> {
    classify(&gather(client).await)
}

fn classify(s: &Signals) -> Option<Wall> {
    let url = s.url.to_lowercase();
    let hay = format!("{} {}", s.title, s.text).to_lowercase();

    // URL-level tells first (cheapest, most reliable).
    if url.contains("/sorry/") {
        return Some(Wall::BotWall);
    }
    if url.contains("consent.") || url.contains("/consent") {
        return Some(Wall::Consent);
    }

    // Interactive challenges.
    if s.has_captcha_frame
        || s.has_cf_challenge
        || contains_any(
            &hay,
            &[
                "verify you are human",
                "i'm not a robot",
                "are you a robot",
                "complete the captcha",
                "press and hold",
                "recaptcha",
                "hcaptcha",
                "cloudflare",
            ],
        )
    {
        return Some(Wall::Captcha);
    }

    if contains_any(
        &hay,
        &[
            "unusual traffic",
            "automated queries",
            "our systems have detected",
        ],
    ) {
        return Some(Wall::BotWall);
    }
    if contains_any(
        &hay,
        &["too many requests", "rate limit", "429", "slow down"],
    ) {
        return Some(Wall::RateLimited);
    }
    if contains_any(
        &hay,
        &[
            "access denied",
            "forbidden",
            "403",
            "error 5",
            "service unavailable",
        ],
    ) {
        return Some(Wall::Error);
    }
    // A password field plus login-y language, and little else, reads as a login gate.
    if s.has_password_field
        && contains_any(
            &hay,
            &[
                "sign in",
                "log in",
                "login",
                "iniciar sesión",
                "inicia sesión",
            ],
        )
    {
        return Some(Wall::LoginRequired);
    }
    None
}

fn contains_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(url: &str, title: &str, text: &str) -> Signals {
        Signals {
            url: url.into(),
            title: title.into(),
            text: text.into(),
            has_captcha_frame: false,
            has_password_field: false,
            has_cf_challenge: false,
        }
    }

    #[test]
    fn detects_google_sorry_wall() {
        assert_eq!(
            classify(&sig(
                "https://www.google.com/sorry/index?continue=x",
                "",
                ""
            )),
            Some(Wall::BotWall)
        );
    }

    #[test]
    fn detects_unusual_traffic() {
        let s = sig(
            "https://x.com",
            "",
            "Our systems have detected unusual traffic from your computer network.",
        );
        assert_eq!(classify(&s), Some(Wall::BotWall));
    }

    #[test]
    fn detects_cloudflare_challenge() {
        let mut s = sig(
            "https://site.com",
            "Just a moment...",
            "Checking your browser before accessing",
        );
        s.has_cf_challenge = true;
        assert_eq!(classify(&s), Some(Wall::Captcha));
    }

    #[test]
    fn detects_rate_limit_and_error() {
        assert_eq!(
            classify(&sig("https://a.com", "", "Too many requests, slow down")),
            Some(Wall::RateLimited)
        );
        assert_eq!(
            classify(&sig("https://a.com", "", "403 Forbidden: access denied")),
            Some(Wall::Error)
        );
    }

    #[test]
    fn detects_login_gate() {
        let mut s = sig(
            "https://a.com/login",
            "Sign in",
            "Please log in to continue",
        );
        s.has_password_field = true;
        assert_eq!(classify(&s), Some(Wall::LoginRequired));
    }

    #[test]
    fn clean_page_is_not_a_wall() {
        assert_eq!(
            classify(&sig(
                "https://example.com",
                "Example Domain",
                "This domain is for examples."
            )),
            None
        );
    }
}
