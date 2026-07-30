use async_trait::async_trait;
use scraper::{Html, Selector};
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct SogouEngine;

const SOGOU_URL: &str = "https://www.sogou.com/web";

#[async_trait]
impl SearchEngine for SogouEngine {
    fn name(&self) -> &'static str { "sogou" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let client = config.build_reqwest_client()
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let mut all_results = Vec::new();
        let mut seen_urls = std::collections::HashSet::new();
        let max_page = std::cmp::max(1, (limit + 9) / 10);

        for page in 1..=max_page {
            if all_results.len() >= limit { break; }

            let url = format!("{}?query={}&page={}&ie=utf8", SOGOU_URL, url_encode(query), page);

            let resp = client.get(&url)
                .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .header("Referer", "https://www.sogou.com/")
                .send()
                .await
                .map_err(|e| SearchError::Http(format!("Sogou request failed: {}", e)))?;

            let html = resp.text().await
                .map_err(|e| SearchError::Http(format!("Sogou read body failed: {}", e)))?;

            check_sogou_challenge(&html)?;

            let results = parse_sogou_results(&html);
            for r in results {
                if seen_urls.insert(r.url.clone()) {
                    all_results.push(r);
                    if all_results.len() >= limit { break; }
                }
            }
        }

        Ok(all_results)
    }
}

fn check_sogou_challenge(html: &str) -> Result<(), SearchError> {
    let lower = html.to_lowercase();
    if lower.contains("antispider")
        || lower.contains("输入验证码")
        || lower.contains("过于频繁")
        || lower.contains("搜狗搜索验证")
    {
        return Err(SearchError::Blocked("Sogou returned a verification page".to_string()));
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
