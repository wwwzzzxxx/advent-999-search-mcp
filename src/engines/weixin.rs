use async_trait::async_trait;
use scraper::{Html, Selector};
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

/// WeChat official account (公众号) article search via Sogou WeChat search.
///
/// Sogou is the only free public channel to search WeChat articles:
/// `https://weixin.sogou.com/weixin?type=2&query=<keyword>&page=N`
///
/// Search results link to `/link?url=...` redirect pages. We keep the redirect URL
/// as-is (it stays valid long-term); `get_page` resolves the real article URL
/// (see fetch.rs `fetch_sogou_weixin_link`) when fetching.
pub struct WeixinEngine;

const WEIXIN_SEARCH_URL: &str = "https://weixin.sogou.com/weixin";

#[async_trait]
impl SearchEngine for WeixinEngine {
    fn name(&self) -> &'static str { "weixin" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        // Sogou blocks proxy/datacenter IPs with captchas — bypass proxy by default
        // (controlled via DIRECT_DOMAINS). Cookie store keeps SUID/SNUID across
        // pages to look like one browser session.
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
            .cookie_store(true);

        if !config.should_bypass_proxy("weixin.sogou.com") {
            if let Some(ref url) = config.proxy_url {
                if let Ok(proxy) = reqwest::Proxy::all(url) {
                    builder = builder.proxy(proxy);
                }
            }
        }

        let client = builder.build()
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let headers = build_browser_headers();

        let mut all_results = Vec::new();
        let mut seen_urls = std::collections::HashSet::new();
        let max_page = std::cmp::max(1, (limit + 9) / 10);

        for page in 1..=max_page {
            if all_results.len() >= limit { break; }

            let url = format!("{}?type=2&query={}&page={}&ie=utf8",
                WEIXIN_SEARCH_URL, url_encode(query), page);

            let resp = client.get(&url)
                .headers(headers.clone())
                .send()
                .await
                .map_err(|e| SearchError::Http(format!("Weixin search request failed: {}", e)))?;

            let final_url = resp.url().to_string();
            let html = resp.text().await
                .map_err(|e| SearchError::Http(format!("Weixin search read body failed: {}", e)))?;

            check_antispider(&final_url, &html)?;

            let results = parse_weixin_results(&html);
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

/// Full browser-like headers (sec-ch-ua family) to reduce Sogou anti-bot
/// false positives — requests missing these are more likely flagged as bots.
fn build_browser_headers() -> reqwest::header::HeaderMap {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(reqwest::header::ACCEPT,
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7".parse().unwrap());
    h.insert(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8".parse().unwrap());
    h.insert(reqwest::header::ACCEPT_ENCODING, "gzip, deflate, br".parse().unwrap());
    h.insert(reqwest::header::REFERER, "https://weixin.sogou.com/".parse().unwrap());
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
/// `/antispider/?...`) and page markers. Some captcha page variants are
/// JS-heavy and contain no "antispider" string in the HTML, so the URL
/// check is the primary signal.
fn check_antispider(final_url: &str, html: &str) -> Result<(), SearchError> {
    let lower = html.to_lowercase();
    if final_url.contains("antispider")
        || lower.contains("antispider")
        || lower.contains("seccoderight")
        || lower.contains("anti.min.css")
        || lower.contains("seccodeinput")
        || lower.contains("请依次点击")
        || lower.contains("验证码")
    {
        return Err(SearchError::Blocked(
            "Weixin (Sogou) returned a verification page; slow down requests or retry later".to_string(),
        ));
    }
    Ok(())
}

/// Parse Sogou WeChat search result list.
///
/// Result rows: `<li id="sogou_vr_11002601_box_N" ...>`
/// - title: `h3 a` (href = /link?url=...&type=2&query=...&token=...)
/// - snippet: `p.txt-info`
/// - account name: `span.all-time-y2`
/// - publish time: `span.s2 > script: document.write(timeConvert('unix_secs'))`
fn parse_weixin_results(html: &str) -> Vec<SearchResult> {
    let doc = Html::parse_document(html);
    let mut results = Vec::new();

    let Ok(box_sel) = Selector::parse("li[id^=sogou_vr_11002601_box_]") else { return results; };
    let a_sel = Selector::parse("h3 a").unwrap();
    let desc_sel = Selector::parse("p.txt-info").unwrap();
    let src_sel = Selector::parse("span.all-time-y2").unwrap();
    let time_sel = Selector::parse("span.s2").unwrap();

    for item in doc.select(&box_sel) {
        let Some(link) = item.select(&a_sel).next() else { continue; };
        let raw_url = link.value().attr("href").unwrap_or("").trim();
        if raw_url.is_empty() { continue; }

        // Sogou returns a relative /link?url=... path — make it absolute.
        let url = if raw_url.starts_with("http://") || raw_url.starts_with("https://") {
            raw_url.to_string()
        } else if raw_url.starts_with("//") {
            format!("https:{}", raw_url)
        } else {
            format!("https://weixin.sogou.com{}", raw_url)
        };

        let title = normalize(&link.text().collect::<String>());
        if title.is_empty() { continue; }

        let desc = item.select(&desc_sel).next()
            .map(|e| normalize(&e.text().collect::<String>()))
            .unwrap_or_default();

        let source = item.select(&src_sel).next()
            .map(|e| normalize(&e.text().collect::<String>()))
            .unwrap_or_default();

        let ts = item.select(&time_sel).next()
            .map(|e| e.html())
            .and_then(|h| extract_timestamp(&h));

        let description = match ts {
            Some(secs) => {
                let date = unix_ts_to_date(secs);
                if date.is_empty() { desc }
                else if desc.is_empty() { format!("发布于 {}", date) }
                else { format!("{} | 发布于 {}", desc, date) }
            }
            None => desc,
        };

        results.push(SearchResult {
            title: title.chars().take(200).collect(),
            url: url.chars().take(500).collect(),
            description: description.chars().take(400).collect(),
            source: source.chars().take(200).collect(),
            engine: "weixin".to_string(),
        });
    }

    results
}

/// Extract the unix timestamp from `timeConvert('1785581730')`.
fn extract_timestamp(html_fragment: &str) -> Option<i64> {
    let marker = "timeConvert('";
    let idx = html_fragment.find(marker)?;
    let rest = &html_fragment[idx + marker.len()..];
    let end = rest.find('\'')?;
    rest[..end].trim().parse::<i64>().ok()
}

/// Unix timestamp → "YYYY-MM-DD HH:MM" without external deps
/// (Howard Hinnant's civil_from_days algorithm).
fn unix_ts_to_date(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs_of_day = ts.rem_euclid(86_400);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let h = secs_of_day / 3_600;
    let mi = (secs_of_day % 3_600) / 60;

    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, m, d, h, mi)
}

fn normalize(s: &str) -> String { s.split_whitespace().collect::<Vec<_>>().join(" ") }

fn url_encode(s: &str) -> String {
    let mut r = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => r.push(b as char),
            b' ' => r.push('+'),
            _ => r.push_str(&format!("%{:02X}", b)),
        }
    }
    r
}
