use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub default_search_engine: String,
    pub allowed_search_engines: Vec<String>,
    pub proxy_url: Option<String>,
    pub use_proxy: bool,
    pub fetch_cookies: String,
    pub fetch_timeout_secs: u64,
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
        let fetch_cookies = env::var("FETCH_COOKIES").unwrap_or_default();
        if !fetch_cookies.is_empty() {
            eprintln!("🍪 Loaded {} bytes of cookies from FETCH_COOKIES env var", fetch_cookies.len());
        }
        let fetch_timeout = env::var("FETCH_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        Self {
            default_search_engine: default_engine,
            allowed_search_engines: allowed,
            proxy_url,
            use_proxy,
            fetch_cookies,
            fetch_timeout_secs: fetch_timeout,
        }
    }

    pub fn build_reqwest_client(&self) -> reqwest::Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36");

        if self.use_proxy {
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
