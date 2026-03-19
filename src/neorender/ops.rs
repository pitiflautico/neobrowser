//! NeoRender ops — Rust functions callable from JavaScript via deno_core.
//!
//! ALL OPS ARE SYNC to avoid deno_core 0.311 RefCell panic with concurrent async ops.
//! HTTP fetches run on dedicated threads. Timers use thread::sleep.

use deno_core::op2;
use deno_core::OpState;
use deno_core::error::AnyError;
use std::cell::RefCell;
use std::rc::Rc;

/// Fetch a URL. SYNC op — runs HTTP on a dedicated thread to avoid async conflicts.
/// Delegates to BrowserNetwork for proper Fetch Standard headers (Sec-Fetch-*, Referer, Origin).
/// Falls back to a fresh client if BrowserNetwork is not in OpState.
#[op2]
#[string]
pub fn op_neorender_fetch(
    state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[string] method: String,
    #[string] body: String,
    #[string] headers_json: String,
) -> Result<String, AnyError> {
    // Skip telemetry/analytics
    if url.contains("telemetry") || url.contains("analytics") || url.contains("tracking")
        || url.contains("beacon") || url.contains("sentry") || url.contains("newrelic")
        || url.contains("amplitude") || url.contains("segment.") || url.contains("hotjar")
        || url.contains("googletagmanager") || url.contains("doubleclick")
        || url.contains("apfc") {
        return Ok(r#"{"status":200,"body":"","headers":{}}"#.to_string());
    }

    eprintln!("[NEORENDER:FETCH] {} {}", method, &url[..url.len().min(100)]);

    // Rate limiting — per-domain token bucket (lazy-init if not in OpState)
    if let Some(domain) = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(|s| s.to_string())) {
        let handle: super::rate_limit::RateLimiterHandle = {
            let s = state.borrow();
            match s.try_borrow::<super::rate_limit::RateLimiterHandle>() {
                Some(h) => h.clone(),
                None => {
                    drop(s);
                    let h: super::rate_limit::RateLimiterHandle =
                        std::sync::Arc::new(std::sync::Mutex::new(super::rate_limit::RateLimiter::new()));
                    let cloned = h.clone();
                    state.borrow_mut().put::<super::rate_limit::RateLimiterHandle>(h);
                    cloned
                }
            }
        };
        let mut limiter = handle.lock().unwrap();
        limiter.wait_if_needed(&domain);
    }

    // Extract BrowserNetwork snapshot from OpState (borrow scope limited)
    // We clone the Arc<Client> and the origin/url strings — BrowserNetwork itself is not Send.
    let (client, net_origin, net_url, referrer_policy) = {
        let s = state.borrow();
        if let Some(net) = s.try_borrow::<super::net::BrowserNetworkHandle>() {
            (
                Some(net.client.clone()),
                net.origin.clone(),
                net.url.clone(),
                net.referrer_policy.clone(),
            )
        } else if let Some(c) = s.try_borrow::<super::session::SharedClient>() {
            // Backward compat: old SharedClient + PageOrigin
            let po = s.try_borrow::<super::session::PageOrigin>().cloned();
            (
                Some(c.clone()),
                po.as_ref().map(|p| p.origin.clone()).unwrap_or_default(),
                po.as_ref().map(|p| p.url.clone()).unwrap_or_default(),
                super::net::ReferrerPolicy::StrictOriginWhenCrossOrigin,
            )
        } else {
            (None, String::new(), String::new(), super::net::ReferrerPolicy::StrictOriginWhenCrossOrigin)
        }
    };

    let body_opt = if body.is_empty() { None } else { Some(body) };
    let headers_opt = if headers_json.is_empty() { None } else { Some(headers_json) };

    // Run HTTP on a dedicated thread (avoids deno_core async conflicts)
    let result = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Runtime: {e}"))?;

        rt.block_on(async {
            let client = match client {
                Some(c) => c,
                None => {
                    std::sync::Arc::new(
                        rquest::Client::builder()
                            .impersonate(rquest::Impersonate::Chrome131)
                            .cookie_store(true)
                            .redirect(rquest::redirect::Policy::limited(10))
                            .timeout(std::time::Duration::from_secs(15))
                            .build()
                            .map_err(|e| format!("Client: {e}"))?
                    )
                }
            };

            let network = super::net::BrowserNetwork::from_parts(
                client,
                &net_origin,
                &net_url,
                referrer_policy,
            );

            let resp = network.fetch(
                &url,
                &method,
                body_opt.as_deref(),
                headers_opt.as_deref(),
                super::net::RequestMode::Cors,
                super::net::RequestDestination::Empty,
            ).await?;

            eprintln!("[NEORENDER:FETCH] → {} ({}B)", resp.status, resp.body.len());

            let resp_headers: serde_json::Map<String, serde_json::Value> = resp.headers.into_iter()
                .map(|(k, v)| (k, serde_json::Value::String(v)))
                .collect();

            Ok(serde_json::json!({
                "status": resp.status,
                "body": resp.body,
                "headers": resp_headers,
            }).to_string())
        })
    }).join().unwrap_or_else(|_| Err("Thread panicked".to_string()));

    result.map_err(|e| deno_core::error::generic_error(e))
}

