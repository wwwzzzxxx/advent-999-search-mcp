//! Parallel Search API (`engines: ["parallel"]`).
//!
//! Objective-style search that returns LLM-ready excerpts instead of snippets.
//! Key-gated: `PARALLEL_API_KEY`.
//!
//! Docs: POST https://api.parallel.ai/v1/search with `x-api-key`.
//! The endpoint rejects unknown fields (e.g. `max_results`) with a 422, so the
//! body is exactly `objective` + `search_queries` + `mode`, and the result
//! count is capped locally.

use async_trait::async_trait;
use crate::config::Config;
use crate::filters::{FreshnessTier, SearchOptions};
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct ParallelEngine;

const ENDPOINT: &str = "https://api.parallel.ai/v1/search";

#[async_trait]
impl SearchEngine for ParallelEngine {
    fn name(&self) -> &'static str {
        "parallel"
    }

    fn freshness_tier(&self) -> FreshnessTier {
        FreshnessTier::Unsupported
    }

    async fn search(
        &self,
        query: &str,
        limit: usize,
        _opts: &SearchOptions,
        config: &Config,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let key = config
            .api_key("parallel")
            .ok_or_else(|| SearchError::Engine("PARALLEL_API_KEY not set".to_string()))?;
        let count = limit.clamp(1, 20);
        let client = config
            .build_reqwest_client_for("api.parallel.ai")
            .map_err(|e| SearchError::Http(format!("parallel client: {}", e)))?;

        let body = serde_json::json!({
            "objective": query,
            "search_queries": [query],
            "mode": "fast",
        });

        let resp = client
            .post(ENDPOINT)
            .header("x-api-key", key)
            .json(&body)
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("parallel: {}", e)))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| SearchError::Http(format!("parallel body: {}", e)))?;
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(SearchError::Engine(format!(
                "parallel auth failed (HTTP {})",
                status.as_u16()
            )));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError::Blocked("parallel rate limited (HTTP 429)".to_string()));
        }
        if !status.is_success() {
            return Err(SearchError::Http(format!(
                "parallel HTTP {}: {}",
                status.as_u16(),
                text.chars().take(200).collect::<String>()
            )));
        }

        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| SearchError::Parse(format!("parallel json: {}", e)))?;
        let items = v["results"].as_array().cloned().unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|r| {
                let url = r["url"].as_str()?.to_string();
                let excerpt = r["excerpts"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| e.as_str())
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .unwrap_or_default();
                Some(SearchResult {
                    title: r["title"].as_str().unwrap_or("").to_string(),
                    description: excerpt,
                    source: host_of(&url),
                    url,
                    engine: "parallel".to_string(),
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
