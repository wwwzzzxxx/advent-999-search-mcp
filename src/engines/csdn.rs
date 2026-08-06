use async_trait::async_trait;
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct CsdnEngine;

#[async_trait]
impl SearchEngine for CsdnEngine {
    fn name(&self) -> &'static str { "csdn" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let client = config.build_reqwest_client()
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let mut all_results = Vec::new();
        let mut page = 1u32;

        while all_results.len() < limit {
            let resp = client.get("https://so.csdn.net/api/v3/search")
                .query(&[("q", query), ("p", &page.to_string())])
                .header("Pragma", "no-cache")
                .header("User-Agent", "Apifox/1.0.0 (https://apifox.com)")
                .header("Accept", "*/*")
                .header("Host", "so.csdn.net")
                .send()
                .await
                .map_err(|e| SearchError::Http(format!("CSDN request failed: {}", e)))?;

            let data: serde_json::Value = resp.json().await
                .map_err(|e| SearchError::Parse(format!("CSDN JSON parse failed: {}", e)))?;

            let result_vos = match data.get("result_vos").and_then(|v| v.as_array()) {
                Some(arr) => arr,
                None => break,
            };

            if result_vos.is_empty() { break; }

            for item in result_vos {
                let title = item["title"].as_str().unwrap_or("").to_string();
                let url = item["url_location"].as_str().unwrap_or("").to_string();
                let description = item["digest"].as_str().unwrap_or("").to_string();
                let source = item["nickname"].as_str().unwrap_or("").to_string();

                if title.is_empty() && url.is_empty() { continue; }

                all_results.push(SearchResult {
                    title,
                    url,
                    description,
                    source,
                    engine: "csdn".to_string(),
                    summary: None,
                });

                if all_results.len() >= limit { break; }
            }

            page += 1;
        }

        all_results.truncate(limit);
        Ok(all_results)
    }
}
