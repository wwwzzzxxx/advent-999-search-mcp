//! Opt-in JSONL log for `web` searches, one line per engine call.
//!
//! `get_page` has its own log; this one exists so a low-recall engine can be
//! identified from data instead of guessed at. Each line records the query,
//! engine, how many results came back, how long it took, the share of `limit`
//! that engine was given, and the error when it failed. Skipped engines are
//! logged too (share = 0), so "limit too small for N engines" shows up as
//! `code: "skipped"` rather than silently vanishing.
//!
//! Off by default. Enable with `ADVENT_SEARCH_LOG=1`; the file defaults to
//! `%LOCALAPPDATA%\advent-mcp\search-log.jsonl` and can be moved with
//! `ADVENT_SEARCH_LOG_PATH`. Appended one JSON object per line, rotated once at
//! 8 MB, and never contains API keys or response bodies.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: u64 = 8 * 1024 * 1024;

static FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());

pub fn default_path() -> PathBuf {
    crate::cookie_cache::cache_path()
        .parent()
        .map(|p| p.join("search-log.jsonl"))
        .unwrap_or_else(|| PathBuf::from("search-log.jsonl"))
}

fn configured_path() -> Option<PathBuf> {
    let on = std::env::var("ADVENT_SEARCH_LOG")
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
        std::env::var("ADVENT_SEARCH_LOG_PATH")
            .ok()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_path),
    )
}

fn file() -> Option<&'static PathBuf> {
    FILE.get_or_init(configured_path).as_ref()
}

pub fn path() -> Option<PathBuf> {
    file().cloned()
}

/// One engine attempt (or an engine that was skipped for lack of a slot).
pub struct Entry<'a> {
    pub query: &'a str,
    pub engine: &'a str,
    pub ok: bool,
    pub results: usize,
    pub duration_ms: u128,
    /// Overall `limit` the caller asked for.
    pub limit: usize,
    /// Slice of `limit` this engine was allowed to fill; 0 means it never ran.
    pub share: usize,
    /// 1-based position of the topic inside a `queries` batch (0 for single).
    pub batch: usize,
    /// `ok` | `error` | `skipped` | `unsupported`
    pub code: &'a str,
    pub error: Option<&'a str>,
}

/// One log line as an object. Newlines are flattened so a line stays a line.
fn to_value(entry: &Entry<'_>) -> serde_json::Value {
    let mut obj = serde_json::json!({
        "ts": crate::fetch_log::now_iso8601(),
        "query": entry.query.replace('\n', " "),
        "engine": entry.engine,
        "ok": entry.ok,
        "code": entry.code,
        "results": entry.results,
        "ms": entry.duration_ms,
        "limit": entry.limit,
        "share": entry.share,
        "batch": entry.batch,
    });
    if let Some(err) = entry.error {
        obj["error"] = serde_json::json!(err.replace('\n', " "));
    }
    obj
}

pub fn write(entry: &Entry<'_>) {
    let Some(path) = file() else { return };
    let line = match serde_json::to_string(&to_value(entry)) {
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

fn rotate_if_needed(path: &std::path::Path) {
    let too_big = std::fs::metadata(path)
        .map(|m| m.len() >= MAX_BYTES)
        .unwrap_or(false);
    if too_big {
        let mut backup = path.as_os_str().to_os_string();
        backup.push(".1");
        let _ = std::fs::rename(path, PathBuf::from(backup));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_stays_one_line() {
        let e = Entry {
            query: "小米\nSU7",
            engine: "sogou",
            ok: false,
            results: 0,
            duration_ms: 120,
            limit: 10,
            share: 3,
            batch: 0,
            code: "error",
            error: Some("Blocked: multi\nline".into()),
        };
        let s = serde_json::to_string(&to_value(&e)).unwrap();
        assert!(!s.contains('\n'), "must stay one line: {}", s);
        assert!(s.contains("multi line"));
        assert!(s.contains("小米 SU7"));
    }
}
