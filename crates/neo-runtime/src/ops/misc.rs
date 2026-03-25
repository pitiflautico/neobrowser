//! Miscellaneous ops — yield, sleep, PoW solver, microtask tick.

use deno_core::op2;

/// No-op async op that resolves immediately.
/// Used to force deno_core's event loop to do a full cycle (including
/// microtask checkpoint) when there are no other pending ops.
#[op2(async(lazy), fast)]
pub async fn op_microtask_tick() {
    // Resolves immediately — no tokio dependency
}

/// Minimal async op — tests async op integration.
#[op2(async(lazy), fast)]
pub async fn op_yield() -> () {
}

/// Async sleep — tests tokio reactor availability.
#[op2(async(lazy), fast)]
pub async fn op_sleep_ms(#[smi] ms: u32) -> () {
    if ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
    }
}

// ─── SHA-256 Proof-of-Work solver (native speed) ───

/// SHA-256 proof-of-work solver. Returns JSON with nonce and hash.
/// Used for ChatGPT's anti-bot challenge.
#[op2]
#[string]
pub fn op_pow_solve(
    #[string] seed: String,
    #[string] difficulty: String,
    #[smi] max_iters: u32,
) -> String {
    let t0 = std::time::Instant::now();
    let max = if max_iters == 0 { 500_000 } else { max_iters };

    for i in 0..max {
        let input = format!("{}{}", seed, i);
        let hash = sha256_hex(input.as_bytes());
        if hash[..difficulty.len()] <= *difficulty {
            let elapsed = t0.elapsed();
            return serde_json::json!({
                "found": true,
                "nonce": i,
                "hash": hash,
                "elapsed_ms": elapsed.as_millis() as u64,
            }).to_string();
        }
    }
    serde_json::json!({
        "found": false,
        "elapsed_ms": t0.elapsed().as_millis() as u64,
    }).to_string()
}

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
    let mut h: [u32; 8] = [
        0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,
        0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19,
    ];
    let bit_len = (data.len() as u64) * 8;
    let pad_len = ((56u64.wrapping_sub(data.len() as u64 + 1) % 64) + 64) % 64;
    let total = data.len() as u64 + 1 + pad_len + 8;
    let mut padded = vec![0u8; total as usize];
    padded[..data.len()].copy_from_slice(data);
    padded[data.len()] = 0x80;
    padded[total as usize - 8..].copy_from_slice(&bit_len.to_be_bytes());
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
        let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
        let (mut e, mut f, mut g, mut hh) = (h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(k[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e; e = d.wrapping_add(t1);
            d = c; c = b; b = a; a = t1.wrapping_add(t2);
        }
        h[0]=h[0].wrapping_add(a); h[1]=h[1].wrapping_add(b);
        h[2]=h[2].wrapping_add(c); h[3]=h[3].wrapping_add(d);
        h[4]=h[4].wrapping_add(e); h[5]=h[5].wrapping_add(f);
        h[6]=h[6].wrapping_add(g); h[7]=h[7].wrapping_add(hh);
    }
    let mut result = [0u8; 32];
    for i in 0..8 {
        result[i*4..i*4+4].copy_from_slice(&h[i].to_be_bytes());
    }
    result
}