/// Sleep — SYNC to avoid async op conflicts in deno_core 0.311.
/// Capped at 100ms. < 5ms is no-op (animation frames).
#[op2(fast)]
pub fn op_neorender_timer(#[smi] ms: u32) -> () {
    if ms > 5 {
        std::thread::sleep(std::time::Duration::from_millis(ms.min(100) as u64));
    }
}

/// SHA-256 proof-of-work solver — native speed (~10M hash/s vs ~100K in JS).
/// Returns the nonce that produces a hash starting with `difficulty` prefix.
#[op2]
#[string]
pub fn op_neorender_pow(#[string] seed: String, #[string] difficulty: String, #[smi] max_iters: u32) -> Result<String, AnyError> {
    use std::fmt::Write;
    eprintln!("[NEORENDER:POW] seed={}... diff={} max={}", &seed[..seed.len().min(10)], difficulty, max_iters);
    let t0 = std::time::Instant::now();

    for i in 0..max_iters {
        let input = format!("{}{}", seed, i);
        // SHA-256
        let hash = {
            use std::collections::hash_map::DefaultHasher;
            // Use ring or manual SHA-256
            // Actually, let's use the sha2 crate... but it's not in deps.
            // Manual SHA-256 is complex. Use a simpler approach: call the system.
            // Actually, deno_core has crypto available. Let's just use a basic impl.

            // Inline SHA-256 (same algorithm as our JS version)
            sha256_hex(input.as_bytes())
        };

        // ChatGPT PoW: hash hex prefix must be <= difficulty (comparison, not exact match)
        if hash[..difficulty.len()] <= *difficulty {
            let elapsed = t0.elapsed();
            eprintln!("[NEORENDER:POW] Found nonce {} in {:?} ({} iters)", i, elapsed, i);
            return Ok(serde_json::json!({
                "found": true,
                "nonce": i,
                "hash": hash,
                "elapsed_ms": elapsed.as_millis() as u64,
            }).to_string());
        }
    }

    let elapsed = t0.elapsed();
    eprintln!("[NEORENDER:POW] Not found in {} iters ({:?})", max_iters, elapsed);
    Ok(serde_json::json!({
        "found": false,
        "elapsed_ms": elapsed.as_millis() as u64,
    }).to_string())
}

