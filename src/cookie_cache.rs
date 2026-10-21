//! Local cookie store, scoped **by domain**.
//!
//! Two kinds of credentials end up here:
//! - long-lived **login** cookies (Bilibili `SESSDATA`, Zhihu `d_c0`), either
//!   pasted from a browser (`set_cookies`) or imported from a dedicated Edge
//!   profile (`import_cookies`, opt-in);
//! - short-lived **anti-bot** cookies (Sogou `SNUID`), refreshed without a
//!   server restart.
//!
//! Every entry is keyed by the domain it may be sent to. Requests ask for the
//! cookies of *their own host* (`cookies_for_host`), so a Bilibili session can
//! never leak to an unrelated site — which is exactly what a flat
//! `FETCH_COOKIES` string used to do.
//!
//! **The file lives outside the repository** (`%LOCALAPPDATA%\advent-mcp\` on
//! Windows, `~/.cache/advent-mcp/` elsewhere) and must never be committed.
//!
//! Layout (v2):
//!
//! ```json
//! {
//!   "version": 2,
//!   "entries": {
//!     "sogou.com": { "cookies": "SNUID=...; SUV=...", "saved_at": 1789000000, "source": "paste" },
//!     "bilibili.com": { "cookies": "SESSDATA=...", "saved_at": 1789000000, "source": "browser" }
//!   }
//! }
//! ```
//!
//! The v1 layout (`{ "sogou": { ... } }`, no version) is migrated on read.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const STORE_VERSION: u32 = 2;

/// One entry: a `Cookie:` header value plus provenance.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Entry {
    pub cookies: String,
    /// Unix seconds when the cookies were captured.
    #[serde(default)]
    pub saved_at: u64,
    /// `paste` (browser DevTools / set_cookies) or `browser` (import_cookies).
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "paste".to_string()
}

type Entries = HashMap<String, Entry>;

/// Raw file contents plus the mtime they came from, so we can skip re-reading
/// (and re-parsing) the file on every single request.
static FILE_CACHE: Mutex<Option<(SystemTime, Entries)>> = Mutex::new(None);

/// Location of the cookie file. Kept outside the repo: this holds live
/// session credentials.
pub fn cache_path() -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    } else {
        std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                PathBuf::from(home).join(".cache")
            })
    };
    base.join("advent-mcp").join("cookie-cache.json")
}

