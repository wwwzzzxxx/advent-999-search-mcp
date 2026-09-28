use async_trait::async_trait;
use scraper::{Html, Selector};
use crate::anubis;
use crate::config::Config;
use crate::filters::SearchOptions;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;
use std::sync::{Mutex, OnceLock};

pub struct StartpageEngine;

const SP_BASE: &str = "https://www.startpage.com";
const SP_SEARCH: &str = "https://www.startpage.com/sp/search";

static SC_CACHE: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);

/// Process-level client with a cookie jar: Startpage hands out an Anubis auth
/// cookie after the PoW is solved, and keeping it avoids re-solving (a
/// difficulty-6 challenge costs millions of hashes) on every search.
static SP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_client(config: &Config) -> Result<&'static reqwest::Client, SearchError> {
    if let Some(c) = SP_CLIENT.get() {
        return Ok(c);
    }
    let built = config
        .build_reqwest_client_for_with_cookies("www.startpage.com")
        .map_err(|e| SearchError::Http(e.to_string()))?;
    let _ = SP_CLIENT.set(built);
    SP_CLIENT.get().ok_or_else(|| SearchError::Http("startpage: client init failed".into()))
}

#[async_trait]
impl SearchEngine for StartpageEngine {
    fn name(&self) -> &'static str { "startpage" }

    async fn search(&self, query: &str, limit: usize, _opts: &SearchOptions, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let client = get_client(config)?;

        let mut all_results = Vec::new();
        let page_size = 10;
        let max_page = std::cmp::max(1, (limit + page_size - 1) / page_size);

        for page in 1..=max_page {
            if all_results.len() >= limit { break; }
            let html = post_search(client, query, page as u32).await?;
            all_results.extend(parse_results(&html));
        }

        all_results.truncate(limit);
        Ok(all_results)
    }
}

/// One search page, transparently solving an Anubis PoW challenge if needed.
async fn post_search(
    client: &reqwest::Client,
    query: &str,
    page: u32,
) -> Result<String, SearchError> {
    let mut html = post_search_once(client, query, page).await?;

    if let Some(challenge) = anubis::detect(&html) {
        eprintln!(
            "🧩 startpage: Anubis challenge detected (difficulty {}), solving…",
            challenge.difficulty
        );
        anubis::pass(client, SP_BASE, &challenge, &format!("{}/", SP_BASE))
            .await
            .map_err(SearchError::Blocked)?;
        // Replay with the auth cookie now stored in the jar.
        html = post_search_once(client, query, page).await?;
    }

    check_captcha(&html)?;
    Ok(html)
}

