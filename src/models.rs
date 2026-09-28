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
    /// Every engine that returned this (de-duplicated) URL, in first-seen
    /// order. Only serialized when more than one engine agreed, so the common
    /// single-engine case keeps its original shape.
    #[serde(skip_serializing_if = "has_one_or_no_engine")]
    pub engines: Vec<String>,
}

fn has_one_or_no_engine(v: &[String]) -> bool {
    v.len() <= 1
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
    /// How many duplicate URLs were folded together (omitted when zero).
    #[serde(
        rename = "duplicatesRemoved",
        skip_serializing_if = "is_zero_usize"
    )]
    pub duplicates_removed: usize,
    /// Echo of the filters that were applied, including per-engine honesty
    /// about which engines could actually honour them (omitted when no filter
    /// was requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<serde_json::Value>,
}

fn is_zero_usize(v: &usize) -> bool {
    *v == 0
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
