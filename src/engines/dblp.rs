use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::OnceLock;
use crate::anubis;
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

/// DBLP — computer science bibliography (free, no API key).
///
/// API: `https://dblp.org/search/publ/api?q=<query>&format=json&h=<hits>&f=<offset>`
/// Free, no key. Be polite: DBLP asks for at most 1-2 requests per second;
/// it responds with 429 + Retry-After when rate limited.
pub struct DblpEngine;

/// DBLP API hosts, tried in order. `dblp.org` is reset by some mainland
/// networks (WinError 10054); `dblp.uni-trier.de` serves the same API and
/// is reachable there. Both deploy Anubis anti-bot (`src/anubis.rs`).
const DBLP_HOSTS: &[&str] = &["dblp.org", "dblp.uni-trier.de"];

/// One client per host, kept for the process lifetime.
///
/// They carry `cookie_store(true)`, so the Anubis auth cookie obtained by
/// solving the PoW survives across searches (Anubis tokens last ~a week) —
/// we pay the hash cost once instead of on every query.
static DBLP_CLIENTS: OnceLock<HashMap<&'static str, reqwest::Client>> = OnceLock::new();

/// Index into `DBLP_HOSTS` of the host that last succeeded.
///
/// `dblp.org` is connection-reset on some mainland networks, and paying its
/// connect timeout on every single search is slow; start from wherever we
/// worked last and only fall back when that host also fails.
static PREFERRED_HOST: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn get_dblp_client(config: &Config, host: &'static str) -> Result<&'static reqwest::Client, SearchError> {
    let clients = DBLP_CLIENTS.get_or_init(|| {
        DBLP_HOSTS
            .iter()
            .filter_map(|h| {
                config
                    .build_reqwest_client_for_with_cookies(h)
                    .ok()
                    .map(|c| (*h, c))
            })
            .collect()
    });
    clients
        .get(host)
        .ok_or_else(|| SearchError::Http(format!("dblp: no client for {}", host)))
}

/// Fetch the API, transparently solving an Anubis PoW challenge if we get one.
async fn fetch_with_anubis(
    client: &reqwest::Client,
    url: &str,
    query: &str,
    hits: usize,
) -> Result<String, SearchError> {
    let body = fetch_dblp_json(client, url, query, hits).await?;
    let Some(challenge) = anubis::detect(&body) else {
        return Ok(body);
    };

    eprintln!("🧩 dblp: Anubis challenge detected (difficulty {}), solving…", challenge.difficulty);
    let origin = anubis::origin_of(url);
    anubis::pass(client, &origin, &challenge, &format!("{}/", origin))
        .await
        .map_err(SearchError::Blocked)?;

    // Replay the original request; the auth cookie is now in the jar.
    fetch_dblp_json(client, url, query, hits).await
}

#[async_trait]
impl SearchEngine for DblpEngine {
    fn name(&self) -> &'static str { "dblp" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let hits = std::cmp::min(limit, 100).max(1);

        let mut body_text: Option<String> = None;
        let mut last_err: Option<SearchError> = None;

        // Try the last-good host first, then wrap around.
        let start = PREFERRED_HOST.load(std::sync::atomic::Ordering::Relaxed) % DBLP_HOSTS.len();
        for offset in 0..DBLP_HOSTS.len() {
            let idx = (start + offset) % DBLP_HOSTS.len();
            let host = DBLP_HOSTS[idx];
            let client = get_dblp_client(config, host)?;
            let url = format!("https://{}/search/publ/api", host);
            match fetch_with_anubis(client, &url, query, hits).await {
                Ok(text) => {
                    PREFERRED_HOST.store(idx, std::sync::atomic::Ordering::Relaxed);
                    body_text = Some(text);
                    break;
                }
                Err(e) => {
                    eprintln!("⚠️ dblp host {} failed: {}", host, e);
                    last_err = Some(e);
                }
            }
        }
        let body_text = match body_text {
            Some(t) => t,
            None => return Err(last_err.unwrap_or_else(|| {
                SearchError::Http("DBLP: all hosts failed".to_string())
            })),
        };

        let data: serde_json::Value = match serde_json::from_str(&body_text) {
            Ok(v) => v,
            Err(e) => {
                // DBLP serves HTML (Anubis challenge / 429 page) instead of JSON
                // when it blocks or throttles the client.
                let lower = body_text.to_lowercase();
                if lower.contains("anubis") || lower.contains("not a bot") {
                    return Err(SearchError::Blocked(
                        "DBLP returned an Anubis anti-bot challenge (proxy/datacenter IP blocked); try another proxy node".to_string(),
                    ));
                }
                if lower.contains("too many requests") {
                    return Err(SearchError::Blocked(
                        "DBLP rate limited (429); slow down requests".to_string(),
                    ));
                }
                let hint = if body_text.trim_start().starts_with('<') {
                    "DBLP returned an XML/HTML response instead of JSON (possibly rate limited)"
                } else {
                    "DBLP JSON parse failed"
                };
                return Err(SearchError::Parse(format!("{}: {}", hint, e)));
            }
        };

        let hits_arr = &data["result"]["hits"]["hit"];
        let hits_arr = match hits_arr {
            serde_json::Value::Array(arr) => arr,
            _ => return Ok(Vec::new()), // no hits
        };

        let mut results = Vec::new();
        for item in hits_arr {
            let info = &item["info"];

            let title = info["title"].as_str().unwrap_or("").trim().to_string();
            if title.is_empty() { continue; }

            // URL: prefer DOI link, else DBLP page, else arXiv
            let url = info["ee"].as_str()
                .or_else(|| info["doi"].as_str())
                .or_else(|| info["url"].as_str())
                .unwrap_or("")
                .to_string();

            let authors: Vec<&str> = info["authors"]["author"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|a| a.as_str()).collect())
                .unwrap_or_else(|| {
                    info["authors"]["author"].as_str().map(|s| vec![s]).unwrap_or_default()
                });

            let venue = info["venue"].as_str().unwrap_or("");
            let year = info["year"].as_str().unwrap_or("");
            let doi = info["doi"].as_str().unwrap_or("");

            let mut description = String::new();
            if !authors.is_empty() {
                description.push_str(&format!("{} | ", authors.join(", ")));
            }
            if !venue.is_empty() {
                description.push_str(&format!("{} | ", venue));
            }
            if !year.is_empty() {
                description.push_str(&format!("{}", year));
            }
            if !doi.is_empty() {
                if !description.is_empty() { description.push_str(" | "); }
                description.push_str(&format!("DOI: {}", doi));
            }

            results.push(SearchResult {
                title: title.chars().take(200).collect(),
                url: url.chars().take(500).collect(),
                description: description.chars().take(400).collect(),
                source: venue.to_string(),
                engine: "dblp".to_string(),
                summary: None,
            });

            if results.len() >= limit { break; }
        }

        Ok(results)
    }
}

/// Fetch the raw response body from one DBLP API host.
async fn fetch_dblp_json(
    client: &reqwest::Client,
    url: &str,
    query: &str,
    hits: usize,
) -> Result<String, SearchError> {
    let resp = client.get(url)
        .query(&[
            ("q", query),
            ("format", "json"),
            ("h", &hits.to_string()),
            ("f", "0"),
        ])
        .header("Accept", "application/json")
        // Override the client default `Accept-Encoding: gzip, deflate, br`:
        // DBLP's server returns HTTP 500 when `br` (brotli) is offered.
        .header(reqwest::header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .map_err(|e| SearchError::Http(format!("DBLP request failed: {}", e)))?;

    let status = resp.status();
    if !status.is_success() {
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError::Blocked("DBLP rate limited (429); slow down requests".to_string()));
        }
        return Err(SearchError::Http(format!("DBLP returned status {}", status)));
    }

    resp.text().await
        .map_err(|e| SearchError::Http(format!("DBLP read body failed: {}", e)))
}
