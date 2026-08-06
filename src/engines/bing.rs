use async_trait::async_trait;
use scraper::{Html, Selector};
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct BingEngine;

const BING_URL: &str = "https://cn.bing.com/search";

const BLOCK_KEYWORDS: &[&str] = &[
    "captcha", "verification", "verify you are human",
    "access denied", "blocked", "rate limit", "too many requests",
    "\u{8bf7}\u{9a8c}\u{8bc1}", "\u{9a8c}\u{8bc1}\u{7801}", "\u{4eba}\u{673a}\u{9a8c}\u{8bc1}",
];

#[async_trait]
impl SearchEngine for BingEngine {
    fn name(&self) -> &'static str { "bing" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let client = config.build_reqwest_client()
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let mut all_results = Vec::new();
        let mut page = 0;

        while all_results.len() < limit {
            let url = format!("{}?q={}&setlang=zh-CN&ensearch=0&first={}",
                BING_URL, url_encode(query), 1 + page * 10);

            let resp = client.get(&url)
                .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .header("Cache-Control", "no-cache")
                .send()
                .await
                .map_err(|e| SearchError::Http(format!("Bing request failed: {}", e)))?;

            let html = resp.text().await
                .map_err(|e| SearchError::Http(format!("Bing read body failed: {}", e)))?;

            check_bing_blocked(&html)?;

            let results = parse_bing_results(&html, limit - all_results.len());
            if results.is_empty() { break; }
            all_results.extend(results);
            page += 1;
        }

        all_results.truncate(limit);
        Ok(all_results)
    }
}

fn check_bing_blocked(html: &str) -> Result<(), SearchError> {
    let lower = html.to_lowercase();
    let detected: Vec<&str> = BLOCK_KEYWORDS.iter()
        .filter(|kw| lower.contains(*kw))
        .copied()
        .collect();

    let doc = Html::parse_document(html);
    let sel = Selector::parse("#b_results > li.b_algo, .b_algo").unwrap();
    let has_results = doc.select(&sel).count() > 0;

    if has_results { return Ok(()); }
    if !detected.is_empty() {
        return Err(SearchError::Blocked(format!(
            "Bing returned verification page (keywords: {})", detected.join(", ")
        )));
    }
    Ok(())
}

fn parse_bing_results(html: &str, limit: usize) -> Vec<SearchResult> {
    let doc = Html::parse_document(html);
    let mut results = Vec::new();

    let selectors = [
        "#b_results > li.b_algo",
        "#b_results > li.b_ans",
        ".b_algo",
        ".b_ans",
    ];

    for sel_str in &selectors {
        if results.len() >= limit { break; }
        let Ok(sel) = Selector::parse(sel_str) else { continue; };

        for element in doc.select(&sel) {
            if results.len() >= limit { break; }
            let class = element.value().attr("class").unwrap_or("");
            if class.contains("b_ad") { continue; }

            let (title, url) = element.select(&Selector::parse("h2 a, .b_title a").unwrap())
                .next()
                .map(|a| {
                    let t = a.text().collect::<String>();
                    let u = a.value().attr("href").unwrap_or("");
                    (normalize_text(&t), sanitize_bing_url(u))
                })
                .unwrap_or_default();

            if url.is_empty() { continue; }

            let description = element.select(&Selector::parse(".b_caption p, .b_caption, .b_snippet").unwrap())
                .next()
                .map(|e| normalize_text(&e.text().collect::<String>()))
                .unwrap_or_default();

            let source = element.select(&Selector::parse(".b_tpcn, cite").unwrap())
                .next()
                .map(|e| normalize_text(&e.text().collect::<String>()))
                .or_else(|| extract_hostname(&url))
                .unwrap_or_default();

            if title.is_empty() && description.is_empty() { continue; }

            results.push(SearchResult {
                title: title.chars().take(200).collect(),
                url,
                description: description.chars().take(400).collect(),
                source: source.chars().take(200).collect(),
                engine: "bing".to_string(),
                summary: None,
            });
        }
    }
    results
}

fn sanitize_bing_url(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() { return String::new(); }

    let resolved = if raw.starts_with("//") {
        format!("https:{}", raw)
    } else if raw.starts_with('/') {
        if raw.starts_with("/search") || raw.starts_with("/ck/") || raw.starts_with("/newtab") {
            return String::new();
        }
        format!("https://cn.bing.com{}", raw)
    } else if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else {
        return String::new();
    };

    let lower = resolved.to_lowercase();
    if lower.contains("bing.com/ck/") {
        if let Some(u_start) = lower.find("?u=").or_else(|| lower.find("&u=")) {
            let after_u = &resolved[u_start + 3..];
            let u_value = after_u.split('&').next().unwrap_or("");
            if let Some(decoded) = decode_bing_url_param(u_value) {
                if decoded.starts_with("http://") || decoded.starts_with("https://") {
                    return sanitize_bing_url(&decoded);
                }
            }
        }
        return String::new();
    }
    if lower.contains("bing.com/search") || lower.contains("bing.com/newtab") {
        return String::new();
    }
    resolved
}

fn decode_bing_url_param(input: &str) -> Option<String> {
    let b64 = if let Some(rest) = input.strip_prefix("a1") { rest } else { input };
    let normalized = b64.replace('-', "+").replace('_', "/");
    let padded = match normalized.len() % 4 {
        2 => format!("{}==", normalized),
        3 => format!("{}=", normalized),
        _ => normalized,
    };

    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut decoded = Vec::new();
    let bytes = padded.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i + 2] == b'=' || bytes[i + 3] == b'=' { break; }
        let a = CHARS.iter().position(|&c| c == bytes[i]).unwrap_or(0) as u8;
        let b2 = CHARS.iter().position(|&c| c == bytes[i+1]).unwrap_or(0) as u8;
        let c = if bytes[i+2] != b'=' { CHARS.iter().position(|&c| c == bytes[i+2]).unwrap_or(0) as u8 } else { 0 };
        let d = if bytes[i+3] != b'=' { CHARS.iter().position(|&c| c == bytes[i+3]).unwrap_or(0) as u8 } else { 0 };
        decoded.push((a << 2) | (b2 >> 4));
        if bytes[i+2] != b'=' { decoded.push(((b2 & 0x0f) << 4) | (c >> 2)); }
        if bytes[i+3] != b'=' { decoded.push(((c & 0x03) << 6) | d); }
        i += 4;
    }
    String::from_utf8(decoded).ok()
}

fn normalize_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_hostname(url: &str) -> Option<String> {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .map(|s| s.to_string())
}

fn url_encode(s: &str) -> String {
    let mut result = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => result.push(b as char),
            b' ' => result.push('+'),
            _ => result.push_str(&format!("%{:02X}", b)),
        }
    }
    result
}
