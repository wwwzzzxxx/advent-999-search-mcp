//! Octen Search API (`engines: ["octen"]`).
//!
//! Ranks high on the Artificial Analysis Search Index; free balance is a
//! one-time signup credit, so it is opt-in via `OCTEN_API_KEY`.
//!
//! Docs: POST https://api.octen.ai/search with `{query, count}`; results carry
//! `title`, `url`, `highlight`, `time_published`.

use async_trait::async_trait;
use crate::config::Config;
use crate::filters::{FreshnessTier, SearchOptions};
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct OctenEngine;

const ENDPOINT: &str = "https://api.octen.ai/search";

#[async_trait]
impl SearchEngine for OctenEngine {
    fn name(&self) -> &'static str {
        "octen"
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
            .api_key("octen")
            .ok_or_else(|| SearchError::Engine("OCTEN_API_KEY not set".to_string()))?;
        let count = limit.clamp(1, 20);
        let client = config
            .build_reqwest_client_for("api.octen.ai")
            .map_err(|e| SearchError::Http(format!("octen client: {}", e)))?;

        let resp = client
            .post(ENDPOINT)
            .bearer_auth(key)
            .json(&serde_json::json!({ "query": query, "count": count }))
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("octen: {}", e)))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| SearchError::Http(format!("octen body: {}", e)))?;
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(SearchError::Engine(format!("octen auth or balance failed (HTTP {})", status.as_u16())));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError::Blocked("octen rate limited (HTTP 429)".to_string()));
        }
        if !status.is_success() {
            return Err(SearchError::Http(format!(
                "octen HTTP {}: {}",
                status.as_u16(),
                text.chars().take(200).collect::<String>()
            )));
        }

        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| SearchError::Parse(format!("octen json: {}", e)))?;
        if v["code"].as_i64().unwrap_or(0) != 0 {
            return Err(SearchError::Engine(format!(
                "octen error {}: {}",
                v["code"],
                v["msg"].as_str().unwrap_or("")
            )));
        }
        let items = v["data"]["results"].as_array().cloned().unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|r| {
                let url = r["url"].as_str()?.to_string();
                let mut desc = r["highlight"].as_str().unwrap_or("").replace('\n', " ");
                if let Some(t) = r["time_published"].as_str().filter(|t| !t.is_empty()) {
                    desc.push_str(&format!(" · {}", t.chars().take(10).collect::<String>()));
                }
                Some(SearchResult {
                    title: r["title"].as_str().unwrap_or("").to_string(),
                    description: desc,
                    source: host_of(&url),
                    url,
                    engine: "octen".to_string(),
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
