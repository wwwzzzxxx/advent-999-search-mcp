//! URL canonicalisation and result de-duplication.
//!
//! Scope: **exact-URL** de-duplication after canonicalisation. This catches
//! the common cases — the same page returned by several engines, or repeated
//! across pages of one engine — and, as a side effect, strips search-result
//! tracking cruft (CSDN URLs carry ~300 chars of it).
//!
//! Deliberately NOT done here: content-level de-duplication (the same article
//! reposted on another domain). That needs the page body fetched, which would
//! turn the fast search path into a slow one, and title-based heuristics merge
//! genuinely different pages. Showing a duplicate beats silently hiding a
//! distinct result.
//!
//! Tracking params are removed by **name allow/deny list**, never by dropping
//! the whole query string: `?p=2` on a bilibili video or `?id=` on any CMS is
//! part of the content identity, and blanket `split('?')` would merge distinct
//! pages (this is where the common Tavily-style recipe gets it wrong).

use std::collections::HashMap;

use crate::models::SearchResult;

/// Query-parameter names that are pure tracking/analytics noise.
const TRACKING_PARAMS: &[&str] = &[
    "ops_request_misc",
    "request_id",
    "biz_id",
    "spm",
    "spm_id_from",
    "fbclid",
    "gclid",
    "gclsrc",
    "msclkid",
    "yclid",
    "dclid",
    "twclid",
    "igshid",
    "mc_cid",
    "mc_eid",
    "_ga",
    "_gl",
    "share_token",
    "share_source",
    "share_medium",
    "wxshare",
    "vd_source",
    "ncid",
    "from",
    "from_source",
    "s_from",
    "ref",
    "referer",
    "referrer",
    "seid",
];

fn is_tracking_param(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("utm_") || n.starts_with("pk_") || TRACKING_PARAMS.contains(&n.as_str())
}

/// Canonical form used as the de-duplication key.
///
/// Not reversible and not meant to be shown to users — only compared.
pub fn canonical_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let unwrapped = unwrap_redirect(trimmed);
    let Ok(mut u) = url::Url::parse(&unwrapped) else {
        return unwrapped.to_ascii_lowercase();
    };

    // Fragments never identify a distinct document.
    u.set_fragment(None);

    // http/https are the same resource for de-duplication purposes.
    if u.scheme() == "http" {
        let _ = u.set_scheme("https");
    }

    // Drop a leading `www.` and lowercase the host.
    if let Some(host) = u.host_str().map(|h| h.to_ascii_lowercase()) {
        let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
        if !host.is_empty() {
            let _ = u.set_host(Some(&host));
        }
        // Drop default ports.
        if matches!(u.port(), Some(80) | Some(443)) {
            let _ = u.set_port(None);
        }
    }

    // Strip tracking params, keep anything else verbatim.
    if u.query().is_some() {
        let kept: Vec<(String, String)> = u
            .query_pairs()
            .filter(|(k, _)| !is_tracking_param(k))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if kept.is_empty() {
            u.set_query(None);
        } else {
            let mut ser = url::form_urlencoded::Serializer::new(String::new());
            for (k, v) in &kept {
                ser.append_pair(k, v);
            }
            u.set_query(Some(&ser.finish()));
        }
    }

    let mut out = u.to_string();
    // Normalise an empty path to `/`, drop a trailing slash elsewhere.
    if out.ends_with('/') && u.path() != "/" {
        out.pop();
    }
    out
}

/// Unwrap Bing's `/ck/a?...&u=a1<base64url>` click-tracker so the real target
/// participates in de-duplication. Sogou/WeChat `/link?url=...` redirects are
/// JS-resolved and carry a token, so they cannot be unwrapped statically.
fn unwrap_redirect(raw: &str) -> String {
    let Ok(u) = url::Url::parse(raw) else {
        return raw.to_string();
    };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    if !host.ends_with("bing.com") || !u.path().starts_with("/ck/") {
        return raw.to_string();
    }
    for (k, v) in u.query_pairs() {
        if k == "u" || k == "url" {
            if let Some(decoded) = decode_bing_u(&v) {
                if decoded.starts_with("http://") || decoded.starts_with("https://") {
                    return decoded;
                }
            }
        }
    }
    raw.to_string()
}

/// Decode Bing's `a1`-prefixed base64url target.
fn decode_bing_u(input: &str) -> Option<String> {
    let b64 = input.strip_prefix("a1").unwrap_or(input);
    let normalized = b64.replace('-', "+").replace('_', "/");
    let padded = match normalized.len() % 4 {
        2 => format!("{}==", normalized),
        3 => format!("{}=", normalized),
        _ => normalized,
    };
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut decoded = Vec::new();
    let bytes = padded.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i + 2] == b'=' || bytes[i + 3] == b'=' {
            break;
        }
        let a = CHARS.iter().position(|&c| c == bytes[i]).unwrap_or(0) as u8;
        let b = CHARS.iter().position(|&c| c == bytes[i + 1]).unwrap_or(0) as u8;
        let c = CHARS.iter().position(|&c| c == bytes[i + 2]).unwrap_or(0) as u8;
        let d = CHARS.iter().position(|&c| c == bytes[i + 3]).unwrap_or(0) as u8;
        decoded.push((a << 2) | (b >> 4));
        decoded.push(((b & 0x0f) << 4) | (c >> 2));
        decoded.push(((c & 0x03) << 6) | d);
        i += 4;
    }
    String::from_utf8(decoded).ok()
}

