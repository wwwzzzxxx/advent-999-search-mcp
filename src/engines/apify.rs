//! Apify "Google Search Results Scraper" (`engines: ["apify"]`).
//!
//! Real Google SERP results without dealing with captchas: Apify runs the
//! browser, we call its synchronous endpoint and get the dataset items back in
//! one request. The free plan includes $5 of platform usage per month (no
//! credit card), which is roughly 2,000-2,700 SERP pages at the Actor's
//! ~$1.80 / 1,000 pages, so unlike Serper's one-time grant it keeps refilling.
//!
//! Endpoint:
//! Start a run with `POST /v2/acts/apify~google-search-scraper/runs`, poll
//! `/v2/actor-runs/<id>` until it succeeds, then read
//! `/v2/datasets/<id>/items`, all with `Authorization: Bearer <APIFY_API_KEY>`.
//!
//! A run typically takes 30-80s, so this engine builds its own client with a
//! long timeout instead of the shared 30s one.

use async_trait::async_trait;
use crate::config::Config;
use crate::filters::{Freshness, FreshnessTier, SearchOptions};
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct ApifyEngine;

const RUN_TIMEOUT_SECS: u64 = 180;
/// Async run + poll. The `run-sync` endpoint works too, but Apify's gateway
/// sometimes cuts a slow run with a 502 while the run keeps going, so polling
/// the run API is steadier.
const RUNS_URL: &str = "https://api.apify.com/v2/acts/apify~google-search-scraper/runs";
const RUN_URL: &str = "https://api.apify.com/v2/actor-runs/";
const DATASET_URL: &str = "https://api.apify.com/v2/datasets/";
const POLL_INTERVAL_SECS: u64 = 2;
const MAX_POLLS: usize = 90;