/// Fetch the search form token and POST the query (plus interstitial follow-up).
async fn post_search_once(
    client: &reqwest::Client,
    query: &str,
    page: u32,
) -> Result<String, SearchError> {
    let sc_code = get_sc_code_async(client).await?;

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

    // Interstitial page: re-submit the payload it embeds.
    if let Some(payload) = extract_interstitial(&html) {
        let form_pairs: Vec<(&str, &str)> = payload.iter()
            .map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let follow = client.post(SP_SEARCH)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Origin", SP_BASE)
            .header("Referer", SP_SEARCH)
            .form(&form_pairs)
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("Startpage interstitial failed: {}", e)))?;
        return follow.text().await
            .map_err(|e| SearchError::Http(format!("Startpage read failed: {}", e)));
    }

    Ok(html)
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
    // Anubis proof-of-work anti-bot (2026): datacenter/proxy IPs receive a JS
    // challenge page instead of results. It contains no "captcha" marker, so
    // without this check the engine silently returns zero results.
    if lower.contains("anubis_challenge")
        || lower.contains("anubis_version")
        || lower.contains("making sure you're not a bot")
    {
        return Err(SearchError::Blocked(
            "Startpage returned an Anubis anti-bot challenge (proxy/datacenter IP blocked); try another proxy node".to_string(),
        ));
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
    let mut seen = std::collections::HashSet::new();

    // Current markup (2026-09, after passing Anubis):
    //   <a class="result-title result-link …" href="URL">
    //     <h2 class="wgl-title …">TITLE</h2>
    //   </a>
    //   <p class="description …">DESC</p>
    // The anchor and the description are SIBLINGS inside a per-result block;
    // the old `.w-gl__result-wrapper` container no longer exists.
    if let Ok(sel) = Selector::parse("a.result-title") {
        for anchor in doc.select(&sel) {
            let url = anchor.value().attr("href").unwrap_or("").trim().to_string();
            if !url.starts_with("http") || !seen.insert(url.clone()) {
                continue;
            }

            let title = anchor_title(&anchor);
            let title = if title.is_empty() {
                extract_hostname(&url).unwrap_or_default()
            } else {
                title
            };

            results.push(SearchResult {
                title,
                url,
                description: find_description(&anchor),
                source: String::new(),
                engine: "startpage".to_string(),
                summary: None,
                engines: Vec::new(),
            });
        }
    }

    // Populate `source` from the URL (the markup hides the hostname in the
    // favicon row, so deriving it from the link is more reliable).
    for r in &mut results {
        r.source = extract_hostname(&r.url).unwrap_or_default();
    }

    // Fallback for older/alternate markup: result containers.
    if results.is_empty() {
        if let Ok(sel) = Selector::parse(".w-gl__result-wrapper, .result-wrapper, .result") {
            for container in doc.select(&sel) {
                let link = container
                    .select(&Selector::parse("a.result-title, a[href]").unwrap())
                    .next();
                let url = link.and_then(|a| a.value().attr("href")).unwrap_or("").to_string();
                if !url.starts_with("http") || !seen.insert(url.clone()) {
                    continue;
                }
                let title = strip_css(link.map(|a| normalize(&a.text().collect::<String>())).unwrap_or_default());
                let title = if title.len() < 3 { extract_hostname(&url).unwrap_or_default() } else { title };
                let description = container
                    .select(&Selector::parse("p.description, .result-description, .desc, p").unwrap())
                    .next()
                    .map(|e| normalize(&e.text().collect::<String>()))
                    .unwrap_or_default();

                results.push(SearchResult {
                    title,
                    url: url.clone(),
                    description,
                    source: extract_hostname(&url).unwrap_or_default(),
                    engine: "startpage".to_string(),
                    summary: None,
                    engines: Vec::new(),
                });
            }
        }
    }

    results
}

/// Extract a result title.
///
/// The title anchor also wraps an inline `<style data-emotion>` block, so a
/// naive `.text()` yields CSS like `.css-i3irj7{line-height:18px;…}`. Prefer
/// the `<h2 class="wgl-title">` child and fall back to CSS-stripped text.
fn anchor_title(anchor: &scraper::ElementRef) -> String {
    if let Ok(sel) = Selector::parse("h2") {
        if let Some(h2) = anchor.select(&sel).next() {
            let t = normalize(&h2.text().collect::<String>());
            if !t.is_empty() {
                return t;
            }
        }
    }
    let raw = strip_css(normalize(&anchor.text().collect::<String>()));
    // Anything still looking like CSS/markup is not a title.
    if raw.starts_with('.') || raw.contains("css-") || raw.starts_with('{') {
        return String::new();
    }
    raw
}

/// `p.description` sits next to the title anchor inside the same result block.
fn find_description(anchor: &scraper::ElementRef) -> String {
    let sel = match Selector::parse("p.description") {
        Ok(s) => s,
        Err(_) => return String::new(),
    };

    // Preferred: a direct child of the anchor's parent (precise, no bleed
    // from neighbouring results).
    if let Some(parent) = anchor.parent() {
        for child in parent.children().filter_map(scraper::ElementRef::wrap) {
            if child.value().name() == "p" && child.select(&sel).next().is_some() {
                return normalize(&child.text().collect::<String>());
            }
            if child.value().name() == "p" {
                return normalize(&child.text().collect::<String>());
            }
        }
    }

    // Fallback: nearest following sibling that is a <p>.
    anchor.next_siblings()
        .filter_map(scraper::ElementRef::wrap)
        .take(6)
        .find(|e| e.value().name() == "p")
        .map(|e| normalize(&e.text().collect::<String>()))
        .unwrap_or_default()
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
