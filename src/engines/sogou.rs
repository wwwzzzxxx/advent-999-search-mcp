use async_trait::async_trait;
use scraper::{Html, Selector};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct SogouEngine;

const SOGOU_URL: &str = "https://www.sogou.com/web";

/// Minimum interval between Sogou requests (community consensus: >= 3s,
/// see 2026 Sogou anti-bot reports; 5s is safer for repeated searches).
const MIN_REQUEST_INTERVAL: Duration = Duration::from_secs(3);

/// Process-level client: keeps the SNUID/SUID cookie session alive ACROSS
/// searches. A fresh client per search looks like "frequent access without
/// cookies" — a high-risk signal that triggers the captcha (zhu327, 2015;
/// WechatSogou caches SNUID the same way).
static SOGOU_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Timestamp of the last request to sogou.com, shared across searches so
/// consecutive tool calls are spaced out.
static LAST_REQUEST: Mutex<Option<Instant>> = Mutex::new(None);

fn get_sogou_client(config: &Config) -> Result<&'static reqwest::Client, SearchError> {
    Ok(SOGOU_CLIENT.get_or_init(|| {
        // Sogou blocks proxy/datacenter IPs with captchas — bypass proxy by
        // default (controlled via DIRECT_DOMAINS).
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
            .cookie_store(true);

        if !config.should_bypass_proxy("www.sogou.com") {
            if let Some(ref url) = config.proxy_url {
                if let Ok(proxy) = reqwest::Proxy::all(url) {
                    builder = builder.proxy(proxy);
                }
            }
        }

        builder.build().expect("failed to build sogou client")
    }))
}

/// Space out requests: sleep until MIN_REQUEST_INTERVAL has elapsed since
/// the last request to sogou.com.
async fn polite_wait() {
    // Compute the wait outside the lock, then release it before awaiting.
    let wait = {
        let last = LAST_REQUEST.lock().unwrap();
        match *last {
            Some(t) => {
                let elapsed = t.elapsed();
                if elapsed < MIN_REQUEST_INTERVAL {
                    Some(MIN_REQUEST_INTERVAL - elapsed)
                } else {
                    None
                }
            }
            None => None,
        }
    };
    if let Some(d) = wait {
        tokio::time::sleep(d).await;
    }
    *LAST_REQUEST.lock().unwrap() = Some(Instant::now());
}

/// Random 1-3s delay between result pages (jaryee's approach).
async fn page_delay() {
    let secs = 1 + (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() % 3)
        .unwrap_or(1) as u64);
    tokio::time::sleep(Duration::from_secs(secs)).await;
}

#[async_trait]
impl SearchEngine for SogouEngine {
    fn name(&self) -> &'static str { "sogou" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let client = get_sogou_client(config)?;

        // First use: warm up the homepage so sogou.com issues session
        // cookies (SUV/SNUID) before we hit the search endpoint.
        if LAST_REQUEST.lock().unwrap().is_none() {
            let _ = client.get("https://www.sogou.com/")
                .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .send()
                .await;
        }

        let headers = build_sogou_headers();

        let mut all_results = Vec::new();
        let mut seen_urls = std::collections::HashSet::new();
        let max_page = std::cmp::max(1, (limit + 9) / 10);

        for page in 1..=max_page {
            if all_results.len() >= limit { break; }

            // Space out requests (across searches AND pages).
            polite_wait().await;

            let url = format!("{}?query={}&page={}&ie=utf8", SOGOU_URL, url_encode(query), page);

            let resp = client.get(&url)
                .headers(headers.clone())
                .send()
                .await
                .map_err(|e| SearchError::Http(format!("Sogou request failed: {}", e)))?;

            let final_url = resp.url().to_string();
            let html = resp.text().await
                .map_err(|e| SearchError::Http(format!("Sogou read body failed: {}", e)))?;

            check_sogou_challenge(&final_url, &html)?;

            let results = parse_sogou_results(&html);
            for r in results {
                if seen_urls.insert(r.url.clone()) {
                    all_results.push(r);
                    if all_results.len() >= limit { break; }
                }
            }

            if page < max_page && all_results.len() < limit {
                page_delay().await;
            }
        }

        Ok(all_results)
    }
}