// Minimal SHA-256 implementation (no external crate needed)
fn sha256_hex(data: &[u8]) -> String {
    let hash = sha256(data);
    let mut hex = String::with_capacity(64);
    for b in &hash {
        use std::fmt::Write;
        write!(hex, "{:02x}", b).unwrap();
    }
    hex
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let k: [u32; 64] = [
        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19];

    // Padding
    let bit_len = (data.len() as u64) * 8;
    let pad_len = ((56u64.wrapping_sub(data.len() as u64 + 1) % 64) + 64) % 64;
    let total = data.len() as u64 + 1 + pad_len + 8;
    let mut padded = vec![0u8; total as usize];
    padded[..data.len()].copy_from_slice(data);
    padded[data.len()] = 0x80;
    padded[total as usize - 8..].copy_from_slice(&bit_len.to_be_bytes());

    // Process blocks
    for chunk in padded.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i*4], chunk[i*4+1], chunk[i*4+2], chunk[i*4+3]]);
        }
        for i in 16..64 {
            let s0 = w[i-15].rotate_right(7) ^ w[i-15].rotate_right(18) ^ (w[i-15] >> 3);
            let s1 = w[i-2].rotate_right(17) ^ w[i-2].rotate_right(19) ^ (w[i-2] >> 10);
            w[i] = w[i-16].wrapping_add(s0).wrapping_add(w[i-7]).wrapping_add(s1);
        }
        let mut a = h[0]; let mut b = h[1]; let mut c = h[2]; let mut d = h[3];
        let mut e = h[4]; let mut f = h[5]; let mut g = h[6]; let mut hh = h[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(k[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e; e = d.wrapping_add(t1); d = c; c = b; b = a; a = t1.wrapping_add(t2);
        }
        h[0]=h[0].wrapping_add(a); h[1]=h[1].wrapping_add(b); h[2]=h[2].wrapping_add(c); h[3]=h[3].wrapping_add(d);
        h[4]=h[4].wrapping_add(e); h[5]=h[5].wrapping_add(f); h[6]=h[6].wrapping_add(g); h[7]=h[7].wrapping_add(hh);
    }

    let mut result = [0u8; 32];
    for i in 0..8 {
        result[i*4..i*4+4].copy_from_slice(&h[i].to_be_bytes());
    }
    result
}

/// Log from JS console.
#[op2(fast)]
pub fn op_neorender_log(#[string] msg: String) {
    eprintln!("[NEORENDER:JS] {}", msg);
}

// ─── Storage ops (SQLite-backed localStorage) ───

/// Domain for storage ops — stored in OpState, set on navigation.
#[derive(Clone)]
pub struct StorageDomain(pub String);

/// Shared handle to BrowserStorage — stored in OpState.
pub struct StorageHandle(pub std::sync::Arc<super::storage::BrowserStorage>);

#[op2]
#[string]
pub fn op_storage_get(state: Rc<RefCell<OpState>>, #[string] key: String) -> Result<String, AnyError> {
    let s = state.borrow();
    let domain = s.try_borrow::<StorageDomain>().map(|d| d.0.clone()).unwrap_or_default();
    let handle = s.try_borrow::<StorageHandle>()
        .ok_or_else(|| deno_core::error::generic_error("No storage"))?;
    Ok(handle.0.get(&domain, &key).unwrap_or_default())
}

#[op2(fast)]
pub fn op_storage_set(state: Rc<RefCell<OpState>>, #[string] key: String, #[string] value: String) -> Result<(), AnyError> {
    let s = state.borrow();
    let domain = s.try_borrow::<StorageDomain>().map(|d| d.0.clone()).unwrap_or_default();
    let handle = s.try_borrow::<StorageHandle>()
        .ok_or_else(|| deno_core::error::generic_error("No storage"))?;
    handle.0.set(&domain, &key, &value).map_err(|e| deno_core::error::generic_error(e))
}

#[op2(fast)]
pub fn op_storage_remove(state: Rc<RefCell<OpState>>, #[string] key: String) -> Result<(), AnyError> {
    let s = state.borrow();
    let domain = s.try_borrow::<StorageDomain>().map(|d| d.0.clone()).unwrap_or_default();
    let handle = s.try_borrow::<StorageHandle>()
        .ok_or_else(|| deno_core::error::generic_error("No storage"))?;
    handle.0.remove(&domain, &key).map_err(|e| deno_core::error::generic_error(e))
}

#[op2(fast)]
pub fn op_storage_clear(state: Rc<RefCell<OpState>>) -> Result<(), AnyError> {
    let s = state.borrow();
    let domain = s.try_borrow::<StorageDomain>().map(|d| d.0.clone()).unwrap_or_default();
    let handle = s.try_borrow::<StorageHandle>()
        .ok_or_else(|| deno_core::error::generic_error("No storage"))?;
    handle.0.clear(&domain).map_err(|e| deno_core::error::generic_error(e))
}
