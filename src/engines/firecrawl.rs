//! Firecrawl Search API (`engines: ["firecrawl"]`).
//!
//! Free plan: 1,000 credits/month (~500 searches). Key-gated:
//! `FIRECRAWL_API_KEY`. Only the `web` source is requested — no scraping, so
//! one search costs one credit instead of search + page.

use async_trait::async_trait;
use crate::config::Config;
use crate::filters::{Freshness, FreshnessTier, SearchOptions};
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct FirecrawlEngine;

const ENDPOINT: &str = "https://api.firecrawl.dev/v2/search";

fn tbs_param(f: &Freshness) -> Option<&'static str> {
    match f {
        Freshness::Day => Some("qdr:d"),
        Freshness::Week => Some("qdr:w"),
        Freshness::Month => Some("qdr:m"),
        Freshness::Year => Some("qdr:y"),
        Freshness::Range { .. } => None,
    }
}

#[async_trait]
impl SearchEngine for FirecrawlEngine {
    fn name(&self) -> &'static str {
        "firecrawl"
    }

    /// Preset windows go through Google-style `tbs=qdr:*`; an explicit date
    /// range is not mapped, hence best-effort rather than applied.
    fn freshness_tier(&self) -> FreshnessTier {
        FreshnessTier::BestEffort
    }

    async fn search(
        &self,
        query: &str,
        limit: usize,
        opts: &SearchOptions,
        config: &Config,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let key = config
            .api_key("firecrawl")
            .ok_or_else(|| SearchError::Engine("FIRECRAWL_API_KEY not set".to_string()))?;
        let count = limit.clamp(1, 20);
        let client = config
            .build_reqwest_client_for("api.firecrawl.dev")
            .map_err(|e| SearchError::Http(format!("firecrawl client: {}", e)))?;

        let mut body = serde_json::json!({
            "query": query,
            "limit": count,
            "sources": [{"type": "web"}],
        });
        if let Some(tbs) = opts.freshness.as_ref().and_then(tbs_param) {
            body["tbs"] = serde_json::json!(tbs);
        }

        let resp = client
            .post(ENDPOINT)
            .bearer_auth(key)
            .json(&body)
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("firecrawl: {}", e)))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| SearchError::Http(format!("firecrawl body: {}", e)))?;
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(SearchError::Engine(format!(
                "firecrawl auth failed (HTTP {})",
                status.as_u16()
            )));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError::Blocked("firecrawl rate limited (HTTP 429)".to_string()));
        }
        if !status.is_success() {
            return Err(SearchError::Http(format!(
                "firecrawl HTTP {}: {}",
                status.as_u16(),
                text.chars().take(200).collect::<String>()
            )));
        }

        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| SearchError::Parse(format!("firecrawl json: {}", e)))?;
        let items = v["data"]["web"]
            .as_array()
            .or_else(|| v["data"].as_array())
            .cloned()
            .unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|r| {
                let url = r["url"].as_str()?.to_string();
                let desc = r["description"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| r["markdown"].as_str())
                    .unwrap_or("")
                    .to_string();
                Some(SearchResult {
                    title: r["title"].as_str().unwrap_or("").to_string(),
                    description: desc,
                    source: host_of(&url),
                    url,
                    engine: "firecrawl".to_string(),
                    summary: None,
                    engines: Vec::new(),
                })
            })
            .take(count)
            .collect())
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}
