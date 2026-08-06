use async_trait::async_trait;
use scraper::{Html, Selector};
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;
use std::sync::Mutex;

pub struct StartpageEngine;

const SP_BASE: &str = "https://www.startpage.com";
const SP_SEARCH: &str = "https://www.startpage.com/sp/search";

static SC_CACHE: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);

#[async_trait]
impl SearchEngine for StartpageEngine {
    fn name(&self) -> &'static str { "startpage" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let client = config.build_reqwest_client()
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let mut all_results = Vec::new();
        let page_size = 10;
        let max_page = std::cmp::max(1, (limit + page_size - 1) / page_size);

        for page in 1..=max_page {
            if all_results.len() >= limit { break; }

            let sc_code = get_sc_code_async(&client).await?;

            let mut form_data: Vec<(String, String)> = vec![
                ("query".into(), query.into()),
                ("cat".into(), "web".into()),
                ("t".into(), "device".into()),
                ("sc".into(), sc_code),
                ("abp".into(), "1".into()),
                ("abd".into(), "1".into()),
                ("abe".into(), "1".into()),
            ];

            if page > 1 {
                form_data.push(("page".into(), page.to_string()));
                form_data.push(("segment".into(), "startpage.udog".into()));
            }

            let form_refs: Vec<(&str, &str)> = form_data.iter()
                .map(|(k, v)| (k.as_str(), v.as_str())).collect();

            let resp = client.post(SP_SEARCH)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/136.0.0.0")
                .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
                .header("Accept-Language", "en-US,en;q=0.9")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Origin", SP_BASE)
                .header("Referer", format!("{}/", SP_BASE))
                .form(&form_refs)
                .send()
                .await
                .map_err(|e| SearchError::Http(format!("Startpage request failed: {}", e)))?;

            let html = resp.text().await
                .map_err(|e| SearchError::Http(format!("Startpage read failed: {}", e)))?;

            // Handle interstitial page
            let final_html = if let Some(payload) = extract_interstitial(&html) {
                let form_pairs: Vec<(&str, &str)> = payload.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                let follow = client.post(SP_SEARCH)
                    .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/136.0.0.0")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .header("Origin", SP_BASE)
                    .header("Referer", SP_SEARCH)
                    .form(&form_pairs)
                    .send()
                    .await
                    .map_err(|e| SearchError::Http(format!("Startpage interstitial failed: {}", e)))?;
                follow.text().await.unwrap_or(html)
            } else {
                html
            };

            check_captcha(&final_html)?;
            all_results.extend(parse_results(&final_html));
        }

        all_results.truncate(limit);
        Ok(all_results)
    }
}

async fn get_sc_code_async(client: &reqwest::Client) -> Result<String, SearchError> {
    // Check cache
    if let Ok(cache) = SC_CACHE.lock() {
        if let Some((ref code, time)) = *cache {
            if time.elapsed().as_secs() < 1800 {
                return Ok(code.clone());
            }
        }
    }

    let resp = client.get(format!("{}/", SP_BASE))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/136.0.0.0")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .send()
        .await
        .map_err(|e| SearchError::Http(format!("Startpage homepage failed: {}", e)))?;

    let html = resp.text().await
        .map_err(|e| SearchError::Http(format!("Startpage read body failed: {}", e)))?;

    check_captcha(&html)?;

    let doc = Html::parse_document(&html);
    let sel = Selector::parse(r#"form[action="/sp/search"] input[name="sc"]"#)
        .unwrap();
    let code = doc.select(&sel).next()
        .and_then(|el| el.value().attr("value"))
        .map(|s| s.to_string())
        .ok_or_else(|| SearchError::Parse("Failed to extract Startpage search token".to_string()))?;

    if let Ok(mut cache) = SC_CACHE.lock() {
        *cache = Some((code.clone(), std::time::Instant::now()));
    }

    Ok(code)
}

fn check_captcha(html: &str) -> Result<(), SearchError> {
    let lower = html.to_lowercase();
    if lower.contains("/sp/captcha") || lower.contains("human verification") || lower.contains("security check") {
        return Err(SearchError::Blocked("Startpage returned verification page".to_string()));
    }
    Ok(())
}

fn extract_interstitial(html: &str) -> Option<Vec<(String, String)>> {
    let marker = "var data = ";
    let idx = html.find(marker)?;
    let rest = &html[idx + marker.len()..];
    let json_start = rest.find('{')?;

    let mut depth = 0u32;
    let mut end = json_start;
    for (i, ch) in rest[json_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => { depth -= 1; if depth == 0 { end = json_start + i + 1; break; } }
            _ => {}
        }
    }
    if depth != 0 { return None; }

    let json: serde_json::Value = serde_json::from_str(&rest[json_start..end]).ok()?;
    let pairs: Vec<(String, String)> = json.as_object()?
        .iter()
        .filter(|(_, v)| v.is_string())
        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
        .collect();

    if pairs.iter().any(|(k, _)| k == "query") { Some(pairs) } else { None }
}

