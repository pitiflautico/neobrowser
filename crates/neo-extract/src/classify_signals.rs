//! Extended page classification signals — pricing, documentation, profile, settings.

use neo_dom::DomEngine;

use super::classify::PageType;

/// Pricing page: pricing tables, price patterns, plan/tier keywords.
pub(crate) fn check_pricing_page(
    dom: &dyn DomEngine,
    candidates: &mut Vec<(PageType, f32, Vec<String>)>,
) {
    let mut features = Vec::new();
    let mut score: f32 = 0.0;

    let title = dom.title().to_lowercase();
    let pricing_keywords = ["pricing", "plans", "prices", "precios", "tarif"];
    for kw in &pricing_keywords {
        if title.contains(kw) {
            score += 0.5;
            features.push(format!("title contains '{kw}'"));
            break;
        }
    }

    let body_text = dom
        .query_selector("body")
        .map(|el| dom.text_content(el))
        .unwrap_or_default();
    let body_lower = body_text.to_lowercase();

    let price_count = count_price_patterns(&body_text);
    if price_count >= 2 {
        score += 0.3;
        features.push(format!("{price_count} price patterns found"));
    }

    let plan_keywords = [
        "free plan",
        "pro plan",
        "enterprise",
        "basic",
        "premium",
        "starter",
        "/month",
        "/mo",
        "/year",
        "/yr",
    ];
    let mut plan_count = 0;
    for kw in &plan_keywords {
        if body_lower.contains(kw) {
            plan_count += 1;
        }
    }
    if plan_count >= 2 {
        score += 0.3;
        features.push(format!("{plan_count} plan/tier keywords"));
    }

    if score > 0.0 {
        candidates.push((PageType::Pricing, score.min(1.0), features));
    }
}

/// Documentation page: code blocks, nav with TOC-like links, breadcrumbs.
pub(crate) fn check_documentation(
    dom: &dyn DomEngine,
    candidates: &mut Vec<(PageType, f32, Vec<String>)>,
) {
    let mut features = Vec::new();
    let mut score: f32 = 0.0;

    let title = dom.title().to_lowercase();
    let doc_keywords = ["docs", "documentation", "reference", "api", "guide"];
    for kw in &doc_keywords {
        if title.contains(kw) {
            score += 0.4;
            features.push(format!("title contains '{kw}'"));
            break;
        }
    }

    let code_blocks = dom.query_selector_all("code");
    let pre_blocks = dom.query_selector_all("pre");
    let code_count = code_blocks.len() + pre_blocks.len();
    if code_count > 2 {
        score += 0.3;
        features.push(format!("{code_count} code/pre blocks"));
    }

    let navs = dom.query_selector_all("nav");
    if !navs.is_empty() && code_count > 0 {
        score += 0.1;
        features.push("nav + code blocks".to_string());
    }

    if score > 0.0 {
        candidates.push((PageType::Documentation, score.min(1.0), features));
    }
}

/// Profile page: avatar image, user details, bio.
pub(crate) fn check_profile(
    dom: &dyn DomEngine,
    candidates: &mut Vec<(PageType, f32, Vec<String>)>,
) {
    let mut features = Vec::new();
    let mut score: f32 = 0.0;

    let title = dom.title().to_lowercase();
    let profile_keywords = ["profile", "perfil", "account", "user"];
    for kw in &profile_keywords {
        if title.contains(kw) {
            score += 0.4;
            features.push(format!("title contains '{kw}'"));
            break;
        }
    }

    let imgs = dom.query_selector_all("img");
    for &img in &imgs {
        let alt = dom
            .get_attribute(img, "alt")
            .unwrap_or_default()
            .to_lowercase();
        let src = dom
            .get_attribute(img, "src")
            .unwrap_or_default()
            .to_lowercase();
        if alt.contains("avatar")
            || alt.contains("profile")
            || src.contains("avatar")
            || src.contains("profile")
        {
            score += 0.3;
            features.push("avatar/profile image".to_string());
            break;
        }
    }

    if score > 0.0 {
        candidates.push((PageType::Profile, score.min(1.0), features));
    }
}

/// Settings page: toggles, checkboxes, save buttons.
pub(crate) fn check_settings(
    dom: &dyn DomEngine,
    candidates: &mut Vec<(PageType, f32, Vec<String>)>,
) {
    let mut features = Vec::new();
    let mut score: f32 = 0.0;

    let title = dom.title().to_lowercase();
    let settings_keywords = [
        "settings",
        "preferences",
        "configuration",
        "ajustes",
        "configuraci",
    ];
    for kw in &settings_keywords {
        if title.contains(kw) {
            score += 0.5;
            features.push(format!("title contains '{kw}'"));
            break;
        }
    }

    let inputs = dom.get_inputs();
    let checkbox_count = inputs
        .iter()
        .filter(|&&el| dom.get_attribute(el, "type").as_deref() == Some("checkbox"))
        .count();
    if checkbox_count >= 3 {
        score += 0.3;
        features.push(format!("{checkbox_count} checkboxes"));
    }

    if score > 0.0 {
        candidates.push((PageType::Settings, score.min(1.0), features));
    }
}

/// Check if text contains price-like patterns ($X, EUR X, etc.).
pub(crate) fn has_price_pattern(text: &str) -> bool {
    count_price_patterns(text) > 0
}

/// Count price pattern occurrences in text.
pub(crate) fn count_price_patterns(text: &str) -> usize {
    let mut count = 0;
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    for i in 0..len {
        let ch = chars[i];
        if (ch == '$' || ch == '\u{20ac}' || ch == '\u{00a3}')
            && i + 1 < len
            && chars[i + 1].is_ascii_digit()
        {
            count += 1;
        }
    }
    count
}
