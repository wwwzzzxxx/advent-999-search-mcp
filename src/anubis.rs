//! Anubis proof-of-work anti-bot solver.
//!
//! Sites behind [Anubis](https://anubis.techaro.lol) answer automated clients
//! with a JS challenge page instead of the requested content:
//!
//! ```html
//! <script id="anubis_version" type="application/json">"v1.27.0"</script>
//! <script id="anubis_challenge" type="application/json">{...}</script>
//! ```
//!
//! The browser script brute-forces a nonce such that
//! `sha256hex(randomData + nonce)` starts with `difficulty` zero **hex digits**,
//! then GETs `/api/pass-challenge`, which sets the auth cookie. We replicate
//! exactly that, so the subsequent request looks like a normal browser session.
//!
//! Note the difficulty unit: the JS computes `Math.pow(16, -difficulty)`, i.e.
//! it counts hex digits, not bits. Difficulty 4 averages ~65k hashes (~1 ms),
//! difficulty 6 averages ~16.7M (~100-300 ms in Rust). Verified against
//! dblp.org (difficulty 4) and startpage.com (difficulty 6).
//!
//! 2026-09: dblp.org, dblp.uni-trier.de and startpage.com all deploy Anubis.

use sha2::{Digest, Sha256};

/// Path of the endpoint that exchanges a solved challenge for an auth cookie.
/// It is prefixed by the site's Anubis base prefix (`anubis_base_prefix`).
const ANUBIS_PASS_PATH: &str = "/.within.website/x/cmd/anubis/api/pass-challenge";

/// A parsed Anubis challenge.
#[derive(Debug, Clone)]
pub struct Challenge {
    /// Opaque challenge id, echoed back to the pass-challenge endpoint.
    pub id: String,
    /// Hex-encoded random data used as the PoW prefix.
    pub random_data: String,
    /// Number of leading zero **hex digits** the SHA-256 digest must have.
    pub difficulty: u32,
    /// URL path prefix the site runs Anubis under (usually empty).
    pub base_prefix: String,
}

/// Detect an Anubis challenge in a response body.
///
/// Returns `None` when the body is real content (or a non-Anubis block page),
/// so callers can use this as a cheap "am I blocked?" test.
pub fn detect(body: &str) -> Option<Challenge> {
    if !body.contains("anubis_challenge") {
        return None;
    }
    let raw = json_script(body, "anubis_challenge")?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let ch = &v["challenge"];

    let difficulty = v["rules"]["difficulty"]
        .as_u64()
        .or_else(|| ch["difficulty"].as_u64())
        .unwrap_or(5) as u32;

    let base_prefix = json_script(body, "anubis_base_prefix")
        .and_then(|s| serde_json::from_str::<String>(&s).ok())
        .unwrap_or_default();

    Some(Challenge {
        id: ch["id"].as_str()?.to_string(),
        random_data: ch["randomData"].as_str()?.to_string(),
        difficulty,
        base_prefix,
    })
}

/// Read the payload of `<script id="<id>" type="application/json">…</script>`.
fn json_script(html: &str, id: &str) -> Option<String> {
    let marker = format!("id=\"{}\"", id);
    let at = html.find(&marker)?;
    let open_end = html[at..].find('>')? + at + 1;
    let close = html[open_end..].find("</script>")? + open_end;
    Some(html[open_end..close].trim().to_string())
}

/// Brute-force the nonce: some `n` where `sha256hex(randomData + n)` has
/// `difficulty` leading zero hex digits.
///
/// Pure CPU work — call it from `spawn_blocking` when inside async code.
/// The search is spread over all available cores: a difficulty-6 challenge
/// expects ~16.7M hashes, which is a few seconds single-threaded but well
/// under a second in parallel.
pub fn solve(challenge: &Challenge) -> (u64, String) {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 16);
    let stop = std::sync::atomic::AtomicBool::new(false);
    let slot: std::sync::Mutex<Option<(u64, String)>> = std::sync::Mutex::new(None);

    std::thread::scope(|scope| {
        for worker in 0..threads {
            let stop = &stop;
            let slot = &slot;
            scope.spawn(move || {
                // Hash randomData once per worker, then clone that mid-state for
                // each attempt — SHA-256 is streaming, so this skips re-hashing
                // the (128-byte) prefix on every single iteration.
                let mut base = Sha256::new();
                base.update(challenge.random_data.as_bytes());
                let prefix = "0".repeat(challenge.difficulty as usize);
                let mut buf = [0u8; 20];

                for nonce in (worker as u64..).step_by(threads) {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let len = write_decimal(&mut buf, nonce);
                    let mut hasher = base.clone();
                    hasher.update(&buf[..len]);
                    let digest = hex(&hasher.finalize());
                    if digest.starts_with(&prefix) {
                        stop.store(true, std::sync::atomic::Ordering::Relaxed);
                        if let Ok(mut g) = slot.lock() {
                            if g.is_none() {
                                *g = Some((nonce, digest));
                            }
                        }
                        return;
                    }
                }
            });
        }
    });

    slot.into_inner()
        .ok()
        .flatten()
        .unwrap_or_else(|| panic!("anubis: no nonce found for difficulty {}",
                                  challenge.difficulty))
}