fn parse_results(html: &str) -> Vec<SearchResult> {
    let doc = Html::parse_document(html);
    let mut results = Vec::new();

    // Use the result container
    if let Ok(sel) = Selector::parse(".w-gl__result-wrapper, .result-wrapper, .result") {
        for container in doc.select(&sel) {
            let link = container.select(&Selector::parse("a.result-title, a[href]").unwrap()).next();
            let url = link.and_then(|a| a.value().attr("href")).unwrap_or("").to_string();
            if url.is_empty() { continue; }

            let title = link.map(|a| normalize(&a.text().collect::<String>())).unwrap_or_default();
            let title = strip_css(title);
            // Fallback to hostname if title is CSS garbage
            let title = if title.is_empty() || title.len() < 3 || title.starts_with('.') || title.starts_with('<') || title.contains("css-") {
                extract_hostname(&url).unwrap_or_default()
            } else {
                title
            };
            if title.is_empty() { continue; }

            let description = container.select(&Selector::parse("p.description, .result-description, .desc").unwrap())
                .next()
                .map(|e| normalize(&e.text().collect::<String>()))
                .unwrap_or_default();

            let source = extract_hostname(&url).unwrap_or_default();
            results.push(SearchResult { title, url, description, source, engine: "startpage".to_string(), summary: None });
        }
    }

    // Fallback: direct link search (filter out CSS garbage)
    if results.is_empty() {
        for link in doc.select(&Selector::parse("a[href]").unwrap()) {
            let url = link.value().attr("href").unwrap_or("").to_string();
            if !url.starts_with("http://") && !url.starts_with("https://") { continue; }
            if url.contains("startpage.com") { continue; }
            let title = normalize(&link.text().collect::<String>());
            let title = strip_css(title);
            // If title still looks like CSS/HTML garbage, use hostname
            let title = if title.is_empty() || title.len() < 5 || title.starts_with('.') || title.starts_with('<') || title.contains("css-") {
                extract_hostname(&url).unwrap_or_default()
            } else {
                title
            };
            if title.is_empty() { continue; }

            results.push(SearchResult {
                title,
                source: extract_hostname(&url).unwrap_or_default(),
                description: String::new(),
                url: url.clone(),
                engine: "startpage".to_string(),
                summary: None,
            });
            if results.len() >= 10 { break; }
        }
    }

    results
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_css(s: String) -> String {
    // Remove HTML tags and CSS blocks
    let mut result = String::new();
    let mut in_tag = false;
    let mut brace_depth = 0u32;
    let mut i = 0;
    let chars: Vec<char> = s.chars().collect();
    while i < chars.len() {
        if chars[i] == '<' {
            in_tag = true;
            i += 1;
            continue;
        }
        if in_tag {
            if chars[i] == '>' { in_tag = false; }
            i += 1;
            continue;
        }
        if chars[i] == '{' { brace_depth += 1; i += 1; continue; }
        if chars[i] == '}' { brace_depth = brace_depth.saturating_sub(1); i += 1; continue; }
        if brace_depth > 0 { i += 1; continue; }
        result.push(chars[i]);
        i += 1;
    }
    result.trim().to_string()
}

fn extract_hostname(url: &str) -> Option<String> {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .map(|s| s.to_string())
}