/// Directory the browser import keeps its private Edge profile in.
pub fn browser_profile_dir() -> PathBuf {
    cache_path()
        .parent()
        .map(|p| p.join("edge-profile"))
        .unwrap_or_else(|| PathBuf::from("edge-profile"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Domain helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Does `host` belong to `domain` (equal or a subdomain)?
///
/// Deliberately one-directional: an entry for `com` must never match
/// `example.com`, so the caller's entry is the *suffix*, never the other way
/// around.
pub fn domain_matches(host: &str, domain: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_lowercase();
    let domain = domain.trim().trim_start_matches('.').trim_end_matches('.').to_lowercase();
    // A single-label entry ("com") would match every host under it; refuse.
    if domain.is_empty() || !domain.contains('.') {
        return false;
    }
    host == domain || host.ends_with(&format!(".{}", domain))
}

/// Normalize user input to a bare, lower-case domain.
///
/// Accepts `https://www.sogou.com/x?y=1`, `.sogou.com`, `SOGOU.COM:443`.
/// Rejects wildcards, paths-only input, single-label names and anything with
/// characters that have no business in a cookie domain.
pub fn normalize_domain(input: &str) -> Option<String> {
    let raw = input.trim();
    if raw.is_empty() || raw.contains('*') {
        return None;
    }
    let mut d = raw.to_lowercase();
    for scheme in ["https://", "http://", "//"] {
        if let Some(rest) = d.strip_prefix(scheme) {
            d = rest.to_string();
            break;
        }
    }
    // Drop path / query / fragment, then port.
    if let Some(idx) = d.find(['/', '?', '#']) {
        d.truncate(idx);
    }
    if let Some((host, port)) = d.rsplit_once(':') {
        if !host.contains(':') && port.chars().all(|c| c.is_ascii_digit()) {
            d = host.to_string();
        }
    }
    let d = d.trim_matches('.').to_string();
    if d.is_empty() || d.len() > 253 {
        return None;
    }
    let labels: Vec<&str> = d.split('.').collect();
    if labels.len() < 2 {
        return None;
    }
    for label in &labels {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return None;
        }
    }
    let tld = labels.last().unwrap();
    if tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(d)
}

/// Map the short names used by engines/tools (`sogou`, `bilibili`) onto the
/// domain that actually owns the cookies. Anything already looking like a
/// domain is normalized and returned as-is.
pub fn canonical_domain(site: &str) -> Option<String> {
    let s = site.trim().to_lowercase();
    let mapped = match s.as_str() {
        // Sogou search and WeChat article search share one trust bucket and
        // one SNUID issued by sogou.com.
        "sogou" | "weixin" | "weixin_sogou" | "sogou_weixin" => "sogou.com",
        "bilibili" | "bili" => "bilibili.com",
        "zhihu" => "zhihu.com",
        "csdn" => "csdn.net",
        "juejin" => "juejin.cn",
        "xiaohongshu" | "xhs" => "xiaohongshu.com",
        _ => s.as_str(),
    };
    normalize_domain(mapped)
}

/// Cookie name → owning domain, for seeding the store from the legacy
/// `FETCH_COOKIES` env var (which carries no domain information at all).
const COOKIE_OWNERS: &[(&str, &str)] = &[
    ("SESSDATA", "bilibili.com"),
    ("buvid3", "bilibili.com"),
    ("buvid4", "bilibili.com"),
    ("bili_jct", "bilibili.com"),
    ("bili_ticket", "bilibili.com"),
    ("DedeUserID", "bilibili.com"),
    ("DedeUserID__ckMd5", "bilibili.com"),
    ("sid", "bilibili.com"),
    ("d_c0", "zhihu.com"),
    ("z_c0", "zhihu.com"),
    ("__zse_ck", "zhihu.com"),
    ("SNUID", "sogou.com"),
    ("SUV", "sogou.com"),
    ("SUID", "sogou.com"),
    ("ABTEST", "sogou.com"),
    ("IPLOC", "sogou.com"),
];

/// Split a raw `Cookie:` header into `(name, value)` pairs, preserving order.
pub fn cookie_pairs(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in raw.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, v)) = part.split_once('=') {
            let k = k.trim();
            if k.is_empty() {
                continue;
            }
            out.push((k.to_string(), v.trim().to_string()));
        }
    }
    out
}

/// Cookie names present in a header value (safe for logs/JSON — no values).
pub fn cookie_names(raw: &str) -> Vec<String> {
    cookie_pairs(raw).into_iter().map(|(k, _)| k).collect()
}

/// Split a legacy `FETCH_COOKIES` blob into per-domain headers.
///
/// The env var is a flat grab-bag with no domain attached, so names are
/// assigned to their known owner. Unknown names are **dropped** and returned
/// separately for a startup warning: silently sending them to arbitrary hosts
/// is the leak this module exists to stop.
pub fn seed_env(raw: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut by_domain: Vec<(String, Vec<String>)> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    for (name, value) in cookie_pairs(raw) {
        let owner = COOKIE_OWNERS
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&name))
            .map(|(_, d)| d.to_string());
        match owner {
            Some(domain) => {
                let pair = format!("{}={}", name, value);
                match by_domain.iter_mut().find(|(d, _)| *d == domain) {
                    Some((_, list)) => list.push(pair),
                    None => by_domain.push((domain, vec![pair])),
                }
            }
            None => unknown.push(name),
        }
    }
    let seeded = by_domain
        .into_iter()
        .map(|(d, list)| (d, list.join("; ")))
        .collect();
    (seeded, unknown)
}

// ─────────────────────────────────────────────────────────────────────────────
// Store I/O
// ─────────────────────────────────────────────────────────────────────────────

/// Parse the file, migrating the v1 layout (`site -> entry`) on the fly.
fn parse_store(text: &str) -> Entries {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Entries::new(),
    };
    if let Some(entries) = value.get("entries").and_then(|v| v.as_object()) {
        return entries
            .iter()
            .filter_map(|(k, v)| {
                let entry: Entry = serde_json::from_value(v.clone()).ok()?;
                let domain = canonical_domain(k)?;
                Some((domain, entry))
            })
            .collect();
    }
    // v1: a bare map of short site names to entries.
    let legacy: HashMap<String, Entry> = serde_json::from_value(value).unwrap_or_default();
    legacy
        .into_iter()
        .filter_map(|(site, entry)| canonical_domain(&site).map(|d| (d, entry)))
        .collect()
}

