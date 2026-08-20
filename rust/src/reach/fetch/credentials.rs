//! Deciding which headers may cross an origin boundary during a redirect.
//!
//! A redirect is the classic way credentials leak: a request to a permitted host answers
//! `302` to an attacker's, and a naive client forwards the Authorization header along with it.
//! So the origin is recorded when the request starts, and anything origin-sensitive is dropped
//! the moment the origin changes.

use super::CROSS_ORIGIN_SAFE_HEADERS;

/// Scheme + host + port, the unit credentials are scoped to. Compared as a whole
/// so an `https -> http` downgrade on the *same* host still counts as a different
/// origin — otherwise a redirect could replay a secure-only cookie in plaintext.
fn origin_of(url: &reqwest::Url) -> (String, String, u16) {
    (
        url.scheme().to_ascii_lowercase(),
        url.host_str().unwrap_or_default().to_ascii_lowercase(),
        url.port_or_known_default().unwrap_or(0),
    )
}

/// Is `name` safe to forward once we have left the original origin?
pub(in crate::reach) fn safe_cross_origin(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    CROSS_ORIGIN_SAFE_HEADERS.contains(&lower.as_str())
}

/// Tracks whether a redirect chain still sits on the origin the caller aimed at,
/// which is the only origin its credentials were meant for.
///
/// Split out of the fetch loop so the rule can be tested exhaustively without a
/// network: the SSRF guard rejects loopback, so a local test server could never
/// exercise this path. Leaving the origin is one-way — see [`Self::visit`].
#[derive(Debug)]
pub(in crate::reach) struct CredentialScope {
    first: Option<(String, String, u16)>,
    on_first_origin: bool,
}

impl CredentialScope {
    pub(in crate::reach) fn new(raw_url: &str) -> Self {
        Self {
            // An unparseable start URL yields None, which never equals a parsed
            // hop's origin, so credentials are withheld. Fail closed.
            first: reqwest::Url::parse(raw_url).ok().map(|u| origin_of(&u)),
            on_first_origin: true,
        }
    }

    /// Record arrival at `url`. Once the chain has left the caller's origin it
    /// stays "off origin" even if a later hop points back: by then the URL we
    /// would be returning to was chosen by the off-origin host, not the caller.
    pub(in crate::reach) fn visit(&mut self, url: &reqwest::Url) {
        if self.on_first_origin && self.first.as_ref() != Some(&origin_of(url)) {
            self.on_first_origin = false;
        }
    }

    /// May credentials (cookies, auth headers) go out on the current hop?
    pub(in crate::reach) fn allows_credentials(&self) -> bool {
        self.on_first_origin && self.first.is_some()
    }
}
