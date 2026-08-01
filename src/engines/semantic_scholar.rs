use async_trait::async_trait;
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

/// Semantic Scholar — free academic paper search with citation counts.
///
/// API: `https://api.semanticscholar.org/graph/v1/paper/search?query=<q>&fields=...`
/// - With a free API key (x-api-key header): 1 RPS dedicated pool.
/// - Without a key: shared pool, frequent 429s. We REQUIRE the key via the
///   `SEMANTIC_SCHOLAR_API_KEY` env var (engine is disabled without it).
/// - Returns JSON with citationCount, externalIds (DOI/arXiv), venue, etc.
pub struct SemanticScholarEngine;

const S2_URL: &str = "https://api.semanticscholar.org/graph/v1/paper/search";
const S2_FIELDS: &str = "title,abstract,authors,year,venue,citationCount,externalIds,openAccessPdf,url";

#[async_trait]
impl SearchEngine for SemanticScholarEngine {
    fn name(&self) -> &'static str { "semantic_scholar" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let key = config.semantic_scholar_api_key.as_deref()
            .ok_or_else(|| SearchError::Engine(
                "SEMANTIC_SCHOLAR_API_KEY not set; set it to enable the semantic_scholar engine".to_string()
            ))?;

        let client = config.build_reqwest_client()
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let limit = std::cmp::min(limit, 100).max(1);

        let resp = client.get(S2_URL)
            .query(&[
                ("query", query),
                ("fields", S2_FIELDS),
                ("limit", &limit.to_string()),
            ])
            .header("x-api-key", key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("Semantic Scholar request failed: {}", e)))?;

        match resp.status() {
            reqwest::StatusCode::TOO_MANY_REQUESTS => {
                return Err(SearchError::Blocked(
                    "Semantic Scholar rate limited (429); slow down or upgrade API key".to_string()
                ));
            }
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                return Err(SearchError::Blocked(
                    "Semantic Scholar rejected the API key (401/403); check SEMANTIC_SCHOLAR_API_KEY".to_string()
                ));
            }
            _ => {}
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| SearchError::Parse(format!("Semantic Scholar JSON parse failed: {}", e)))?;

        let items = match data["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(Vec::new()),
        };

        let mut results = Vec::new();
        for item in items {
            let title = item["title"].as_str().unwrap_or("").trim().to_string();
            if title.is_empty() { continue; }

            // URL: prefer open-access PDF, else paper page
            let url = item["openAccessPdf"]["url"].as_str()
                .or_else(|| item["url"].as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    item["externalIds"]["ArXiv"].as_str()
                        .map(|id| format!("https://arxiv.org/abs/{}", id))
                })
                .unwrap_or_default();

            let authors: Vec<&str> = item["authors"].as_array()
                .map(|arr| arr.iter().filter_map(|a| a["name"].as_str()).collect())
                .unwrap_or_default();

            let year = item["year"].as_i64().unwrap_or(0);
            let venue = item["venue"].as_str().unwrap_or("");
            let citations = item["citationCount"].as_i64().unwrap_or(0);
            let doi = item["externalIds"]["DOI"].as_str().unwrap_or("");

            let mut description = String::new();
            if !authors.is_empty() {
                description.push_str(&format!("{} | ", authors.join(", ")));
            }
            if !venue.is_empty() {
                description.push_str(&format!("{} | ", venue));
            }
            if year > 0 {
                description.push_str(&format!("{} | ", year));
            }
            description.push_str(&format!("被引 {}", citations));
            if !doi.is_empty() {
                description.push_str(&format!(" | DOI: {}", doi));
            }

            results.push(SearchResult {
                title: title.chars().take(200).collect(),
                url: url.chars().take(500).collect(),
                description: description.chars().take(400).collect(),
                source: venue.to_string(),
                engine: "semantic_scholar".to_string(),
            });

            if results.len() >= limit { break; }
        }

        Ok(results)
    }
}
