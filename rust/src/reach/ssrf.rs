//! The SSRF guard: which URLs may be fetched at all.
//!
//! Split out because it is the security-critical core of this module and deserves to be
//! readable on its own. It is also the part with the least to do with HTTP: it is pure
//! address classification, and the IPv6-disguise cases below (IPv4-mapped, 6to4, Teredo)
//! are each a documented way of smuggling a private address past a naive check.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};

/// SSRF guard: true only for a public http(s) URL with no userinfo whose host
/// resolves entirely to public addresses.
pub fn validate_url(raw: &str) -> bool {
    resolve_public_url(raw).is_some()
}

/// Full SSRF validation of `raw`. On success returns the parsed URL plus, for
/// domain hosts, the exact socket addresses validation checked — callers pin
/// them on the request (`ClientBuilder::resolve_to_addrs`) so an attacker
/// can't re-resolve to a private IP between validation and connect
/// (DNS-rebinding TOCTOU).
pub(super) fn resolve_public_url(raw: &str) -> Option<(reqwest::Url, Vec<SocketAddr>)> {
    let url = reqwest::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return None; // credentials-in-URL
    }
    let host = url.host_str()?;
    // A literal IP: check it directly (no DNS, nothing to pin). host_str keeps
    // the brackets on IPv6 literals ("[::1]") — strip them before parsing.
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return is_public(ip).then_some((url, Vec::new()));
    }
    // Block obvious internal names outright.
    let h = host.to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h == "metadata.google.internal" {
        return None;
    }
    // Resolve and require every address to be public.
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().ok()?.collect();
    if addrs.is_empty() || !addrs.iter().all(|a| is_public(a.ip())) {
        return None;
    }
    Some((url, addrs))
}

/// Reject loopback / private / link-local / unspecified / cloud-metadata IPs.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            // Loopback/unspecified/multicast first: to_ipv4() would otherwise
            // map ::1 to 0.0.0.1 and skip the loopback check.
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
                return false;
            }
            // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible (::a.b.c.d)
            // forms embed an IPv4 that must pass the same checks — otherwise
            // ::ffff:127.0.0.1 or ::ffff:a9fe:a9fe would sail through.
            if let Some(v4) = v6.to_ipv4() {
                return is_public_v4(v4);
            }
            let segs = v6.segments();
            // 6to4 (2002::/16): the next 32 bits are an IPv4 address.
            if segs[0] == 0x2002 {
                let v4 = Ipv4Addr::new(
                    (segs[1] >> 8) as u8,
                    segs[1] as u8,
                    (segs[2] >> 8) as u8,
                    segs[2] as u8,
                );
                return is_public_v4(v4);
            }
            // Teredo (2001:0000::/32): the last 32 bits are the IPv4 XOR all-ones.
            if segs[0] == 0x2001 && segs[1] == 0x0000 {
                let v4 = Ipv4Addr::new(
                    !(segs[6] >> 8) as u8,
                    !segs[6] as u8,
                    !(segs[7] >> 8) as u8,
                    !segs[7] as u8,
                );
                return is_public_v4(v4);
            }
            !(
                // unique-local fc00::/7
                (segs[0] & 0xfe00) == 0xfc00
                // link-local fe80::/10
                || (segs[0] & 0xffc0) == 0xfe80
            )
        }
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    if v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_documentation()
    {
        return false;
    }
    // Carrier-grade NAT 100.64.0.0/10 and metadata 169.254.169.254.
    let o = v4.octets();
    if o[0] == 100 && (64..=127).contains(&o[1]) {
        return false;
    }
    true
}
