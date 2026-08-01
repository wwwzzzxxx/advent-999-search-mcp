use std::env;

/// Mainland-China domains that are reachable without a proxy and whose
/// anti-bot systems actively BLOCK proxy/datacenter IPs (Sogou captcha, etc).
/// Override with the DIRECT_DOMAINS env var; set to `none` to proxy everything.
const DEFAULT_DIRECT_DOMAINS: &[&str] = &[
    "sogou",       // 搜狗 + 搜狗微信搜索
    "weixin",      // 微信 (mp.weixin.qq.com 等)
    "baidu",       // 百度
    "bilibili",    // B站
    "zhihu",       // 知乎
    "csdn",        // CSDN
    "juejin",      // 掘金
    "xiaohongshu", // 小红书
];

#[derive(Debug, Clone)]
pub struct Config {
    pub default_search_engine: String,
    pub allowed_search_engines: Vec<String>,
    pub proxy_url: Option<String>,
    pub use_proxy: bool,
    /// Domain substrings that must bypass the proxy (mainland direct access).
    pub direct_domains: Vec<String>,
    pub fetch_cookies: String,
    pub fetch_timeout_secs: u64,
    /// Optional IEEE Xplore API key (https://developer.ieee.org).
    /// When unset, the `ieee` engine is disabled.
    pub ieee_api_key: Option<String>,
    /// Optional Semantic Scholar API key (https://www.semanticscholar.org/product/api).
    /// When unset, the `semantic_scholar` engine is disabled.
    pub semantic_scholar_api_key: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let default_engine = env::var("DEFAULT_SEARCH_ENGINE")
            .unwrap_or_else(|_| "exa".to_string());

        let allowed = env::var("ALLOWED_SEARCH_ENGINES")
            .map(|s| s.split(',').map(|e| e.trim().to_string()).collect())
            .unwrap_or_default();

        let proxy_url = env::var("PROXY_URL").ok();
        let use_proxy = env::var("USE_PROXY").map(|v| v != "false").unwrap_or(true);
        let direct_domains = match env::var("DIRECT_DOMAINS") {
            // Unset → mainland defaults (sogou/weixin/baidu/... go direct).
            Err(_) => DEFAULT_DIRECT_DOMAINS.iter().map(|s| s.to_string()).collect(),
            // `none` or empty → no bypass at all, everything goes through the proxy
            // (for users outside mainland China).
            Ok(v) if v.trim().is_empty() || v.trim().eq_ignore_ascii_case("none") => Vec::new(),
            // Custom comma-separated list → override the defaults.
            Ok(v) => v.split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect(),
        };
        let fetch_cookies = env::var("FETCH_COOKIES").unwrap_or_default();
        if !fetch_cookies.is_empty() {
            eprintln!("🍪 Loaded {} bytes of cookies from FETCH_COOKIES env var", fetch_cookies.len());
        }
        let fetch_timeout = env::var("FETCH_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        // Optional API keys — engines are disabled when unset.
        let ieee_api_key = env::var("IEEE_API_KEY").ok().filter(|k| !k.trim().is_empty());
        let semantic_scholar_api_key = env::var("SEMANTIC_SCHOLAR_API_KEY")
            .ok().filter(|k| !k.trim().is_empty());

        if ieee_api_key.is_some() {
            eprintln!("🔑 IEEE_API_KEY set, ieee engine enabled");
        }
        if semantic_scholar_api_key.is_some() {
            eprintln!("🔑 SEMANTIC_SCHOLAR_API_KEY set, semantic_scholar engine enabled");
        }

        Self {
            default_search_engine: default_engine,
            allowed_search_engines: allowed,
            proxy_url,
            use_proxy,
            direct_domains,
            fetch_cookies,
            fetch_timeout_secs: fetch_timeout,
            ieee_api_key,
            semantic_scholar_api_key,
        }
    }

    /// Whether an engine requiring credentials is available.
    pub fn has_ieee_key(&self) -> bool { self.ieee_api_key.is_some() }
    pub fn has_semantic_scholar_key(&self) -> bool { self.semantic_scholar_api_key.is_some() }

    /// Whether requests to this host must bypass the proxy entirely.
    ///
    /// True for domains in DIRECT_DOMAINS (mainland sites that block proxy IPs
    /// or are directly reachable); also true when USE_PROXY=false.
    pub fn should_bypass_proxy(&self, domain: &str) -> bool {
        if !self.use_proxy {
            return true;
        }
        let d = domain.to_lowercase();
        self.direct_domains.iter().any(|pat| d.contains(&pat.to_lowercase()))
    }

    /// Build a reqwest client, applying the proxy only when the target domain
    /// should use it (i.e. NOT bypassing).
    pub fn build_reqwest_client_for(&self, domain: &str) -> reqwest::Result<reqwest::Client> {
        self.build_reqwest_client_with_proxy(!self.should_bypass_proxy(domain))
    }

    pub fn build_reqwest_client(&self) -> reqwest::Result<reqwest::Client> {
        self.build_reqwest_client_with_proxy(self.use_proxy)
    }

    /// Build a reqwest client, optionally bypassing the proxy entirely.
    ///
    /// Sogou / WeChat domains block datacenter and proxy IPs with captchas,
    /// so those requests must always go direct (use_proxy = false).
    pub fn build_reqwest_client_with_proxy(&self, use_proxy: bool) -> reqwest::Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36");

        if use_proxy {
            if let Some(ref url) = self.proxy_url {
                let proxy = reqwest::Proxy::all(url).map_err(|e| {
                    eprintln!("⚠️ Invalid proxy URL '{}': {}", url, e);
                    reqwest::Error::from(e)
                })?;
                builder = builder.proxy(proxy);
            }
        }

        builder.build()
    }

    pub fn is_engine_allowed(&self, engine: &str) -> bool {
        if self.allowed_search_engines.is_empty() {
            return true;
        }
        self.allowed_search_engines.contains(&engine.to_string())
    }

    pub fn resolve_engines(&self, requested: &[String]) -> Vec<String> {
        if self.allowed_search_engines.is_empty() {
            return requested.to_vec();
        }
        let filtered: Vec<String> = requested
            .iter()
            .filter(|e| self.allowed_search_engines.contains(e))
            .cloned()
            .collect();
        if filtered.is_empty() {
            vec![self.default_search_engine.clone()]
        } else {
            filtered
        }
    }
}