/// Load and parse the store, reusing the previous parse when the file has not
/// been modified. A missing or malformed file yields an empty map.
fn load() -> Entries {
    let path = cache_path();
    let mtime = match std::fs::metadata(&path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return Entries::new(), // absent → nothing cached
    };

    if let Ok(guard) = FILE_CACHE.lock() {
        if let Some((cached_mtime, entries)) = guard.as_ref() {
            if *cached_mtime == mtime {
                return entries.clone();
            }
        }
    }

    let entries = match std::fs::read_to_string(&path) {
        Ok(text) => parse_store(&text),
        Err(_) => Entries::new(),
    };

    if let Ok(mut guard) = FILE_CACHE.lock() {
        *guard = Some((mtime, entries.clone()));
    }
    entries
}

fn save(entries: &Entries) -> Result<(), String> {
    let path = cache_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
    }
    let json = serde_json::to_string_pretty(&serde_json::json!({
        "version": STORE_VERSION,
        "entries": entries,
    }))
    .map_err(|e| format!("serialize failed: {}", e))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    // Force the next `load` to re-read rather than serve the old parse.
    if let Ok(mut guard) = FILE_CACHE.lock() {
        *guard = None;
    }
    Ok(())
}

/// Store (or replace) the cookies of one domain.
pub fn put(domain: &str, cookies: &str, source: &str) -> Result<String, String> {
    let domain = canonical_domain(domain)
        .ok_or_else(|| format!("'{}' is not a valid cookie domain", domain))?;
    let mut entries = load();
    entries.insert(
        domain.clone(),
        Entry {
            cookies: cookies.to_string(),
            saved_at: now_secs(),
            source: source.to_string(),
        },
    );
    save(&entries)?;
    Ok(domain)
}

/// Get the cached header for exactly one domain (used by engines, which know
/// which site they are).
pub fn get_domain(domain: &str, ttl_secs: u64) -> Option<String> {
    let domain = canonical_domain(domain)?;
    let entries = load();
    let entry = entries.get(&domain)?;
    if entry.cookies.trim().is_empty() {
        return None;
    }

    let age = now_secs().saturating_sub(entry.saved_at);
    if ttl_secs > 0 && entry.saved_at > 0 && age > ttl_secs {
        eprintln!(
            "⚠️ cookie-cache: '{}' is {} min old (TTL {} min) — it may already be rejected; \
             re-seed it with fresh cookies when requests start failing",
            domain,
            age / 60,
            ttl_secs / 60
        );
    } else {
        eprintln!(
            "🍪 cookie-cache: using '{}' session cookies from {} ({} min old, {}, names: {})",
            domain,
            cache_path().display(),
            age / 60,
            entry.source,
            cookie_names(&entry.cookies).join(",")
        );
    }
    Some(entry.cookies.clone())
}

/// Back-compat shim for engines that pass a short site name.
pub fn get(site: &str, ttl_secs: u64) -> Option<String> {
    get_domain(site, ttl_secs)
}

/// Every stored entry that belongs to `host`, as `(domain, header)` pairs.
///
/// More specific domains are sorted first; [`merge_for_host`] resolves name
/// collisions.
pub fn cookies_for_host(host: &str) -> Vec<(String, String)> {
    let entries = load();
    let mut matched: Vec<(&String, &Entry)> = entries
        .iter()
        .filter(|(domain, entry)| {
            !entry.cookies.trim().is_empty() && domain_matches(host, domain)
        })
        .collect();
    matched.sort_by_key(|(domain, _)| std::cmp::Reverse(domain.len()));
    matched
        .into_iter()
        .map(|(domain, entry)| (domain.clone(), entry.cookies.clone()))
        .collect()
}

/// Merge env-seeded cookies and cached cookies into one `Cookie:` header for
/// `host`. Only entries whose domain covers `host` are included; cached values
/// win on name collisions (the cache is the fresher source).
///
/// Both inputs carry their domain, so scoping is enforced here rather than by
/// the caller.
pub fn merge_for_host(
    env: &[(String, String)],
    cached: &[(String, String)],
    host: &str,
) -> String {
    let mut names: Vec<String> = Vec::new();
    let mut out: Vec<String> = Vec::new();

    for (domain, header) in env {
        if !domain_matches(host, domain) {
            continue;
        }
        for (name, value) in cookie_pairs(header) {
            if !names.iter().any(|n| n == &name) {
                names.push(name.clone());
                out.push(format!("{}={}", name, value));
            }
        }
    }

    for (domain, header) in cached {
        if !domain_matches(host, domain) {
            continue;
        }
        for (name, value) in cookie_pairs(header) {
            match names.iter().position(|n| n == &name) {
                Some(i) => out[i] = format!("{}={}", name, value),
                None => {
                    names.push(name.clone());
                    out.push(format!("{}={}", name, value));
                }
            }
        }
    }

    out.join("; ")
}

