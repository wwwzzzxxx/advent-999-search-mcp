use async_trait::async_trait;
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

/// DBLP — computer science bibliography (free, no API key).
///
/// API: `https://dblp.org/search/publ/api?q=<query>&format=json&h=<hits>&f=<offset>`
/// Free, no key. Be polite: DBLP asks for at most 1-2 requests per second;
/// it responds with 429 + Retry-After when rate limited.
pub struct DblpEngine;

const DBLP_URL: &str = "https://dblp.org/search/publ/api";

#[async_trait]
impl SearchEngine for DblpEngine {
    fn name(&self) -> &'static str { "dblp" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let client = config.build_reqwest_client()
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let hits = std::cmp::min(limit, 100).max(1);

        let resp = client.get(DBLP_URL)
            .query(&[
                ("q", query),
                ("format", "json"),
                ("h", &hits.to_string()),
                ("f", "0"),
            ])
            .header("Accept", "application/json")
            // Override the client default `Accept-Encoding: gzip, deflate, br`:
            // dblp.org's server returns HTTP 500 when `br` (brotli) is offered.
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

        let body_text = resp.text().await
            .map_err(|e| SearchError::Http(format!("DBLP read body failed: {}", e)))?;

        let data: serde_json::Value = match serde_json::from_str(&body_text) {
            Ok(v) => v,
            Err(e) => {
                // DBLP occasionally serves an XML error page instead of JSON.
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
