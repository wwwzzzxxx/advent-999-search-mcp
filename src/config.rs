use std::env;

/// Mainland-China domains that are reachable without a proxy and whose
/// anti-bot systems actively BLOCK proxy/datacenter IPs (Sogou captcha, etc).
/// Override with the DIRECT_DOMAINS env var; set to `none` to proxy everything.
const DEFAULT_DIRECT_DOMAINS: &[&str] = &[
    "sogou",       // 搜狗 + 搜狗微信搜索
    "weixin",      // 微信 (mp.weixin.qq.com 等)
    "baidu",       // 百度
    "bilibili",    // B站
    "hdslb",       // B站静态资源 CDN (aisubtitle.hdslb.com 字幕文件等)
    "zhihu",       // 知乎
    "csdn",        // CSDN
    "juejin",      // 掘金
    "xiaohongshu", // 小红书
    "deepseek",    // DeepSeek 官方 API (国内直连)
    "cnki",        // 知网 (scholar/kns/www.cnki.net 在腾讯 EdgeOne 后，代理/数据中心 IP 一律 418，必须直连)
    "dblp",        // DBLP (dblp.org / dblp.uni-trier.de 对代理 IP 返回 Anubis 反爬挑战，国内直连可用)
];

#[derive(Debug, Clone)]
pub struct Config {
    pub default_search_engine: String,
    pub allowed_search_engines: Vec<String>,
    pub proxy_url: Option<String>,
    pub use_proxy: bool,
    /// Domain substrings that must bypass the proxy (mainland direct access).
    pub direct_domains: Vec<String>,
    /// `twscrape` executable used for X/Twitter reads (default `twscrape`).
    /// The engine and `get_page` silently fall back when it is not installed.
    pub twscrape_path: String,
    /// Minimum spacing between consecutive `twscrape` calls, in ms.
    /// A random 0–3s jitter is added on top so the cadence is not robotic.
    pub x_min_interval_ms: u64,
    /// Cookies seeded from the legacy `FETCH_COOKIES` env var, already split
    /// by owning domain. Requests only receive the entries matching their own
    /// host — see [`Config::cookies_for`].
    pub env_cookies: Vec<(String, String)>,
    pub fetch_timeout_secs: u64,
    /// DeepSeek web-search key — either DeepSeek official API key
    /// (`sk-` + 32 hex, 35 chars) or an OpenCode Go subscription key
    /// (`sk-` + 64 chars). Auto-detected by format; override with
    /// `DEEPSEEK_API_MODE=official|go`. When unset (and no OpenCode
    /// auth.json), the `deepseek` engine is disabled.
    pub deepseek_api_key: Option<String>,
    /// Optional override for DeepSeek key format detection:
    /// "official" (api.deepseek.com) or "go" (opencode.ai/zen/go).
    pub deepseek_api_mode: Option<String>,
    /// IEEE Xplore Metadata Search API key — enables the `ieee` engine.
    /// Get one at https://developer.ieee.org (key needs IEEE activation
    /// during US business hours before first use). Read ONLY from this
    /// env var (never from any file).
    pub ieee_api_key: Option<String>,
    /// Optional keys for the keyed web-search engines, keyed by engine name
    /// (`firecrawl`, `octen`, `parallel`). Read from `FIRECRAWL_API_KEY` and
    /// `OCTEN_API_KEY` / `PARALLEL_API_KEY` — engines without a key are not
    /// registered.
    pub search_api_keys: std::collections::HashMap<String, String>,
    /// How long a cookie-cache entry is considered fresh, in seconds.
    ///
    /// Advisory only: stale entries are still used (the remote server is the
    /// real authority) but a warning is logged. Defaults to 1200s (~20 min),
    /// matching the observed lifetime of Sogou's `SNUID`.
    /// Override with `COOKIE_CACHE_TTL` (0 disables the warning).
    pub cookie_cache_ttl_secs: u64,
    /// Whether the `import_cookies` tool is available. Off unless
    /// `ADVENT_ALLOW_COOKIE_IMPORT=1`; the tool is then not even registered.
    pub cookie_import_enabled: bool,
    /// Whether `web` may attach related Engram memories.
    pub engram_search_enabled: bool,
    /// Child MCP client used for optional Engram searches.
    pub engram: crate::engram::EngramClient,
}

