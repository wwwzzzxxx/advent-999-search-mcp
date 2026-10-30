//! Serper.dev Google SERP API (`engines: ["serper"]`).
//!
//! Returns real Google web results (organic listings) as JSON. This is the
//! engine to use when keyword-exact Google results matter — fresh news, entity
//! lookups, `site:` style research — because the free scrapers that front
//! Google (startpage) are blocked from datacenter IPs.
//!
//! Free tier: 2,500 queries on signup, no credit card; then $1 / 1k. Opt-in via
//! `SERPER_API_KEY`, so the engine only exists when a key is configured.
//!
//! Docs: POST https://google.serper.dev/search with `X-API-KEY`, body
//! `{q, num, gl, hl, tbs}`; results are in `organic[]` with `title`, `link`,
//! `snippet` and an optional `date`.

use async_trait::async_trait;
use crate::config::Config;
use crate::filters::{Freshness, FreshnessTier, SearchOptions};
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct SerperEngine;

const ENDPOINT: &str = "https://google.serper.dev/search";

#[async_trait]
impl SearchEngine for SerperEngine {
    fn name(&self) -> &'static str {
        "serper"
    }

    fn freshness_tier(&self) -> FreshnessTier {
        // `tbs=qdr:*` / `cdr:1` are real Google server-side filters.
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
            .api_key("serper")
            .ok_or_else(|| SearchError::Engine("SERPER_API_KEY not set".to_string()))?;
        let num = limit.clamp(1, 100);
        let client = config
            .build_reqwest_client_for("google.serper.dev")
            .map_err(|e| SearchError::Http(format!("serper client: {}", e)))?;

        let mut body = serde_json::json!({
            "q": query,
            "num": num,
            // Google localises by `hl`; CJK queries need zh-CN to keep the
            // snippets in the language the user asked in.
            "hl": if has_cjk(query) { "zh-cn" } else { "en" },
        });
        if let Some(freshness) = &opts.freshness {
            body["tbs"] = serde_json::json!(tbs_for(freshness));
        }

        let resp = client
            .post(ENDPOINT)
            .header("X-API-KEY", key)
            .json(&body)
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("serper: {}", e)))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| SearchError::Http(format!("serper body: {}", e)))?;

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(SearchError::Engine(format!(
                "serper auth failed (HTTP {}); check SERPER_API_KEY",
                status.as_u16()
            )));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError::Blocked("serper rate limited (HTTP 429)".to_string()));
        }
        if !status.is_success() {
            return Err(SearchError::Http(format!(
                "serper HTTP {}: {}",
                status.as_u16(),
                text.chars().take(200).collect::<String>()
            )));
        }

        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| SearchError::Parse(format!("serper json: {}", e)))?;
        if let Some(msg) = v["message"].as_str().filter(|m| !m.is_empty()) {
            return Err(SearchError::Engine(format!("serper error: {}", msg)));
        }

        let items = v["organic"].as_array().cloned().unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|r| {
                let url = r["link"].as_str()?.to_string();
                let mut desc = r["snippet"].as_str().unwrap_or("").replace('\n', " ");
                if let Some(date) = r["date"].as_str().filter(|d| !d.is_empty()) {
                    desc.push_str(&format!(" · {}", date));
                }
                Some(SearchResult {
                    title: r["title"].as_str().unwrap_or("").to_string(),
                    description: desc,
                    source: host_of(&url),
                    url,
                    engine: "serper".to_string(),
                    summary: None,
                    engines: Vec::new(),
                })
            })
            .take(num)
            .collect())
    }
}

/// Google's `tbs` token for a time window. `qdr:*` is relative, `cdr:1` an
/// explicit range; both are honoured server-side by Google.
fn tbs_for(freshness: &Freshness) -> String {
    match freshness {
        Freshness::Day => "qdr:d".to_string(),
        Freshness::Week => "qdr:w".to_string(),
        Freshness::Month => "qdr:m".to_string(),
        Freshness::Year => "qdr:y".to_string(),
        Freshness::Range { start, end } => format!(
            "cdr:1,cd_min:{},cd_max:{}",
            google_date(start),
            google_date(end)
        ),
    }
}

/// `YYYY-MM-DD` → Google's `M/D/YYYY` (what `tbs` expects).
fn google_date(iso: &str) -> String {
    let mut parts = iso.split('-');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(y), Some(m), Some(d)) => {
            let m = m.trim_start_matches('0');
            let d = d.trim_start_matches('0');
            format!("{}/{}/{}", m, d, y)
        }
        _ => iso.to_string(),
    }
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '\u{4E00}'..='\u{9FFF}'))
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_maps_to_tbs() {
        assert_eq!(tbs_for(&Freshness::Day), "qdr:d");
        assert_eq!(tbs_for(&Freshness::Week), "qdr:w");
        assert_eq!(
            tbs_for(&Freshness::Range {
                start: "2026-10-01".into(),
                end: "2026-10-30".into()
            }),
            "cdr:1,cd_min:10/1/2026,cd_max:10/30/2026"
        );
    }

    #[test]
    fn cjk_detection() {
        assert!(has_cjk("小米SU7 事故"));
        assert!(!has_cjk("tokio select macro"));
    }
}