/// Result of a de-duplication pass.
pub struct DedupeOutcome {
    pub results: Vec<SearchResult>,
    /// How many duplicate entries were folded into an existing result.
    pub removed: usize,
}

/// Fold exact-duplicate URLs together, recording which engines each survivor
/// came from, then rank survivors by how many engines agreed on them (stable,
/// so original per-engine ordering is preserved within equal agreement).
pub fn dedupe(results: Vec<SearchResult>) -> DedupeOutcome {
    // First-seen order is preserved; a HashMap holds the survivors.
    let mut order: Vec<String> = Vec::with_capacity(results.len());
    let mut map: HashMap<String, SearchResult> = HashMap::with_capacity(results.len());
    let mut removed = 0usize;

    for (i, mut r) in results.into_iter().enumerate() {
        let key = canonical_url(&r.url);
        // URL-less results get a unique key so they flow through the same path
        // but can never collide with anything.
        let key = if key.is_empty() {
            format!("\u{0}no-url-{}", i)
        } else {
            key
        };

        match map.get_mut(&key) {
            Some(existing) => {
                removed += 1;
                if !existing.engines.contains(&r.engine) {
                    existing.engines.push(r.engine.clone());
                }
                // Keep the most informative variant of each text field.
                if r.description.chars().count() > existing.description.chars().count() {
                    existing.description = std::mem::take(&mut r.description);
                }
                if r.title.chars().count() > existing.title.chars().count() {
                    existing.title = std::mem::take(&mut r.title);
                }
                if existing.source.is_empty() {
                    existing.source = std::mem::take(&mut r.source);
                }
                if existing.summary.is_none() {
                    existing.summary = r.summary.take();
                }
            }
            None => {
                if r.engines.is_empty() {
                    r.engines = vec![r.engine.clone()];
                }
                order.push(key.clone());
                map.insert(key, r);
            }
        }
    }

    let mut out: Vec<SearchResult> = order
        .into_iter()
        .filter_map(|k| map.remove(&k))
        .collect();

    // Rank: more agreeing engines first. `sort_by` is stable, so results from
    // one engine keep their own relevance order.
    out.sort_by(|a, b| b.engines.len().cmp(&a.engines.len()));

    DedupeOutcome {
        results: out,
        removed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(engine: &str, url: &str) -> SearchResult {
        SearchResult {
            title: "t".into(),
            url: url.into(),
            description: "d".into(),
            source: "s".into(),
            engine: engine.into(),
            summary: None,
            engines: vec![engine.into()],
        }
    }

    #[test]
    fn strips_tracking_keeps_meaningful_params() {
        assert_eq!(
            canonical_url("https://blog.csdn.net/a/article/details/123?ops_request_misc=x&request_id=y&biz_id=0&utm_medium=z&utm_term=q"),
            "https://blog.csdn.net/a/article/details/123"
        );
        // bilibili multi-part: ?p=2 must survive.
        assert_eq!(
            canonical_url("https://www.bilibili.com/video/BV1x?p=2"),
            "https://bilibili.com/video/BV1x?p=2"
        );
        // mixed: tracking removed, meaningful kept.
        assert_eq!(
            canonical_url("https://example.com/p?id=9&utm_source=nl&fbclid=z"),
            "https://example.com/p?id=9"
        );
    }

    #[test]
    fn normalises_scheme_host_fragment_slash() {
        assert_eq!(
            canonical_url("http://www.Example.com/a/b/"),
            "https://example.com/a/b"
        );
        assert_eq!(
            canonical_url("https://example.com/a#section"),
            "https://example.com/a"
        );
        assert_eq!(canonical_url("https://example.com"), "https://example.com/");
    }

    #[test]
    fn dedupes_and_counts_engines() {
        let out = dedupe(vec![
            r("exa", "https://docs.python.org/"),
            r("exa", "https://other.com/x"),
            r("bing", "https://www.docs.python.org/"), // same after canonicalisation
        ]);
        assert_eq!(out.removed, 1);
        assert_eq!(out.results.len(), 2);
        // docs.python.org agreed by two engines -> first
        assert_eq!(out.results[0].url, "https://docs.python.org/");
        assert_eq!(out.results[0].engines, vec!["exa", "bing"]);
        assert_eq!(out.results[1].url, "https://other.com/x");
    }

    #[test]
    fn unwraps_bing_redirect() {
        // "https://python.org/" base64url, a1-prefixed.
        let wrapped = "https://www.bing.com/ck/a?!&&p=abc&u=a1aHR0cHM6Ly9weXRob24ub3JnLw";
        assert_eq!(canonical_url(wrapped), "https://python.org/");
    }
}
