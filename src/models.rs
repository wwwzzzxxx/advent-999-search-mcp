use serde::Serialize;
use std::fmt;

/// A single search result item
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
    pub source: String,
    pub engine: String,
    /// Optional AI-synthesized summary (only populated by LLM-backed
    /// engines like deepseek; omitted from JSON when absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Partial failure for an engine
#[derive(Debug, Clone, Serialize)]
pub struct PartialFailure {
    pub engine: String,
    pub code: String,
    pub message: String,
}

/// Full search response matching the Node.js output format
#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub engines: Vec<String>,
    #[serde(rename = "totalResults")]
    pub total_results: usize,
    pub results: Vec<SearchResult>,
    #[serde(rename = "partialFailures")]
    pub partial_failures: Vec<PartialFailure>,
}

/// Search error type
#[derive(Debug)]
pub enum SearchError {
    Engine(String),
    Http(String),
    Parse(String),
    Blocked(String),
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchError::Engine(msg) => write!(f, "Engine error: {}", msg),
            SearchError::Http(msg) => write!(f, "HTTP error: {}", msg),
            SearchError::Parse(msg) => write!(f, "Parse error: {}", msg),
            SearchError::Blocked(msg) => write!(f, "Blocked: {}", msg),
        }
    }
}

impl std::error::Error for SearchError {}

/// Normalize engine name (matching Node.js behavior)
pub fn normalize_engine(engine: &str) -> String {
    let cleaned = engine.trim().to_lowercase();
    let compact: String = cleaned.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    match compact.as_str() {
        "exa" => "exa".to_string(),
        "bing" => "bing".to_string(),
        "csdn" => "csdn".to_string(),
        "juejin" => "juejin".to_string(),
        "startpage" => "startpage".to_string(),
        "sogou" | "sougou" | "搜狗" => "sogou".to_string(),
        "weixin" | "wechat" | "微信" | "公众号" => "weixin".to_string(),
        "dblp" => "dblp".to_string(),
        "semantic_scholar" | "semanticscholar" | "s2" => "semantic_scholar".to_string(),
        "ieee" | "ieeexplore" => "ieee".to_string(),
        "cnki" | "知网" => "cnki".to_string(),
        _ => cleaned,
    }
}