#[async_trait]
impl SearchEngine for ApifyEngine {
    fn name(&self) -> &'static str {
        "apify"
    }

    fn freshness_tier(&self) -> FreshnessTier {
        // quickDateRange / beforeDate+afterDate are Google `tbs` filters.
        FreshnessTier::Applied
    }

    async fn search(
        &self,
        query: &str,
        limit: usize,
        opts: &SearchOptions,
        config: &Config,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let key = config
            .api_key("apify")
            .ok_or_else(|| SearchError::Engine("APIFY_API_KEY not set".to_string()))?;
        // Google shows ~10 organic results per page.
        let pages = limit.clamp(1, 30).div_ceil(10).min(3) as u64;
        let client = config
            .build_reqwest_client_for_slow("api.apify.com", RUN_TIMEOUT_SECS)
            .map_err(|e| SearchError::Http(format!("apify client: {}", e)))?;

        let cjk = has_cjk(query);
        let mut body = serde_json::json!({
            "queries": query,
            "maxPagesPerQuery": pages,
            "countryCode": if cjk { "cn" } else { "us" },
            "languageCode": if cjk { "zh-CN" } else { "en" },
            "searchLanguage": if cjk { "zh-CN" } else { "en" },
            // Skip the extra-cost add-ons; plain organic results only.
            "saveHtml": false,
            "saveHtmlToKeyValueStore": false,
        });
        if let Some(freshness) = &opts.freshness {
            match freshness {
                Freshness::Range { start, end } => {
                    body["beforeDate"] = serde_json::json!(end);
                    body["afterDate"] = serde_json::json!(start);
                }
                other => {
                    let code = match other {
                        Freshness::Day => "d1",
                        Freshness::Week => "w1",
                        Freshness::Month => "m1",
                        Freshness::Year => "y1",
                        Freshness::Range { .. } => unreachable!(),
                    };
                    body["quickDateRange"] = serde_json::json!(code);
                }
            }
        }

        // 1. Start the actor run.
        let start = client
            .post(RUNS_URL)
            .bearer_auth(key)
            .json(&body)
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("apify start: {}", e)))?;
        let status = start.status();
        let text = start
            .text()
            .await
            .map_err(|e| SearchError::Http(format!("apify start body: {}", e)))?;
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            return Err(SearchError::Engine(format!(
                "apify auth failed (HTTP {}); check APIFY_API_KEY",
                status.as_u16()
            )));
        }
        if status == reqwest::StatusCode::PAYMENT_REQUIRED {
            return Err(SearchError::Engine(
                "apify monthly usage credit exhausted (HTTP 402)".to_string(),
            ));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError::Blocked("apify rate limited (HTTP 429)".to_string()));
        }
        if !status.is_success() {
            return Err(SearchError::Http(format!(
                "apify start HTTP {}: {}",
                status.as_u16(),
                text.chars().take(300).collect::<String>()
            )));
        }
        let started: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| SearchError::Parse(format!("apify start json: {}", e)))?;
        let run_id = started["data"]["id"]
            .as_str()
            .ok_or_else(|| SearchError::Parse("apify: no run id".to_string()))?;
        let dataset_id = started["data"]["defaultDatasetId"]
            .as_str()
            .ok_or_else(|| SearchError::Parse("apify: no dataset id".to_string()))?;

        // 2. Poll until the run finishes.
        let mut finished = false;
        for _ in 0..MAX_POLLS {
            tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;
            let resp = client
                .get(format!("{}{}", RUN_URL, run_id))
                .bearer_auth(key)
                .send()
                .await
                .map_err(|e| SearchError::Http(format!("apify poll: {}", e)))?;
            let body = resp
                .text()
                .await
                .map_err(|e| SearchError::Http(format!("apify poll body: {}", e)))?;
            let run: serde_json::Value = serde_json::from_str(&body)
                .map_err(|e| SearchError::Parse(format!("apify poll json: {}", e)))?;
            match run["data"]["status"].as_str().unwrap_or("") {
                "SUCCEEDED" => {
                    finished = true;
                    break;
                }
                "FAILED" | "ABORTED" | "TIMED-OUT" => {
                    let msg = run["data"]["statusMessage"]
                        .as_str()
                        .unwrap_or("run did not succeed");
                    return Err(SearchError::Engine(format!("apify run {}: {}", run_id, msg)));
                }
                _ => {}
            }
        }
        if !finished {
            return Err(SearchError::Engine(format!(
                "apify run {} did not finish within {}s",
                run_id,
                (MAX_POLLS as u64) * POLL_INTERVAL_SECS
            )));
        }

        // 3. Read the dataset items.
        let resp = client
            .get(format!(
                "{}{}/items?clean=true&format=json",
                DATASET_URL, dataset_id
            ))
            .bearer_auth(key)
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("apify dataset: {}", e)))?;
        let text = resp
            .text()
            .await
            .map_err(|e| SearchError::Http(format!("apify dataset body: {}", e)))?;
        let items: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| SearchError::Parse(format!("apify dataset json: {}", e)))?;
        let pages_arr = items
            .as_array()
            .ok_or_else(|| SearchError::Parse("apify: dataset is not an array".to_string()))?;

        let mut out = Vec::new();
        for page in pages_arr {
            if page.get("#error").and_then(|e| e.as_bool()).unwrap_or(false) {
                let msg = page
                    .get("#debug")
                    .and_then(|d| d.get("errorMessages"))
                    .and_then(|m| m.as_array())
                    .map(|m| {
                        m.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
                    .unwrap_or_default();
                if out.is_empty() {
                    return Err(SearchError::Engine(format!("apify actor error: {}", msg)));
                }
                break;
            }
            let Some(results) = page.get("organicResults").and_then(|r| r.as_array()) else {
                continue;
            };
            for r in results {
                let Some(url) = r.get("url").and_then(|u| u.as_str()) else {
                    continue;
                };
                let title = r.get("title").and_then(|t| t.as_str()).unwrap_or("");
                let desc = r
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .replace('\n', " ");
                out.push(SearchResult {
                    title: title.to_string(),
                    description: desc,
                    source: source_of(url, r.get("displayedUrl").and_then(|d| d.as_str())),
                    url: url.to_string(),
                    engine: "apify".to_string(),
                    summary: None,
                    engines: Vec::new(),
                });
                if out.len() >= limit {
                    return Ok(out);
                }
            }
        }
        Ok(out)
    }
}

/// Prefer the human-readable domain Google displays; Google sometimes hands
/// back a `google.com/goto?url=...` redirect whose own host is useless.
fn source_of(url: &str, displayed: Option<&str>) -> String {
    if let Some(d) = displayed {
        let host = d
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('›')
            .trim_end_matches('/');
        if !host.is_empty() {
            return host.to_string();
        }
    }
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '\u{4E00}'..='\u{9FFF}'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_prefers_displayed_domain() {
        assert_eq!(
            source_of(
                "https://www.google.com/goto?url=abc",
                Some("docs.rs › tokio › macro")
            ),
            "docs.rs"
        );
        assert_eq!(
            source_of("https://tokio.rs/tokio/tutorial/select", None),
            "tokio.rs"
        );
    }

    #[test]
    fn cjk_detection() {
        assert!(has_cjk("小米SU7"));
        assert!(!has_cjk("tokio select"));
    }
}
