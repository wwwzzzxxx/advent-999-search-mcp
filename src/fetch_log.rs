//! Opt-in JSONL log for `get_page` fetches.
//!
//! Every fetch writes one line: URL, host, outcome, HTTP status, timing, how
//! the content was obtained, how many characters were returned, and which
//! stored cookie names were attached (names only — never values). It exists so
//! failures and slow paths can be analysed after the fact instead of guessed
//! at.
//!
//! Off by default. Enable with `ADVENT_FETCH_LOG=1`; the file defaults to
//! `%LOCALAPPDATA%\advent-mcp\fetch-log.jsonl` and can be moved with
//! `ADVENT_FETCH_LOG_PATH`.
//!
//! The file is appended to (one JSON object per line), rotated once at 8 MB,
//! and never contains cookie values or response bodies.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Upper bound before rotating to `<path>.1`.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

static FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());

/// Default location, next to the cookie store (outside the repository).
pub fn default_path() -> PathBuf {
    crate::cookie_cache::cache_path()
        .parent()
        .map(|p| p.join("fetch-log.jsonl"))
        .unwrap_or_else(|| PathBuf::from("fetch-log.jsonl"))
}

fn configured_path() -> Option<PathBuf> {
    let on = std::env::var("ADVENT_FETCH_LOG")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    if !on {
        return None;
    }
    Some(
        std::env::var("ADVENT_FETCH_LOG_PATH")
            .ok()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_path),
    )
}

/// The log file when enabled, `None` otherwise.
fn file() -> Option<&'static PathBuf> {
    FILE.get_or_init(configured_path).as_ref()
}

pub fn path() -> Option<PathBuf> {
    file().cloned()
}

/// One fetch attempt.
pub struct Entry<'a> {
    pub url: &'a str,
    pub host: &'a str,
    pub site: &'a str,
    /// `content` | `find` | `highlights` | `error`
    pub mode: &'a str,
    pub ok: bool,
    pub status: Option<u16>,
    pub error: Option<&'a str>,
    /// Characters in the extracted document.
    pub total_chars: usize,
    /// Characters actually handed back to the caller.
    pub returned_chars: usize,
    pub truncated: bool,
    pub duration_ms: u128,
    /// How the content was obtained (`direct`, `jina`, `zhihu-api`, ...).
    pub via: &'a str,
    /// Stored cookie names attached to the request (max 30, names only).
    pub cookie_names: Vec<String>,
    /// Position inside a multi-URL call (1-based).
    pub index: usize,
    pub batch: usize,
    pub retried: bool,
}

/// Append one line. Does nothing when logging is disabled.
pub fn write(entry: &Entry<'_>) {
    let Some(path) = file() else { return };
    let mut obj = serde_json::json!({
        "ts": iso8601(now_secs()),
        "url": entry.url,
        "host": entry.host,
        "site": entry.site,
        "mode": entry.mode,
        "ok": entry.ok,
        "status": entry.status,
        "totalChars": entry.total_chars,
        "returnedChars": entry.returned_chars,
        "truncated": entry.truncated,
        "ms": entry.duration_ms,
        "via": entry.via,
        "cookies": entry.cookie_names.len(),
        "batch": entry.batch,
        "index": entry.index,
        "retried": entry.retried,
    });
    if !entry.cookie_names.is_empty() {
        obj["cookieNames"] = serde_json::json!(entry.cookie_names);
    }
    if let Some(err) = entry.error {
        // Keep the line single-line even when an error message is multi-line.
        obj["error"] = serde_json::json!(err.replace('\n', " "));
    }

    let line = match serde_json::to_string(&obj) {
        Ok(l) => l,
        Err(_) => return,
    };

    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
    }
    rotate_if_needed(path);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{}", line);
    }
}

/// One-step rotation: `fetch-log.jsonl` → `fetch-log.jsonl.1`.
fn rotate_if_needed(path: &std::path::Path) {
    let too_big = std::fs::metadata(path).map(|m| m.len() >= MAX_BYTES).unwrap_or(false);
    if too_big {
        let mut backup = path.as_os_str().to_os_string();
        backup.push(".1");
        let _ = std::fs::rename(path, PathBuf::from(backup));
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `1970-01-01T00:00:00Z` style UTC timestamp (no chrono dependency).
fn iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = crate::filters::civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_formats_utc() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn entry_is_one_json_line_without_values() {
        let e = Entry {
            url: "https://www.zhihu.com/question/1",
            host: "www.zhihu.com",
            site: "知乎",
            mode: "content",
            ok: true,
            status: Some(200),
            error: None,
            total_chars: 100,
            returned_chars: 80,
            truncated: true,
            duration_ms: 42,
            via: "zhihu-api",
            cookie_names: vec!["d_c0".into()],
            index: 1,
            batch: 2,
            retried: false,
        };
        let mut obj = serde_json::json!({
            "ts": iso8601(0), "url": e.url, "host": e.host, "site": e.site,
            "mode": e.mode, "ok": e.ok, "status": e.status,
            "totalChars": e.total_chars, "returnedChars": e.returned_chars,
            "truncated": e.truncated, "ms": e.duration_ms, "via": e.via,
            "cookies": e.cookie_names.len(), "cookieNames": e.cookie_names,
            "batch": e.batch, "index": e.index, "retried": e.retried,
        });
        obj["error"] = serde_json::json!("multi\nline".replace('\n', " "));
        let s = serde_json::to_string(&obj).unwrap();
        assert!(!s.contains('\n'), "must stay one line: {}", s);
        assert!(s.contains("\"cookies\":1"));
        assert!(s.contains("multi line"));
    }
}
