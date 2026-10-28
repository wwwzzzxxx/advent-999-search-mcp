//! X / Twitter search engine (`web` with `engines: ["x"]`).
//!
//! Only registered when x.com login cookies (`auth_token` + `ct0`) are in the
//! cookie store — without them X's search endpoint is a login wall. The actual
//! GraphQL call is made by the `twscrape` CLI (see `src/x.rs`), which is
//! spawned per search and exits; nothing stays resident.
//!
//! X rate-limits per endpoint and per account and starts throttling before the
//! documented limit, so this engine is deliberately conservative: calls are
//! serialized with a random inter-call delay, the result count is capped, and
//! a locked account is reported instead of retried in a loop.

use async_trait::async_trait;
use crate::config::Config;
use crate::filters::{today_days, FreshnessTier, SearchOptions};
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct XEngine;

/// Hard cap for one search. X returns a GraphQL page even when asked for less;
/// keeping this low keeps both the token bill and the ban risk down.
const MAX_LIMIT: usize = 30;

impl XEngine {
    fn to_result(t: &serde_json::Value) -> SearchResult {
        let text = crate::x::text_of(t, "rawContent");
        let handle = crate::x::handle_of(t);
        let url = {
            let u = crate::x::text_of(t, "url");
            if u.is_empty() {
                format!(
                    "https://x.com/i/status/{}",
                    crate::x::text_of(t, "id_str")
                )
            } else {
                u
            }
        };
        let title = {
            let first = text.lines().next().unwrap_or("").trim();
            if first.is_empty() {
                format!("@{} on X", handle)
            } else if first.chars().count() > 100 {
                format!("{}…", first.chars().take(100).collect::<String>())
            } else {
                first.to_string()
            }
        };
        let description = format!(
            "{}\n\n♥ {} · 🔁 {} · 💬 {} · 👁 {} · {}",
            text,
            crate::x::num_of(t, "likeCount"),
            crate::x::num_of(t, "retweetCount"),
            crate::x::num_of(t, "replyCount"),
            crate::x::num_of(t, "viewCount"),
            crate::x::date_of(t),
        );
        SearchResult {
            title,
            url,
            description,
            source: "x.com".to_string(),
            engine: "x".to_string(),
            summary: None,
            engines: Vec::new(),
        }
    }
}

#[async_trait]
impl SearchEngine for XEngine {
    fn name(&self) -> &'static str {
        "x"
    }

    /// X's search accepts `since:` / `until:` operators, which are applied
    /// server-side — but timezone handling is X's, so this is reported as a
    /// hint rather than a verified exact filter.
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
        let limit = limit.clamp(1, MAX_LIMIT);
        let mut q = query.trim().to_string();
        if q.is_empty() {
            return Err(SearchError::Engine("empty x query".to_string()));
        }
        if let Some(freshness) = opts.freshness.as_ref() {
            let (start, end) = freshness.iso_range(today_days());
            q = format!("{} since:{} until:{}", q, start, end);
        }

        let tweets = crate::x::search_tweets(config, &q, limit)
            .await
            .map_err(SearchError::Engine)?;

        Ok(tweets.iter().take(limit).map(Self::to_result).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_a_tweet_to_a_search_result() {
        let tweet = json!({
            "id_str": "20",
            "url": "https://x.com/jack/status/20",
            "date": "2006-03-21 20:50:14+00:00",
            "rawContent": "just setting up my twttr\nsecond line",
            "user": { "username": "jack", "displayname": "jack" },
            "likeCount": 310000,
            "retweetCount": 130000,
            "replyCount": 18000,
            "viewCount": 22000
        });
        let r = XEngine::to_result(&tweet);
        assert_eq!(r.engine, "x");
        assert_eq!(r.source, "x.com");
        assert_eq!(r.url, "https://x.com/jack/status/20");
        assert_eq!(r.title, "just setting up my twttr");
        assert!(r.description.contains("♥ 310000"));
        assert!(r.engines.is_empty());
    }

    #[test]
    fn falls_back_to_a_stable_url_and_handle_title() {
        let tweet = json!({
            "id_str": "42",
            "date": "2026-01-01 00:00:00+00:00",
            "rawContent": "",
            "user": { "username": "someone" }
        });
        let r = XEngine::to_result(&tweet);
        assert_eq!(r.url, "https://x.com/i/status/42");
        assert_eq!(r.title, "@someone on X");
    }
}
