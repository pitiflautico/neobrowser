//! Server-side fetching: redirect following, credential scoping, and HTML reduction.
//!
//! The credential-scoping rule lives here and is the reason this file exists separately:
//! a cookie or an auth header must not survive a redirect off the origin the caller
//! asked for, including an `https` → `http` downgrade on the same host.
//!
//! Split into [`credentials`] (which headers may cross an origin boundary), [`get`] (the
//! guarded request, bounded in redirects, size and time) and [`text`] (turning HTML into
//! readable text without an HTML parser).

pub mod credentials;
pub mod get;
pub mod text;

pub(super) use get::{guarded_get, read_capped};
pub use text::browse;

/// Max redirects followed manually (each hop re-runs the full SSRF guard).
const MAX_REDIRECTS: u32 = 5;

/// Caller-supplied headers that stay on a cross-origin redirect.
///
/// Deliberately an allowlist, not a blocklist of "sensitive" names. A blocklist
/// has to guess every way a secret can be spelled — `Authorization`, `Cookie`,
/// `X-Api-Key`, `X-Acme-Session`, … — and a header it has never heard of gets
/// forwarded to whatever host the redirect names. Reversing the default means an
/// unknown header is treated as a possible secret, which is what it usually is:
/// these headers are set by a model or a config file precisely to authenticate.
/// What remains here is content negotiation, which carries nothing private.
const CROSS_ORIGIN_SAFE_HEADERS: &[&str] =
    &["accept", "accept-charset", "accept-language", "user-agent"];