/// One row of `set_cookies(action: "list")`. Values are never exposed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Summary {
    pub domain: String,
    pub source: String,
    pub saved_at: u64,
    pub age_minutes: u64,
    pub names: Vec<String>,
    pub bytes: usize,
}

/// List stored entries without their values.
pub fn list() -> Vec<Summary> {
    let entries = load();
    let mut rows: Vec<Summary> = entries
        .iter()
        .map(|(domain, entry)| Summary {
            domain: domain.clone(),
            source: entry.source.clone(),
            saved_at: entry.saved_at,
            age_minutes: now_secs().saturating_sub(entry.saved_at) / 60,
            names: cookie_names(&entry.cookies),
            bytes: entry.cookies.len(),
        })
        .collect();
    rows.sort_by(|a, b| a.domain.cmp(&b.domain));
    rows
}

/// Remove one domain. Returns whether anything was removed.
pub fn clear(domain: &str) -> Result<bool, String> {
    let domain = canonical_domain(domain)
        .ok_or_else(|| format!("'{}' is not a valid cookie domain", domain))?;
    let mut entries = load();
    let removed = entries.remove(&domain).is_some();
    if removed {
        save(&entries)?;
    }
    Ok(removed)
}

/// Remove every entry.
pub fn clear_all() -> Result<usize, String> {
    let mut entries = load();
    let n = entries.len();
    entries.clear();
    save(&entries)?;
    Ok(n)
}

/// Keep only the named cookies from a raw `Cookie:` header value.
///
/// Engines narrow cookies to what their anti-bot check needs: a 30-cookie
/// grab-bag is itself a bot signal.
pub fn filter_cookie_header(raw: &str, names: &[&str]) -> Option<String> {
    let pairs: Vec<String> = cookie_pairs(raw)
        .into_iter()
        .filter(|(k, _)| names.iter().any(|n| n.eq_ignore_ascii_case(k)))
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
    if pairs.is_empty() {
        None
    } else {
        Some(pairs.join("; "))
    }
}

/// Non-reversible short identifier for a cookie string, safe for logs.
///
/// Shows only the first and last 4 characters of the value, so a user can tell
/// *which* cookie is loaded without the log ever containing a usable
/// credential. Kept as public surface for tooling/tests even though the main
/// log path now prints cookie names instead.
#[allow(dead_code)]
pub fn fingerprint(cookies: &str) -> String {
    match filter_cookie_header(cookies, &["SNUID"]) {
        Some(snuid) => value_fingerprint(&snuid),
        None => format!("{} cookies, {} bytes", cookie_names(cookies).len(), cookies.len()),
    }
}

