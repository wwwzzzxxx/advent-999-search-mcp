//! Short-lived local cookie cache for anti-bot sessions.
//!
//! Some engines need a credential that only a human can obtain — notably
//! Sogou's `SNUID`, which is issued *after* the user solves a click-captcha.
//! Putting it in an env var works, but has two annoyances: editing the MCP
//! config, and restarting the server (env vars are read once at startup).
//!
//! This module keeps such cookies in a small JSON file under the user's local
//! data directory instead. The file is re-read whenever its mtime changes, so
//! refreshing a cookie takes effect on the *next search* — no restart. Entries
//! carry a timestamp, and readers warn once they are older than the TTL
//! (~20 min by default, matching Sogou's real `SNUID` lifetime).
//!
//! **The file lives outside the repository** (`%LOCALAPPDATA%\advent-mcp\` on
//! Windows, `~/.cache/advent-mcp/` elsewhere) and must never be committed.
//!
//! Layout:
//!
//! ```json
//! {
//!   "sogou": { "cookies": "SNUID=...; SUV=...", "saved_at": 1789000000 }
//! }
//! ```

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Parsed view of the cache file: `site -> (cookies, saved_at)`.
type Entries = std::collections::HashMap<String, Entry>;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Entry {
    pub cookies: String,
    /// Unix seconds when the cookies were captured.
    #[serde(default)]
    pub saved_at: u64,
}

/// Raw file contents plus the mtime they came from, so we can skip re-reading
/// (and re-parsing) the file on every single search.
static FILE_CACHE: Mutex<Option<(SystemTime, Entries)>> = Mutex::new(None);

/// Location of the cache file.
///
/// Kept outside the repo: this holds live session credentials.
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

/// Load and parse the cache file, reusing the previous parse when the file has
/// not been modified. A missing or malformed file yields an empty map.
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

    let entries: Entries = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Entries::new(),
    };

    if let Ok(mut guard) = FILE_CACHE.lock() {
        *guard = Some((mtime, entries.clone()));
    }
    entries
}

/// Get the cached cookie header for `site`, if present.
///
/// `ttl_secs` is advisory: an older entry is still returned (the server is the
/// real authority on validity) but a warning is logged so the user knows to
/// re-seed it.
pub fn get(site: &str, ttl_secs: u64) -> Option<String> {
    let entries = load();
    let entry = entries.get(site)?;
    if entry.cookies.trim().is_empty() {
        return None;
    }

    let age = now_secs().saturating_sub(entry.saved_at);
    if ttl_secs > 0 && entry.saved_at > 0 && age > ttl_secs {
        eprintln!(
            "⚠️ cookie-cache: '{}' is {} min old (TTL {} min) — it may already be rejected; \
             re-seed {} with fresh cookies when searches start failing",
            site,
            age / 60,
            ttl_secs / 60,
            cache_path().display()
        );
    } else {
        eprintln!(
            "🍪 cookie-cache: using '{}' session cookies from {} ({} min old, fingerprint {})",
            site,
            cache_path().display(),
            age / 60,
            fingerprint(&entry.cookies)
        );
    }
    Some(entry.cookies.clone())
}

/// Write (or update) the cached cookies for `site`.
pub fn put(site: &str, cookies: &str) -> Result<(), String> {
    let path = cache_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
    }

    // Preserve other sites' entries.
    let mut entries = match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str::<Entries>(&t).unwrap_or_default(),
        Err(_) => Entries::new(),
    };
    entries.insert(
        site.to_string(),
        Entry { cookies: cookies.to_string(), saved_at: now_secs() },
    );

    let json = serde_json::to_string_pretty(&entries)
        .map_err(|e| format!("serialize failed: {}", e))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;

    // Force the next `get` to re-read rather than serve the old parse.
    if let Ok(mut guard) = FILE_CACHE.lock() {
        *guard = None;
    }
    Ok(())
}

/// Keep only the named cookies from a raw `Cookie:` header value.
///
/// Users often paste a whole `document.cookie` dump (including unrelated sites'
/// cookies), and sending that grab-bag everywhere is itself a bot signal, so
/// engines narrow it to the cookies they need. Returns `None` if none match.
pub fn filter_cookie_header(raw: &str, names: &[&str]) -> Option<String> {
    let mut pairs: Vec<&str> = Vec::new();
    for part in raw.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let key = match part.split_once('=') {
            Some((k, _)) => k.trim(),
            None => continue,
        };
        if names.iter().any(|n| n.eq_ignore_ascii_case(key)) {
            pairs.push(part);
        }
    }
    if pairs.is_empty() {
        None
    } else {
        Some(pairs.join("; "))
    }
}

/// Non-reversible short identifier for a cookie string, safe for logs.
///
/// Shows only the first and last 4 characters of each value's length, so a
/// user can tell *which* cookie is loaded without the log ever containing a
/// usable credential.
fn fingerprint(cookies: &str) -> String {
    match filter_cookie_header(cookies, &["SNUID"]) {
        Some(snuid) => {
            let value = snuid.split_once('=').map(|(_, v)| v).unwrap_or("");
            if value.len() <= 8 {
                format!("SNUID(len {})", value.len())
            } else {
                format!("SNUID({}…{}, len {})", &value[..4], &value[value.len() - 4..], value.len())
            }
        }
        None => format!("{} bytes, no SNUID", cookies.len()),
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
        // Obviously-fake value: this file is public, so no real credential may
        // ever appear — not even in a test fixture.
        let raw = "SNUID=DEADBEEFCAFEBABE0123456789ABCDEF";
        let fp = fingerprint(raw);
        assert!(fp.contains("DEAD"), "should hint the prefix: {}", fp);
        assert!(!fp.contains("DEADBEEFCAFEBABE0123456789ABCDEF"), "must not leak: {}", fp);
    }
}
