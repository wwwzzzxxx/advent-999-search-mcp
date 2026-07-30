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

pub fn create_engine_map(config: &Config) -> Vec<Box<dyn SearchEngine>> {
    let engines: Vec<Box<dyn SearchEngine>> = vec![
        Box::new(exa::ExaEngine),
        Box::new(bing::BingEngine),
        Box::new(csdn::CsdnEngine),
        Box::new(juejin::JuejinEngine),
        Box::new(startpage::StartpageEngine),
        Box::new(sogou::SogouEngine),
    ];

    engines
        .into_iter()
        .filter(|e| config.is_engine_allowed(e.name()))
        .collect()
}
