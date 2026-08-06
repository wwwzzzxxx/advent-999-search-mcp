use async_trait::async_trait;
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use crate::python_embed;
use super::SearchEngine;

/// DeepSeek web search — LLM-backed search via the Anthropic-compatible
/// Messages API with the server-side `web_search_20250305` tool.
///
/// One engine, two backends (auto-detected from the key format):
/// - **official**: DeepSeek official API key (`sk-` + 32 hex, 35 chars)
///   → `https://api.deepseek.com/anthropic/v1/messages` (direct, mainland,
///   plain reqwest works — no Cloudflare fingerprint check)
/// - **go**: OpenCode Go subscription key (`sk-` + 64 chars, e.g. from
///   `~/.local/share/opencode/auth.json`)
///   → `https://opencode.ai/zen/go/v1/messages` (via proxy). opencode.ai
///   sits behind Cloudflare which blocks reqwest/curl TLS fingerprints, so
///   this backend spawns the embedded Python runtime (requests/urllib3
///   fingerprint passes).
///
/// Override detection with `DEEPSEEK_API_MODE=official|go`.
/// Disabled entirely when no key is present.
pub struct DeepseekEngine;

const OFFICIAL_BASE: &str = "https://api.deepseek.com/anthropic";
const DEFAULT_MODEL: &str = "deepseek-v4-flash";
/// LLM search is slow (search + summarize), well beyond the default 30s.
const SEARCH_TIMEOUT_SECS: u64 = 120;

/// Python search script for the go backend. Runs with `python -c`, query on
/// stdin, prints one JSON line: {"status": N, "results": [...], "summary": "..."}.
const GO_SEARCH_SCRIPT: &str = r#"
import json, os, sys
import requests

query = sys.stdin.read()
with open(os.path.expanduser("~/.local/share/opencode/auth.json"), encoding="utf-8") as f:
    key = json.load(f)["opencode-go"]["key"]

body = {
    "model": os.environ.get("DEEPSEEK_MODEL", "deepseek-v4-flash"),
    "max_tokens": 2048,
    "tools": [{"type": "web_search_20250305", "name": "web_search"}],
    "messages": [{"role": "user", "content": query}],
}
r = requests.post("https://opencode.ai/zen/go/v1/messages",
                  headers={"x-api-key": key, "anthropic-version": "2023-06-01"},
                  json=body, timeout=90)
j = r.json()
results = []
summary = ""
for b in j.get("content", []):
    if b["type"] == "web_search_tool_result":
        for item in b.get("content", []):
            results.append({
                "title": item.get("title", ""),
                "url": item.get("url", ""),
                "page_age": item.get("page_age") or "",
            })
    elif b["type"] == "text":
        summary += b.get("text", "")
print(json.dumps({"status": r.status_code, "results": results, "summary": summary}))
"#;

#[derive(Debug, Clone, Copy, PartialEq)]
enum DeepseekMode {
    Official,
    Go,
}

impl DeepseekMode {
    fn detect(key: &str, override_mode: Option<&str>) -> Self {
        match override_mode.map(|m| m.trim().to_lowercase()).as_deref() {
            Some("official") => return DeepseekMode::Official,
            Some("go") => return DeepseekMode::Go,
            _ => {}
        }
        // Auto-detection: official keys are exactly `sk-` + 32 hex chars.
        if key.len() == 35 && key.starts_with("sk-")
            && key[3..].chars().all(|c| c.is_ascii_hexdigit()) {
            DeepseekMode::Official
        } else {
            DeepseekMode::Go
        }
    }
}

fn build_body(query: &str, model: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "max_tokens": 2048,
        "messages": [
            {"role": "system", "content":
                "You are a web search assistant. Use the web_search tool to find \
                 up-to-date information, then summarize what you found in 2-4 \
                 sentences in the same language as the query. Do not repeat \
                 search results; just summarize."},
            {"role": "user", "content": query}
        ],
        // `name` is REQUIRED — the OpenCode Go gateway 400s without it
        // (the plain Anthropic standard form does not work there).
        "tools": [{"type": "web_search_20250305", "name": "web_search"}],
        "tool_choice": {"type": "auto"},
        "thinking": {"type": "disabled"}
    })
}