fn value_fingerprint(pair: &str) -> String {
    let (name, value) = pair.split_once('=').unwrap_or(("?", ""));
    if value.len() <= 8 {
        format!("{}(len {})", name, value.len())
    } else {
        format!(
            "{}({}…{}, len {})",
            name,
            &value[..4],
            &value[value.len() - 4..],
            value.len()
        )
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_to_requested_names_only() {
        let raw = "SNUID=AAA; SUV=BBB; SESSDATA=SECRET; buvid3=CCC; IPLOC=CN3201";
        let got = filter_cookie_header(raw, &["SNUID", "SUV", "IPLOC"]).unwrap();
        assert_eq!(got, "SNUID=AAA; SUV=BBB; IPLOC=CN3201");
        assert!(!got.contains("SESSDATA"), "unrelated cookies must be dropped");
    }

    #[test]
    fn filter_is_case_insensitive_and_keeps_order() {
        let raw = "suv=BBB; snuid=AAA";
        assert_eq!(
            filter_cookie_header(raw, &["SNUID", "SUV"]).unwrap(),
            "suv=BBB; snuid=AAA"
        );
    }

    #[test]
    fn filter_returns_none_when_nothing_matches() {
        assert!(filter_cookie_header("a=1; b=2", &["SNUID"]).is_none());
        assert!(filter_cookie_header("", &["SNUID"]).is_none());
    }

    #[test]
    fn fingerprint_never_leaks_the_full_value() {
        let raw = "SNUID=DEADBEEFCAFEBABE0123456789ABCDEF";
        let fp = fingerprint(raw);
        assert!(fp.contains("DEAD"), "should hint the prefix: {}", fp);
        assert!(
            !fp.contains("DEADBEEFCAFEBABE0123456789ABCDEF"),
            "must not leak: {}",
            fp
        );
    }

    #[test]
    fn domain_matching_is_suffix_bounded() {
        assert!(domain_matches("www.sogou.com", "sogou.com"));
        assert!(domain_matches("weixin.sogou.com", "sogou.com"));
        assert!(domain_matches("sogou.com", "sogou.com"));
        assert!(!domain_matches("notsogou.com", "sogou.com"));
        assert!(!domain_matches("sogou.com.evil.net", "sogou.com"));
        assert!(!domain_matches("example.com", "com"));
    }

    #[test]
    fn normalize_accepts_real_world_input() {
        assert_eq!(normalize_domain("https://www.sogou.com/x?y=1").unwrap(), "www.sogou.com");
        assert_eq!(normalize_domain(".SOGOU.com").unwrap(), "sogou.com");
        assert_eq!(normalize_domain("bilibili.com:443").unwrap(), "bilibili.com");
        assert!(normalize_domain("com").is_none());
        assert!(normalize_domain("*").is_none());
        assert!(normalize_domain("foo/bar").is_none());
        assert!(normalize_domain("").is_none());
    }

    #[test]
    fn short_site_names_map_to_their_owner() {
        assert_eq!(canonical_domain("sogou").unwrap(), "sogou.com");
        assert_eq!(canonical_domain("weixin").unwrap(), "sogou.com");
        assert_eq!(canonical_domain("bilibili").unwrap(), "bilibili.com");
        assert_eq!(canonical_domain("zhihu").unwrap(), "zhihu.com");
    }

    #[test]
    fn seed_env_splits_cookies_by_owner_and_reports_unknowns() {
        let raw = "SESSDATA=abc; d_c0=def; SNUID=ghi; mystery=1";
        let (seeded, unknown) = seed_env(raw);
        let get = |d: &str| {
            seeded
                .iter()
                .find(|(dom, _)| dom == d)
                .map(|(_, h)| h.as_str())
                .unwrap_or("")
        };
        assert_eq!(get("bilibili.com"), "SESSDATA=abc");
        assert_eq!(get("zhihu.com"), "d_c0=def");
        assert_eq!(get("sogou.com"), "SNUID=ghi");
        assert_eq!(unknown, vec!["mystery".to_string()]);
        assert!(!seeded.iter().any(|(_, h)| h.contains("mystery")));
    }

    #[test]
    fn v1_file_is_migrated_to_domains() {
        let text = r#"{"sogou":{"cookies":"SNUID=AAA","saved_at":1}}"#;
        let parsed = parse_store(text);
        assert!(parsed.contains_key("sogou.com"));
        assert!(!parsed.contains_key("sogou"));
    }

    #[test]
    fn v2_file_round_trips() {
        let text = r#"{"version":2,"entries":{"bilibili.com":{"cookies":"SESSDATA=x","saved_at":5,"source":"browser"}}}"#;
        let parsed = parse_store(text);
        let entry = parsed.get("bilibili.com").unwrap();
        assert_eq!(entry.cookies, "SESSDATA=x");
        assert_eq!(entry.source, "browser");
    }

    #[test]
    fn merge_never_crosses_domains_and_cache_wins() {
        let env = vec![
            ("bilibili.com".to_string(), "SESSDATA=old; buvid3=b1".to_string()),
            ("zhihu.com".to_string(), "d_c0=zh".to_string()),
        ];

        // Bilibili host: env cookies for that domain, cache overrides SESSDATA.
        let cached = vec![("bilibili.com".to_string(), "SESSDATA=new".to_string())];
        let got = merge_for_host(&env, &cached, "www.bilibili.com");
        assert_eq!(got, "SESSDATA=new; buvid3=b1");

        // Unrelated host gets nothing at all.
        assert_eq!(merge_for_host(&env, &cached, "blog.csdn.net"), "");
        assert_eq!(merge_for_host(&env, &cached, "evil-bilibili.com"), "");
    }
}
