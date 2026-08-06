use async_trait::async_trait;
use crate::config::Config;
use crate::models::{SearchError, SearchResult};

#[async_trait]
pub trait SearchEngine: Send + Sync {
    fn name(&self) -> &'static str;
    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError>;
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

/// Engines that require an API key / cookie from the environment.
/// They are only added to the engine map when the credential is present;
/// otherwise they are disabled entirely (not listed, not callable).
fn credential_gated_engines(config: &Config) -> Vec<Box<dyn SearchEngine>> {
    let mut engines: Vec<Box<dyn SearchEngine>> = Vec::new();
    if config.has_deepseek_key() {
        engines.push(Box::new(deepseek::DeepseekEngine));
    }
    engines
}

pub fn create_engine_map(config: &Config) -> Vec<Box<dyn SearchEngine>> {
    let engines: Vec<Box<dyn SearchEngine>> = vec![
        Box::new(exa::ExaEngine),
        Box::new(bing::BingEngine),
        Box::new(csdn::CsdnEngine),
        Box::new(juejin::JuejinEngine),
        Box::new(startpage::StartpageEngine),
        Box::new(sogou::SogouEngine),
        Box::new(weixin::WeixinEngine),
        Box::new(dblp::DblpEngine),
        Box::new(cnki::CnkiEngine),
    ];

    engines
        .into_iter()
        .chain(credential_gated_engines(config))
        .filter(|e| config.is_engine_allowed(e.name()))
        .collect()
}