/// Write `n` as decimal into the start of `buf`, returning the byte count.
/// Avoids the allocation `n.to_string()` would make in the hot loop.
fn write_decimal(buf: &mut [u8; 20], mut n: u64) -> usize {
    if n == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    buf.copy_within(i.., 0);
    buf.len() - i
}

/// Lowercase hex encoding (avoids pulling in the `hex` crate).
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Solve the challenge and hand it to Anubis, which stores the auth cookie in
/// the client's cookie jar.
///
/// `client` **must** have `cookie_store(true)` — the authentication lives in a
/// `Set-Cookie` on the pass-challenge response, so without a cookie jar the
/// follow-up request would be challenged again.
///
/// `origin` is the scheme+host of the site; `redir` is where Anubis should
/// send the client afterwards (a benign `origin + "/"` is fine — we replay our
/// own request ourselves).
pub async fn pass(
    client: &reqwest::Client,
    origin: &str,
    challenge: &Challenge,
    redir: &str,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    let owned = challenge.clone();
    let (nonce, response) = tokio::task::spawn_blocking(move || solve(&owned))
        .await
        .map_err(|e| format!("anubis: solver panicked: {}", e))?;

    // Anubis records this for its own statistics; report the real wall time.
    let elapsed_ms = started.elapsed().as_millis().max(1);

    let url = format!(
        "{}{}{}?id={}&response={}&nonce={}&redir={}&elapsedTime={}",
        origin,
        challenge.base_prefix,
        ANUBIS_PASS_PATH,
        url_encode(&challenge.id),
        response,
        nonce,
        url_encode(redir),
        elapsed_ms,
    );

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("anubis: pass-challenge request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("anubis: pass-challenge returned HTTP {}", status));
    }

    eprintln!(
        "✅ anubis: solved difficulty={} in {}ms (nonce={})",
        challenge.difficulty, elapsed_ms, nonce
    );
    Ok(())
}

/// Percent-encode a query-string component (RFC 3986 unreserved set kept as-is).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Scheme + host of a URL, e.g. `https://dblp.org` from
/// `https://dblp.org/search/publ/api?q=x`. Used as the Anubis origin.
pub fn origin_of(url: &str) -> String {
    let after_scheme = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => return url.to_string(),
    };
    let end = after_scheme.find('/').unwrap_or(after_scheme.len());
    let scheme_end = url.find("://").unwrap() + 3;
    url[..scheme_end + end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_challenge_and_solves_it() {
        // difficulty 1 => first hex digit must be '0'; trivially findable.
        let html = r#"<html><script id="anubis_version" type="application/json">"v1.27.0"</script>
<script id="anubis_base_prefix" type="application/json">""</script>
<script id="anubis_challenge" type="application/json">{"rules":{"algorithm":"fast","difficulty":2},"challenge":{"id":"abc-123","method":"fast","randomData":"deadbeef","difficulty":2,"spent":false}}</script></html>"#;

        let ch = detect(html).expect("challenge should be detected");
        assert_eq!(ch.id, "abc-123");
        assert_eq!(ch.random_data, "deadbeef");
        assert_eq!(ch.difficulty, 2);
        assert_eq!(ch.base_prefix, "");

        let (nonce, digest) = solve(&ch);
        assert!(digest.starts_with("00"), "digest {} lacks 2 zero nibbles", digest);

        // Re-derive independently to prove the nonce really satisfies the PoW.
        let mut h = Sha256::new();
        h.update(ch.random_data.as_bytes());
        h.update(nonce.to_string().as_bytes());
        assert_eq!(hex(&h.finalize()), digest);
    }

    #[test]
    fn ignores_ordinary_pages() {
        assert!(detect("<html><body>hello</body></html>").is_none());
        // Mentions the name without an actual challenge payload.
        assert!(detect("<p>anubis_challenge docs</p>").is_none());
    }

    #[test]
    fn extracts_origin() {
        assert_eq!(origin_of("https://dblp.org/search/publ/api?q=x"), "https://dblp.org");
        assert_eq!(origin_of("https://www.startpage.com/sp/search"), "https://www.startpage.com");
        assert_eq!(origin_of("https://dblp.uni-trier.de/"), "https://dblp.uni-trier.de");
    }
}
