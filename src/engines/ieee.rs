use async_trait::async_trait;
use crate::config::Config;
use crate::filters::{today_days, FreshnessTier, SearchOptions};
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

/// IEEE Xplore Metadata Search API — academic papers, standards, books.
///
/// Endpoint: `GET https://ieeexploreapi.ieee.org/api/v1/search/articles`
/// Auth: `apikey` query param, key comes ONLY from the `IEEE_API_KEY` env var
/// (never from any file). Disabled entirely when the env var is not set.
///
/// On a bad/disabled key the API answers HTTP 403 with an HTML body
/// ("Developer Inactive"); the special "waiting" state from a fresh key
/// email (key issued but not yet enabled by IEEE) behaves the same way.
pub struct IeeeEngine;

const IEEE_API_URL: &str = "https://ieeexploreapi.ieee.org/api/v1/search/articles";

#[async_trait]
impl SearchEngine for IeeeEngine {
    fn name(&self) -> &'static str { "ieee" }

    /// IEEE's Metadata API has real `start_date`/`end_date` filters (insert
    /// date, `YYYYMMDD`), so a freshness window maps onto it directly.
    fn freshness_tier(&self) -> FreshnessTier { FreshnessTier::Applied }

    async fn search(&self, query: &str, limit: usize, opts: &SearchOptions, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let api_key = config.ieee_api_key.clone()
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| SearchError::Engine(
                "IEEE engine requires the IEEE_API_KEY env var".to_string()
            ))?;

        let client = config.build_reqwest_client()
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let max_records = std::cmp::min(limit, 200).max(1);

        let mut params: Vec<(&str, String)> = vec![
            ("apikey", api_key.clone()),
            ("format", "json".to_string()),
            ("querytext", query.to_string()),
            ("max_records", max_records.to_string()),
            ("start_record", "1".to_string()),
        ];
        if let Some(f) = &opts.freshness {
            let (start, end) = f.compact_range(today_days());
            params.push(("start_date", start));
            params.push(("end_date", end));
        }

        let resp = client.get(IEEE_API_URL)
            .query(&params)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("IEEE request failed: {}", e)))?;

        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(SearchError::Blocked(
                "IEEE rejected the API key (403 Developer Inactive); the key may still be awaiting IEEE activation".to_string()
            ));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError::Blocked("IEEE rate limited (429); slow down requests".to_string()));
        }
        if !status.is_success() {
            return Err(SearchError::Http(format!("IEEE returned status {}", status)));
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| SearchError::Parse(format!("IEEE JSON parse failed: {}", e)))?;

        let articles = match data["articles"].as_array() {
            Some(arr) => arr,
            None => return Ok(Vec::new()), // no hits
        };

        let mut results = Vec::new();
        for item in articles {
            let title = item["title"].as_str().unwrap_or("").trim().to_string();
            if title.is_empty() { continue; }

            // URL: prefer the Xplore document page, else the DOI link.
            let url = item["html_url"].as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| item["abstract_url"].as_str().filter(|s| !s.is_empty()))
                .map(|s| s.to_string())
                .or_else(|| {
                    item["doi"].as_str().filter(|s| !s.is_empty())
                        .map(|d| format!("https://doi.org/{}", d))
                })
                .unwrap_or_default();

            let authors: Vec<String> = item["authors"]["authors"]
                .as_array()
                .map(|arr| arr.iter()
                    .filter_map(|a| a["full_name"].as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect())
                .unwrap_or_default();

            let venue = item["publication_title"].as_str().unwrap_or("");
            let year = match &item["publication_year"] {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => String::new(),
            };
            let year = if year == "null" || year == "0" { String::new() } else { year };
            let doi = item["doi"].as_str().unwrap_or("");
            let content_type = item["content_type"].as_str().unwrap_or("");
            let citing = item["citing_paper_count"].as_u64();

            let mut description = String::new();
            if !authors.is_empty() {
                description.push_str(&authors.join(", "));
            }
            if !venue.is_empty() {
                if !description.is_empty() { description.push_str(" | "); }
                description.push_str(venue);
            }
            if !year.is_empty() {
                if !description.is_empty() { description.push_str(" | "); }
                description.push_str(&year);
            }
            if !content_type.is_empty() {
                if !description.is_empty() { description.push_str(" | "); }
                description.push_str(content_type);
            }
            if let Some(c) = citing {
                if !description.is_empty() { description.push_str(" | "); }
                description.push_str(&format!("cited by {}", c));
            }
            if !doi.is_empty() {
                if !description.is_empty() { description.push_str(" | "); }
                description.push_str(&format!("DOI: {}", doi));
            }
            let abstract_text = item["abstract"].as_str().unwrap_or("").trim();
            if !abstract_text.is_empty() {
                if !description.is_empty() { description.push_str(" — "); }
                description.push_str(abstract_text);
            }

            let source = if !venue.is_empty() { venue } else { content_type };

            results.push(SearchResult {
                title: title.chars().take(200).collect(),
                url: url.chars().take(500).collect(),
                description: description.chars().take(600).collect(),
                source: source.chars().take(120).collect(),
                engine: "ieee".to_string(),
                summary: None,
                engines: Vec::new(),
            });

            if results.len() >= limit { break; }
        }

        Ok(results)
    }
}
