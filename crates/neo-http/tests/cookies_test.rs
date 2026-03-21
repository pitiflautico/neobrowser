use neo_http::cookies::SqliteCookieStore;
use neo_http::CookieStore;

#[test]
fn test_store_and_retrieve() {
    let store = SqliteCookieStore::in_memory().unwrap();
    store.store_set_cookie(
        "https://example.com/page",
        "session=abc123; Path=/; Max-Age=3600",
    );
    let header = store.get_for_request("https://example.com/page", None, true);
    assert!(header.contains("session=abc123"), "got: {header}");
}

#[test]
fn test_samesite_strict_blocked_cross_site() {
    let store = SqliteCookieStore::in_memory().unwrap();
    store.store_set_cookie(
        "https://bank.com/login",
        "token=secret; Path=/; SameSite=Strict; Max-Age=3600",
    );

    // Same-site request: cookie should be present
    let same = store.get_for_request(
        "https://bank.com/api/data",
        Some("https://bank.com/"),
        false,
    );
    assert!(
        same.contains("token=secret"),
        "same-site should work: {same}"
    );

    // Cross-site request: cookie should be blocked
    let cross = store.get_for_request(
        "https://bank.com/api/data",
        Some("https://evil.com/phishing"),
        false,
    );
    assert!(
        !cross.contains("token=secret"),
        "cross-site strict should block: {cross}"
    );
}

#[test]
fn test_samesite_lax_allows_top_level_cross_site() {
    let store = SqliteCookieStore::in_memory().unwrap();
    store.store_set_cookie(
        "https://shop.com/",
        "pref=dark; Path=/; SameSite=Lax; Max-Age=3600",
    );

    // Cross-site top-level navigation: Lax allows
    let top = store.get_for_request("https://shop.com/", Some("https://referrer.com/"), true);
    assert!(
        top.contains("pref=dark"),
        "lax top-level should work: {top}"
    );

    // Cross-site sub-request: Lax blocks
    let sub = store.get_for_request("https://shop.com/api", Some("https://referrer.com/"), false);
    assert!(
        !sub.contains("pref=dark"),
        "lax sub-request should block: {sub}"
    );
}