impl Config {
    pub fn from_env() -> Self {
        // Out-of-the-box default. Override per deployment with
        // DEFAULT_SEARCH_ENGINE (e.g. `exa` where its key is configured).
        let default_engine = env::var("DEFAULT_SEARCH_ENGINE")
            .unwrap_or_else(|_| "bing".to_string());

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
        // Long-lived login cookies. The env var is a flat, domain-less blob, so
        // it is split by cookie owner here; unknown names are dropped and
        // reported instead of being sprayed across every fetched host.
        let raw_fetch_cookies = env::var("FETCH_COOKIES").unwrap_or_default();
        let (env_cookies, unknown_cookies) = crate::cookie_cache::seed_env(&raw_fetch_cookies);
        if !raw_fetch_cookies.is_empty() {
            let domains: Vec<String> = env_cookies
                .iter()
                .map(|(d, h)| {
                    format!("{} ({})", d, crate::cookie_cache::cookie_names(h).join(","))
                })
                .collect();
            eprintln!(
                "🍪 FETCH_COOKIES: {} bytes, scoped to {}",
                raw_fetch_cookies.len(),
                if domains.is_empty() { "no known domain".to_string() } else { domains.join("; ") }
            );
            if !unknown_cookies.is_empty() {
                eprintln!(
                    "⚠️ FETCH_COOKIES: dropped unrecognized cookies (no known owner domain): {}. \
                     Store them per domain with set_cookies instead.",
                    unknown_cookies.join(",")
                );
            }
        }
        let fetch_timeout = env::var("FETCH_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        // Optional API keys — engines are disabled when unset.
        // DeepSeek: key comes ONLY from the DEEPSEEK_API_KEY env var.
        let deepseek_api_key = env::var("DEEPSEEK_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty());
        let deepseek_api_mode = env::var("DEEPSEEK_API_MODE")
            .ok().filter(|m| !m.trim().is_empty());

        if let Some(k) = &deepseek_api_key {
            let mode = match deepseek_api_mode.as_deref() {
                Some("official") => "official (forced)",
                Some("go") => "go (forced)",
                _ => {
                    if k.len() == 35 && k.starts_with("sk-") {
                        "official (auto)"
                    } else {
                        "go (auto)"
                    }
                }
            };
            eprintln!("🔑 DEEPSEEK_API_KEY set, deepseek engine enabled ({})", mode);
        }

        // IEEE Xplore key — enables the `ieee` engine when set.
        let ieee_api_key = env::var("IEEE_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty());
        // Keyed services we can activate without a card:
        // Firecrawl 1,000 credits/month, Octen one-time $5 credit,
        // Parallel one-time $20 credit.
        let mut search_api_keys = std::collections::HashMap::new();
        for (engine, var) in [
            ("firecrawl", "FIRECRAWL_API_KEY"),
            ("octen", "OCTEN_API_KEY"),
            ("parallel", "PARALLEL_API_KEY"),
        ] {
            if let Ok(k) = env::var(var) {
                let k = k.trim().to_string();
                if !k.is_empty() {
                    eprintln!("🔑 {} set, {} engine enabled", var, engine);
                    search_api_keys.insert(engine.to_string(), k);
                }
            }
        }

        if ieee_api_key.is_some() {
            eprintln!("🔑 IEEE_API_KEY set, ieee engine enabled");
        }

        // Cookie-cache freshness window (advisory warning threshold).
        let cookie_cache_ttl_secs = env::var("COOKIE_CACHE_TTL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1200);

        // Browser cookie import is a credential-access capability, so it is
        // off unless the user explicitly turns it on. When off the tool is not
        // registered at all.
        let cookie_import_enabled = env::var("ADVENT_ALLOW_COOKIE_IMPORT")
            .map(|v| v.trim() == "1")
            .unwrap_or(false);
        if cookie_import_enabled {
            eprintln!("🔓 ADVENT_ALLOW_COOKIE_IMPORT=1: import_cookies tool enabled");
        }

        let engram_search_enabled = env::var("ADVENT_ENGRAM_ENABLED")
            .map(|v| v.trim() == "1")
            .unwrap_or(false);
        if engram_search_enabled {
            eprintln!("🧠 ADVENT_ENGRAM_ENABLED=1: web searches will include up to 2 Engram memories by default");
        }
        let engram = crate::engram::EngramClient::from_env();

        let twscrape_path = env::var("TWSCRAPE_PATH").unwrap_or_else(|_| "twscrape".to_string());
        let x_min_interval_ms = env::var("X_MIN_INTERVAL_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(8000);

        // Anti-bot session cookies (sogou/weixin SNUID & friends) live in a
        // small local file rather than an env var, so refreshing them takes
        // effect on the next search instead of requiring a restart.
        eprintln!(
            "🍪 cookie cache: {}",
            crate::cookie_cache::cache_path().display()
        );

        Self {
            default_search_engine: default_engine,
            allowed_search_engines: allowed,
            proxy_url,
            use_proxy,
            direct_domains,
            env_cookies,
            twscrape_path,
            x_min_interval_ms,
            fetch_timeout_secs: fetch_timeout,
            deepseek_api_key,
            deepseek_api_mode,
            ieee_api_key,
            search_api_keys,
            cookie_cache_ttl_secs,
            cookie_import_enabled,
            engram_search_enabled,
            engram,
        }
    }

    /// `Cookie:` header value to send to `host`, or an empty string.
    ///
    /// Only entries whose domain is `host` or a parent of it are included, so
    /// a Bilibili session never reaches an unrelated site. Cache entries
    /// override env-seeded ones on name collisions (the cache is the fresher
    /// source: `set_cookies` / `import_cookies` write there).
    pub fn cookies_for(&self, host: &str) -> String {
        crate::cookie_cache::merge_for_host(
            &self.env_cookies,
            &crate::cookie_cache::cookies_for_host(host),
            host,
        )
    }

    /// Whether the DeepSeek engine is available (DEEPSEEK_API_KEY env var set).
    pub fn has_deepseek_key(&self) -> bool { self.deepseek_api_key.is_some() }

    /// Whether the IEEE engine is available (IEEE_API_KEY env var set).
    pub fn has_ieee_key(&self) -> bool { self.ieee_api_key.is_some() }

    /// API key for a keyed web-search engine (`firecrawl`, `octen`,
    /// `parallel`), if configured.
    pub fn api_key(&self, engine: &str) -> Option<&str> {
        self.search_api_keys.get(engine).map(|s| s.as_str())
    }

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
    ///
    /// NOTE: reqwest enables an automatic "system" proxy by default
    /// (`auto_sys_proxy`), which reads the Windows registry
    /// (`HKCU\...\Internet Settings`) and the HTTP_PROXY/HTTPS_PROXY env vars.
    /// That matcher would silently route "direct" requests back through the
    /// proxy whenever the user turns on a global/system proxy. Calling
    /// `.no_proxy()` in the bypass branch disables it, so DIRECT_DOMAINS
    /// actually means "direct".
    pub fn build_reqwest_client_with_proxy(&self, use_proxy: bool) -> reqwest::Result<reqwest::Client> {
        self.client_builder(use_proxy)?.build()
    }

    /// Shared builder so every client gets identical timeout / UA / proxy rules.
    fn client_builder(&self, use_proxy: bool) -> reqwest::Result<reqwest::ClientBuilder> {
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
        } else {
            // Disable reqwest's automatic system/env proxy — otherwise the
            // "bypass" is a no-op when a system proxy (registry) is active.
            builder = builder.no_proxy();
        }

        Ok(builder)
    }

    /// Like [`build_reqwest_client_for`], but with a cookie jar enabled.
    ///
    /// Required for sites that hand out a session cookie we must replay:
    /// Anubis challenge tokens (`src/anubis.rs`) and Sogou's SNUID/SUV.
    pub fn build_reqwest_client_for_with_cookies(&self, domain: &str) -> reqwest::Result<reqwest::Client> {
        self.client_builder(!self.should_bypass_proxy(domain))?
            .cookie_store(true)
            .build()
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

