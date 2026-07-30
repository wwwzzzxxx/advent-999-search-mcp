use async_trait::async_trait;
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct JuejinEngine;

#[async_trait]
impl SearchEngine for JuejinEngine {
    fn name(&self) -> &'static str { "juejin" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let client = config.build_reqwest_client()
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let mut all_results = Vec::new();
        let mut cursor = "0".to_string();

        while all_results.len() < limit {
            let resp = client.get("https://api.juejin.cn/search_api/v1/search")
                .query(&[
                    ("aid", "2608"),
                    ("uuid", "7259393293459605051"),
                    ("spider", "0"),
                    ("query", query),
                    ("cursor", &cursor),
                    ("limit", &format!("{}", std::cmp::min(20, limit - all_results.len()))),
                    ("search_type", "0"),
                    ("sort_type", "0"),
                ])
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
                .header("Accept", "*/*")
                .header("Content-Type", "application/json")
                .send()
                .await
                .map_err(|e| SearchError::Http(format!("Juejin request failed: {}", e)))?;

            let data: serde_json::Value = resp.json().await
                .map_err(|e| SearchError::Parse(format!("Juejin JSON parse failed: {}", e)))?;

            if data["err_no"].as_i64().unwrap_or(-1) != 0 {
                let msg = data["err_msg"].as_str().unwrap_or("unknown");
                eprintln!("❌ Juejin API error: {}", msg);
                break;
            }

            let items = match data["data"].as_array() {
                Some(arr) => arr,
                None => break,
            };

            if items.is_empty() { break; }

            for item in items {
                let result_model = &item["result_model"];
                let article_info = &result_model["article_info"];
                let author_info = &result_model["author_user_info"];

                let title = item["title_highlight"].as_str()
                    .map(|s| s.replace("<em>", "").replace("</em>", ""))
                    .unwrap_or_default();
                let content = item["content_highlight"].as_str()
                    .map(|s| s.replace("<em>", "").replace("</em>", ""))
                    .unwrap_or_default();
                let article_id = result_model["article_id"].as_str().unwrap_or("");
                let user_name = author_info["user_name"].as_str().unwrap_or("");
                let category_name = result_model["category"]["category_name"].as_str().unwrap_or("");
                let digg = article_info["digg_count"].as_i64().unwrap_or(0);
                let views = article_info["view_count"].as_i64().unwrap_or(0);

                let url = if !article_id.is_empty() {
                    format!("https://juejin.cn/post/{}", article_id)
                } else { continue; };

                let description = format!("{} | 分类: {} | 👍 {} | 👀 {}", content, category_name, digg, views);
                let clean_title = title.replace("<em>", "").replace("</em>", "");

                all_results.push(SearchResult {
                    title: clean_title,
                    url,
                    description,
                    source: user_name.to_string(),
                    engine: "juejin".to_string(),
                });

                if all_results.len() >= limit { break; }
            }

            if !data["has_more"].as_bool().unwrap_or(false) { break; }
            cursor = data["cursor"].as_str().unwrap_or("0").to_string();
        }

        all_results.truncate(limit);
        Ok(all_results)
    }
}
