//! DuckDuckGo HTML search (`engines: ["duckduckgo"]`).
//!
//! Free, no API key, no account. Useful as the zero-config fallback when a
//! keyed engine is unavailable. DuckDuckGo serves a plain HTML endpoint that
//! does not need JavaScript; heavy querying can trigger its anomaly page, so
//! this engine is throttled to one request at a time and reports the block
//! instead of retrying.

use async_trait::async_trait;
use scraper::{Html, Selector};
use crate::config::Config;
use crate::filters::{Freshness, FreshnessTier, SearchOptions};
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct DuckDuckGoEngine;

const ENDPOINT: &str = "https://html.duckduckgo.com/html/";

fn df_param(f: &Freshness) -> Option<&'static str> {
    match f {
        Freshness::Day => Some("d"),
        Freshness::Week => Some("w"),
        Freshness::Month => Some("m"),
        Freshness::Year => Some("y"),
        Freshness::Range { .. } => None,
    }
}

/// `//duckduckgo.com/l/?uddg=<encoded>` → the real target.
fn unwrap_redirect(href: &str) -> String {
    let absolute = if href.starts_with("//") {
        format!("https:{}", href)
    } else {
        href.to_string()
    };
    if let Ok(u) = url::Url::parse(&absolute) {
        if u.host_str() == Some("duckduckgo.com") {
            if let Some((_, target)) = u.query_pairs().find(|(k, _)| k == "uddg") {
                return target.to_string();
            }
        }
        return absolute;
    }
    href.to_string()
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

#[async_trait]
impl SearchEngine for DuckDuckGoEngine {
    fn name(&self) -> &'static str {
        "duckduckgo"
    }

    /// `df=d|w|m|y` is applied server-side; explicit date ranges are not
    /// supported by this endpoint.
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
        let count = limit.clamp(1, 20);
        let client = config
            .build_reqwest_client_for("duckduckgo.com")
            .map_err(|e| SearchError::Http(format!("ddg client: {}", e)))?;

        let mut req = client
            .get(ENDPOINT)
            .header("Accept", "text/html,application/xhtml+xml")
            .query(&[("q", query)]);
        if let Some(df) = opts.freshness.as_ref().and_then(df_param) {
            req = req.query(&[("df", df)]);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("ddg: {}", e)))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| SearchError::Http(format!("ddg body: {}", e)))?;
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError::Blocked("duckduckgo rate limited (HTTP 429)".to_string()));
        }
        if !status.is_success() {
            return Err(SearchError::Http(format!("ddg HTTP {}", status.as_u16())));
        }
        if body.contains("anomaly-modal") || body.contains("challenge-form") {
            return Err(SearchError::Blocked(
                "duckduckgo returned its anti-bot page; slow down or try another engine".to_string(),
            ));
        }

        let doc = Html::parse_document(&body);
        let block_sel = Selector::parse("div.result").expect("static selector");
        let link_sel = Selector::parse("a.result__a").expect("static selector");
        let snippet_sel = Selector::parse(".result__snippet").expect("static selector");

        let mut out = Vec::new();
        for block in doc.select(&block_sel) {
            let Some(link) = block.select(&link_sel).next() else { continue };
            let raw_href = link.value().attr("href").unwrap_or("");
            if raw_href.is_empty() {
                continue;
            }
            let target = unwrap_redirect(raw_href);
            if !target.starts_with("http") {
                continue;
            }
            let title: String = link.text().collect::<String>().trim().to_string();
            let description = block
                .select(&snippet_sel)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            out.push(SearchResult {
                title,
                source: host_of(&target),
                url: target,
                description,
                engine: "duckduckgo".to_string(),
                summary: None,
                engines: Vec::new(),
            });
            if out.len() >= count {
                break;
            }
        }

        if out.is_empty() {
            return Err(SearchError::Engine(
                "duckduckgo returned no parsable results".to_string(),
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_ddg_redirects() {
        assert_eq!(
            unwrap_redirect("//duckduckgo.com/l/?uddg=https%3A%2F%2Ftokio.rs%2F&rut=abc"),
            "https://tokio.rs/"
        );
        assert_eq!(unwrap_redirect("https://example.com/x"), "https://example.com/x");
    }
}
