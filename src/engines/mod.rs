use async_trait::async_trait;
use std::sync::Arc;
use crate::config::Config;
use crate::filters::{FreshnessTier, SearchOptions};
use crate::models::{SearchError, SearchResult};

#[async_trait]
pub trait SearchEngine: Send + Sync {
    fn name(&self) -> &'static str;

    async fn search(
        &self,
        query: &str,
        limit: usize,
        opts: &SearchOptions,
        config: &Config,
    ) -> Result<Vec<SearchResult>, SearchError>;

    /// How honestly this engine can honour a `freshness` filter. Defaults to
    /// `Unsupported`; engines that do something report their real confidence so
    /// the response never claims a filter was applied when it was not.
    fn freshness_tier(&self) -> FreshnessTier {
        FreshnessTier::Unsupported
    }

    /// Whether this engine honours the `topic` hint.
    fn supports_topic(&self) -> bool {
        false
    }
}

pub mod exa;
pub mod bing;
pub mod csdn;
pub mod juejin;
pub mod startpage;
pub mod sogou;
pub mod weixin;
pub mod dblp;
pub mod cnki;
pub mod deepseek;
pub mod ieee;
pub mod x;
pub mod firecrawl;
pub mod octen;
pub mod parallel;
pub mod serper;
pub mod apify;
pub mod duckduckgo;

/// Engines that require an API key / cookie from the environment.
/// They are only added to the engine map when the credential is present;
/// otherwise they are disabled entirely (not listed, not callable).
fn credential_gated_engines(config: &Config) -> Vec<Arc<dyn SearchEngine>> {
    let mut engines: Vec<Arc<dyn SearchEngine>> = Vec::new();
    if config.has_deepseek_key() {
        engines.push(Arc::new(deepseek::DeepseekEngine));
    }
    if config.has_ieee_key() {
        engines.push(Arc::new(ieee::IeeeEngine));
    }
    // X needs a logged-in x.com session; without auth_token + ct0 the search
    // endpoint is a login wall, so the engine is not registered at all.
    if crate::x::has_login(config) {
        engines.push(Arc::new(x::XEngine));
    }
    // Keyed web-search APIs: only engines whose key we actually hold.
    if config.api_key("firecrawl").is_some() {
        engines.push(Arc::new(firecrawl::FirecrawlEngine));
    }
    if config.api_key("octen").is_some() {
        engines.push(Arc::new(octen::OctenEngine));
    }
    if config.api_key("parallel").is_some() {
        engines.push(Arc::new(parallel::ParallelEngine));
    }
    // Real Google SERP results; replaces the blocked startpage scraper.
    if config.api_key("serper").is_some() {
        engines.push(Arc::new(serper::SerperEngine));
    }
    // Google SERP via Apify's hosted actor: free $5/month keeps refilling.
    if config.api_key("apify").is_some() {
        engines.push(Arc::new(apify::ApifyEngine));
    }
    engines
}

/// Build the engine set. Engines are held behind `Arc` so a single request can
/// run them concurrently (`tokio::task::JoinSet`) without cloning state.
pub fn create_engine_map(config: &Config) -> Vec<Arc<dyn SearchEngine>> {
    let engines: Vec<Arc<dyn SearchEngine>> = vec![
        Arc::new(exa::ExaEngine),
        Arc::new(bing::BingEngine),
        Arc::new(csdn::CsdnEngine),
        Arc::new(juejin::JuejinEngine),
        Arc::new(startpage::StartpageEngine),
        Arc::new(sogou::SogouEngine),
        Arc::new(weixin::WeixinEngine),
        Arc::new(dblp::DblpEngine),
        Arc::new(cnki::CnkiEngine),
        // No key, no account: the zero-config fallback.
        Arc::new(duckduckgo::DuckDuckGoEngine),
    ];

    engines
        .into_iter()
        .chain(credential_gated_engines(config))
        .filter(|e| config.is_engine_allowed(e.name()))
        .collect()
}