#[async_trait]
impl SearchEngine for DeepseekEngine {
    fn name(&self) -> &'static str { "deepseek" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let key = config.deepseek_api_key.as_deref()
            .ok_or_else(|| SearchError::Engine(
                "DEEPSEEK_API_KEY not set; set it (DeepSeek official or OpenCode Go key) to enable the deepseek engine".to_string()
            ))?;

        let mode = DeepseekMode::detect(key, config.deepseek_api_mode.as_deref());
        let data = match mode {
            DeepseekMode::Official => {
                self.search_official(query, key, config).await?
            }
            DeepseekMode::Go => {
                self.search_go(query).await?
            }
        };

        parse_response(&data, limit)
    }
}

impl DeepseekEngine {
    /// Official backend: plain reqwest to api.deepseek.com (no Cloudflare).
    async fn search_official(&self, query: &str, key: &str, config: &Config) -> Result<serde_json::Value, SearchError> {
        let client = config.build_reqwest_client_for("api.deepseek.com")
            .map_err(|e| SearchError::Http(format!("deepseek client build failed: {}", e)))?;

        let model = std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let body = build_body(query, &model);

        let resp = client.post(format!("{}/v1/messages", OFFICIAL_BASE))
            .header("content-type", "application/json")
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .timeout(std::time::Duration::from_secs(SEARCH_TIMEOUT_SECS))
            .json(&body)
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("deepseek request failed: {}", e)))?;

        match resp.status() {
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                return Err(SearchError::Blocked(
                    "deepseek rejected the API key (401/403); check DEEPSEEK_API_KEY".to_string()
                ));
            }
            reqwest::StatusCode::TOO_MANY_REQUESTS | reqwest::StatusCode::SERVICE_UNAVAILABLE => {
                return Err(SearchError::Blocked(
                    "deepseek rate limited (429/503); slow down requests".to_string()
                ));
            }
            _ => {}
        }

        resp.json().await
            .map_err(|e| SearchError::Parse(format!("deepseek JSON parse failed: {}", e)))
    }

    /// Go backend: spawn embedded Python (requests passes Cloudflare).
    async fn search_go(&self, query: &str) -> Result<serde_json::Value, SearchError> {
        let py = python_embed::python_exe()
            .map_err(|e| SearchError::Engine(format!("embedded python unavailable: {}", e)))?;

        // Spawn on a blocking thread: std::process::Command is blocking.
        let py_clone = py.clone();
        let query_owned = query.to_string();
        let output = tokio::task::spawn_blocking(move || {
            use std::io::Write;
            use std::process::{Command, Stdio};

            let mut child = Command::new(&py_clone)
                .arg("-c")
                .arg(GO_SEARCH_SCRIPT)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("spawn python failed: {}", e))?;

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(query_owned.as_bytes())
                    .map_err(|e| format!("write query to python stdin failed: {}", e))?;
            }

            let out = child.wait_with_output()
                .map_err(|e| format!("wait python failed: {}", e))?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(format!("python exited {:?}: {}", out.status.code(), stderr.trim()));
            }
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        })
        .await
        .map_err(|e| SearchError::Engine(format!("python task join failed: {}", e)))?
        .map_err(SearchError::Engine)?;

        let data: serde_json::Value = serde_json::from_str(&output)
            .map_err(|e| SearchError::Parse(format!("go backend JSON parse failed: {}", e)))?;

        // Python script reports status separately.
        if let Some(status) = data["status"].as_u64() {
            match status {
                401 | 403 => return Err(SearchError::Blocked(
                    "opencode.ai rejected the key (401/403); check auth.json / OPENCODE_GO_KEY".to_string()
                )),
                429 => return Err(SearchError::Blocked(
                    "opencode.ai rate limited (429); slow down requests".to_string()
                )),
                s if s >= 400 => return Err(SearchError::Engine(
                    format!("opencode.ai returned HTTP {}", s)
                )),
                _ => {}
            }
        }

        Ok(data)
    }
}