/// Full browser-like headers (sec-ch-ua family) to reduce Sogou anti-bot
/// false positives — requests missing these are more likely flagged as bots.
fn build_sogou_headers() -> reqwest::header::HeaderMap {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(reqwest::header::ACCEPT,
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7".parse().unwrap());
    h.insert(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8".parse().unwrap());
    h.insert(reqwest::header::ACCEPT_ENCODING, "gzip, deflate, br".parse().unwrap());
    h.insert(reqwest::header::REFERER, "https://www.sogou.com/".parse().unwrap());
    h.insert(reqwest::header::HeaderName::from_static("sec-ch-ua"),
        "\"Chromium\";v=\"136\", \"Google Chrome\";v=\"136\", \"Not?A_Brand\";v=\"99\"".parse().unwrap());
    h.insert(reqwest::header::HeaderName::from_static("sec-ch-ua-mobile"), "?0".parse().unwrap());
    h.insert(reqwest::header::HeaderName::from_static("sec-ch-ua-platform"), "\"Windows\"".parse().unwrap());
    h.insert(reqwest::header::HeaderName::from_static("sec-fetch-site"), "same-origin".parse().unwrap());
    h.insert(reqwest::header::HeaderName::from_static("sec-fetch-mode"), "navigate".parse().unwrap());
    h.insert(reqwest::header::HeaderName::from_static("sec-fetch-user"), "?1".parse().unwrap());
    h.insert(reqwest::header::HeaderName::from_static("sec-fetch-dest"), "document".parse().unwrap());
    h.insert(reqwest::header::UPGRADE_INSECURE_REQUESTS, "1".parse().unwrap());
    h
}

/// Detect Sogou's anti-bot verification page.
///
/// Checks both the final URL (Sogou redirects blocked requests to
/// `/antispider/?...`) and page markers.
fn check_sogou_challenge(final_url: &str, html: &str) -> Result<(), SearchError> {
    let lower = html.to_lowercase();
    if final_url.contains("antispider")
        || lower.contains("antispider")
        || lower.contains("输入验证码")
        || lower.contains("过于频繁")
        || lower.contains("搜狗搜索验证")
        || lower.contains("seccoderight")
        || lower.contains("seccodeinput")
        || lower.contains("请依次点击")
        || lower.contains("验证码")
    {
        return Err(SearchError::Blocked(
            "Sogou returned a verification page; slow down requests or retry later".to_string(),
        ));
    }
    Ok(())
}

fn parse_sogou_results(html: &str) -> Vec<SearchResult> {
    let doc = Html::parse_document(html);
    let mut results = Vec::new();

    let selectors = ["#main .vrwrap", "#main .rb", "#main .result", "#results .vrwrap", ".results .vrwrap"];

    for sel_str in &selectors {
        let Ok(sel) = Selector::parse(sel_str) else { continue; };
        for element in doc.select(&sel) {
            let a_sel = Selector::parse("h3 a[href], h2 a[href], .vr-title a[href], .pt a[href]").unwrap();
            let link = match element.select(&a_sel).next() {
                Some(a) => a,
                None => continue,
            };
            let raw_url = link.value().attr("href").unwrap_or("");
            let url = resolve_sogou_url(raw_url);
            if url.is_empty() { continue; }

            let title = normalize(&link.text().collect::<String>());
            if title.is_empty() { continue; }

            let desc = element.select(&Selector::parse(".str_info, .ft, .text-layout, .fz-mid, p").unwrap())
                .next().map(|e| normalize(&e.text().collect::<String>())).unwrap_or_default();

            let source = element.select(&Selector::parse("cite, .citeurl, .g, .url").unwrap())
                .next().map(|e| normalize(&e.text().collect::<String>()))
                .or_else(|| extract_host(&url)).unwrap_or_default();

            results.push(SearchResult {
                title: title.chars().take(200).collect(),
                url: url.chars().take(500).collect(),
                description: desc.chars().take(400).collect(),
                source: source.chars().take(200).collect(),
                engine: "sogou".to_string(),
            });
        }
    }
    results
}

fn resolve_sogou_url(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() { return String::new(); }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        if let Some(q) = raw.find('?') {
            for pair in raw[q+1..].split('&') {
                if let Some(eq) = pair.find('=') {
                    let (k, v) = (&pair[..eq], &pair[eq+1..]);
                    if (k == "url" || k == "u" || k == "link") && !v.is_empty() {
                        let d = percent_decode(v);
                        if d.starts_with("http://") || d.starts_with("https://") { return d; }
                    }
                }
            }
        }
        return raw.to_string();
    }
    if raw.starts_with("//") { return format!("https:{}", raw); }
    if raw.starts_with('/') { return format!("https://www.sogou.com{}", raw); }
    String::new()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut r = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = hex_val(b[i+1]).and_then(|h| hex_val(b[i+2]).map(|l| (h << 4) | l));
            if let Some(v) = h { r.push(v); i += 3; continue; }
        }
        r.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8(r).unwrap_or_default()
}

fn hex_val(b: u8) -> Option<u8> {
    match b { b'0'..=b'9' => Some(b - b'0'), b'a'..=b'f' => Some(b - b'a' + 10), b'A'..=b'F' => Some(b - b'A' + 10), _ => None }
}

fn normalize(s: &str) -> String { s.split_whitespace().collect::<Vec<_>>().join(" ") }
fn extract_host(url: &str) -> Option<String> { url.trim_start_matches("https://").trim_start_matches("http://").split('/').next().map(|s| s.to_string()) }
fn url_encode(s: &str) -> String {
    let mut r = String::new();
    for b in s.bytes() { match b { b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => r.push(b as char), b' ' => r.push('+'), _ => r.push_str(&format!("%{:02X}", b)), } }
    r
}
