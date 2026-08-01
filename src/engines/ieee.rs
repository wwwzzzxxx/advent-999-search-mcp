use async_trait::async_trait;
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

/// IEEE Xplore — computer science & engineering papers (API key required).
///
/// API: `https://ieeexploreapi.ieee.org/api/v1/search/articles?apikey=<key>&querytext=<q>&max_records=N`
/// - Free key registration at https://developer.ieee.org (personal use OK,
///   review takes a few business days).
/// - No public monthly quota, but max_records <= 200 per request and
///   querytext <= 10 words.
/// - Enabled only when the `IEEE_API_KEY` env var is set.
pub struct IeeeEngine;

const IEEE_URL: &str = "https://ieeexploreapi.ieee.org/api/v1/search/articles";

#[async_trait]
impl SearchEngine for IeeeEngine {
    fn name(&self) -> &'static str { "ieee" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let key = config.ieee_api_key.as_deref()
            .ok_or_else(|| SearchError::Engine(
                "IEEE_API_KEY not set; set it to enable the ieee engine".to_string()
            ))?;

        let client = config.build_reqwest_client()
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let max_records = std::cmp::min(limit, 200).max(1);

        let resp = client.get(IEEE_URL)
            .query(&[
                ("apikey", key),
                ("querytext", query),
                ("max_records", &max_records.to_string()),
                ("format", "json"),
            ])
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("IEEE request failed: {}", e)))?;

        match resp.status() {
            reqwest::StatusCode::TOO_MANY_REQUESTS | reqwest::StatusCode::SERVICE_UNAVAILABLE => {
                return Err(SearchError::Blocked(
                    "IEEE rate limited (429/503); slow down requests".to_string()
                ));
            }
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                return Err(SearchError::Blocked(
                    "IEEE rejected the API key (401/403); check IEEE_API_KEY".to_string()
                ));
            }
            _ => {}
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| SearchError::Parse(format!("IEEE JSON parse failed: {}", e)))?;

        let items = match data["articles"].as_array() {
            Some(arr) => arr,
            None => return Ok(Vec::new()),
        };

        let mut results = Vec::new();
        for item in items {
            let title = item["title"].as_str().unwrap_or("").trim().to_string();
            if title.is_empty() { continue; }

            let article_num = item["article_number"].as_str().unwrap_or("");
            let doi = item["doi"].as_str().unwrap_or("");

            // URL: prefer DOI, else IEEE article page
            let url = if !doi.is_empty() {
                format!("https://doi.org/{}", doi)
            } else if !article_num.is_empty() {
                format!("https://ieeexplore.ieee.org/document/{}", article_num)
            } else {
                String::new()
            };
            if url.is_empty() { continue; }

            let authors: Vec<&str> = item["authors"].as_array()
                .map(|arr| arr.iter().filter_map(|a| a["full_name"].as_str()).collect())
                .unwrap_or_default();

            let year = item["publication_year"].as_str().unwrap_or("");
            let publication = item["publication_title"].as_str().unwrap_or("");
            let abstract_text = item["abstract"].as_str().unwrap_or("");
            let citation_count = item["citing_paper_count"].as_i64().unwrap_or(0);

            let mut description = String::new();
            if !authors.is_empty() {
                description.push_str(&format!("{} | ", authors.join(", ")));
            }
            if !publication.is_empty() {
                description.push_str(&format!("{} | ", publication));
            }
            if !year.is_empty() {
                description.push_str(&format!("{} | ", year));
            }
            if citation_count > 0 {
                description.push_str(&format!("被引 {} | ", citation_count));
            }
            if !abstract_text.is_empty() {
                description.push_str(&abstract_text.chars().take(200).collect::<String>());
            }

            results.push(SearchResult {
                title: title.chars().take(200).collect(),
                url: url.chars().take(500).collect(),
                description: description.chars().take(400).collect(),
                source: publication.to_string(),
                engine: "ieee".to_string(),
            });

            if results.len() >= limit { break; }
        }

        Ok(results)
    }
}