/// Parse the Anthropic-style content blocks into SearchResults.
/// Works for both backends: the official backend returns the raw API
/// response (content blocks at top level); the go backend returns the
/// Python-normalized {"results": [...], "summary": "..."} object.
fn parse_response(data: &serde_json::Value, limit: usize) -> Result<Vec<SearchResult>, SearchError> {
    // Go backend: normalized shape.
    if let Some(items) = data["results"].as_array() {
        let summary = data["summary"].as_str().unwrap_or("").trim().to_string();
        let mut results: Vec<SearchResult> = Vec::new();
        for item in items {
            let title = item["title"].as_str().unwrap_or("").trim().to_string();
            let url = item["url"].as_str().unwrap_or("").trim().to_string();
            if title.is_empty() || url.is_empty() { continue; }
            let page_age = item["page_age"].as_str().unwrap_or("").trim().to_string();
            results.push(SearchResult {
                title: title.chars().take(200).collect(),
                url: url.chars().take(500).collect(),
                description: page_age,
                source: extract_hostname(&url),
                engine: "deepseek".to_string(),
                summary: None,
            });
            if results.len() >= limit { break; }
        }
        if results.is_empty() && summary.is_empty() {
            return Err(SearchError::Engine(
                "deepseek returned no search results (model may have answered without searching)".to_string()
            ));
        }
        if !summary.is_empty() && !results.is_empty() {
            results[0].summary = Some(summary.chars().take(2000).collect());
        }
        return Ok(results);
    }

    // Official backend: raw Anthropic content blocks.
    let content = data["content"].as_array()
        .ok_or_else(|| SearchError::Parse(
            "deepseek response missing content block".to_string()
        ))?;

    let mut results: Vec<SearchResult> = Vec::new();
    let mut summary_parts: Vec<String> = Vec::new();

    for block in content {
        match block["type"].as_str() {
            Some("web_search_tool_result") => {
                if let Some(inner) = block["content"].as_array() {
                    for item in inner {
                        if item["type"].as_str() != Some("web_search_result") { continue; }
                        let title = item["title"].as_str().unwrap_or("").trim().to_string();
                        let url = item["url"].as_str().unwrap_or("").trim().to_string();
                        if title.is_empty() || url.is_empty() { continue; }
                        let page_age = item["page_age"].as_str().unwrap_or("").trim().to_string();
                        results.push(SearchResult {
                            title: title.chars().take(200).collect(),
                            url: url.chars().take(500).collect(),
                            description: page_age,
                            source: extract_hostname(&url),
                            engine: "deepseek".to_string(),
                            summary: None,
                        });
                        if results.len() >= limit { break; }
                    }
                }
            }
            Some("text") => {
                let text = block["text"].as_str().unwrap_or("").trim().to_string();
                if !text.is_empty() { summary_parts.push(text); }
            }
            _ => {}
        }
    }

    if results.is_empty() && summary_parts.is_empty() {
        return Err(SearchError::Engine(
            "deepseek returned no search results (model may have answered without searching)".to_string()
        ));
    }

    // Attach the model's AI summary to the first result (only deepseek
    // engines populate this field; other engines omit it in JSON).
    if !summary_parts.is_empty() && !results.is_empty() {
        results[0].summary = Some(summary_parts.join("\n\n").chars().take(2000).collect());
    }

    Ok(results)
}

fn extract_hostname(url: &str) -> String {
    url.split('/')
        .find(|s| s.contains('.'))
        .unwrap_or("")
        .to_string()
}
