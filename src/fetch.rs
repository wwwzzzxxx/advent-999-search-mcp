use crate::config::Config;
use scraper::{Html, Selector, ElementRef, Node};
use std::time::Duration;

/// Tags whose content should be entirely discarded (script, style, noscript, template, svg).
const SKIP_TAGS: &[&str] = &["script", "style", "noscript", "template", "svg", "math",
    "iframe", "object", "embed", "canvas", "applet", "link", "meta", "head"];

/// Tags that represent non-content boilerplate — their children are skipped.
const NON_CONTENT_TAGS: &[&str] = &["nav", "footer", "header", "aside",
    "form", "figure", "figcaption", "details", "dialog", "menu", "menuitem"];

/// Void (self-closing) elements that shouldn't trigger block-level newlines.
const VOID_ELEMENTS: &[&str] = &["br", "hr", "img", "input", "area", "base",
    "col", "embed", "meta", "param", "source", "track", "wbr"];

/// Block-level elements — we add a leading newline before them to separate content.
const BLOCK_TAGS: &[&str] = &["div", "p", "h1", "h2", "h3", "h4", "h5", "h6",
    "ul", "ol", "li", "blockquote", "pre", "table", "section", "article",
    "tr", "hr", "main", "fieldset"];

/// Check if a tag name is in a static list.
fn tag_in_list(tag: &str, list: &[&str]) -> bool {
    list.iter().any(|t| t == &tag)
}

/// Trim trailing whitespace / newlines from the buffer.
fn trim_buf_end(buf: &mut String) {
    while buf.ends_with(' ') || buf.ends_with('\n') {
        buf.pop();
    }
}

/// Convert an ElementRef to markdown, preserving semantic structure.
fn element_to_markdown(el: &ElementRef) -> String {
    let mut result = String::new();
    for child in el.children() {
        append_node_markdown(&child, &mut result);
    }
    result.trim().to_string()
}

/// Append markdown representation of a DOM node to the buffer.
///
/// Handles: headings, paragraphs, links, images, lists, code blocks, blockquotes,
/// strong/em, tables, horizontal rules, line breaks — and crucially, SKIPS
/// script/style/noscript/nav/footer content entirely.
fn append_node_markdown(node: &ego_tree::NodeRef<'_, Node>, buf: &mut String) {
    match node.value() {
        Node::Text(text) => {
            let t = &text.text;
            // Collapse whitespace (HTML rendering rule)
            let collapsed = t.split_whitespace().collect::<Vec<_>>().join(" ");
            if !collapsed.is_empty() {
                buf.push_str(&collapsed);
            }
        }
        Node::Element(elem) => {
            let tag = &*elem.name.local;
            let tag_lower = tag.to_lowercase();

            // ── silent tags: skip entirely ──
            if tag_in_list(&tag_lower, SKIP_TAGS) {
                return;
            }

            // ── non-content tags: skip their children ──
            if tag_in_list(&tag_lower, NON_CONTENT_TAGS) {
                return;
            }

            // ── block newline: insert leading newline for block-level elements ──
            let is_block = tag_in_list(&tag_lower, BLOCK_TAGS);

            if is_block {
                trim_buf_end(buf);
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
            }

            match tag_lower.as_str() {
                // ── headings ──
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    let level = tag_lower[1..].parse::<usize>().unwrap_or(1);
                    let prefix = "#".repeat(level);
                    let start = buf.len();
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    // Extract heading text and prepend marks
                    let heading = buf[start..].trim().to_string();
                    buf.truncate(start);
                    buf.push_str(&prefix);
                    buf.push(' ');
                    buf.push_str(&heading);
                    buf.push('\n');
                }
                // ── links ──
                "a" => {
                    let href = elem.attr("href").unwrap_or("");
                    let start = buf.len();
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    let link_text = buf[start..].trim().to_string();
                    buf.truncate(start);
                    if !href.is_empty() && !link_text.is_empty() {
                        // Resolve relative URLs — simple prefix check
                        let full_href = if href.starts_with('/') && !href.starts_with("//") {
                            format!("{}", href) // kept as-is; caller can resolve
                        } else {
                            href.to_string()
                        };
                        buf.push('[');
                        buf.push_str(&link_text);
                        buf.push_str("](");
                        buf.push_str(&full_href);
                        buf.push(')');
                    } else {
                        buf.push_str(&link_text);
                    }
                }
                // ── images ──
                "img" => {
                    let src = elem.attr("src")
                        .or_else(|| elem.attr("data-src"))
                        .unwrap_or("");
                    let alt = elem.attr("alt").unwrap_or("");
                    if !src.is_empty() {
                        buf.push_str("![");
                        buf.push_str(alt);
                        buf.push_str("](");
                        buf.push_str(src);
                        buf.push(')');
                    } else if !alt.is_empty() {
                        buf.push_str(&alt);
                    }
                }
                // ── line break & horizontal rule ──
                "br" => {
                    trim_buf_end(buf);
                    buf.push('\n');
                }
                "hr" => {
                    buf.push_str("\n---\n");
                }
                // ── pre / code ──
                "pre" => {
                    let start = buf.len();
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    let code_text = buf[start..].trim().to_string();
                    buf.truncate(start);
                    if !code_text.is_empty() {
                        buf.push_str("\n```\n");
                        buf.push_str(&code_text);
                        buf.push_str("\n```\n");
                    }
                }
                "code" => {
                    // Inline code — wrapped in backticks
                    let start = buf.len();
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    let inner = buf[start..].trim().to_string();
                    buf.truncate(start);
                    if !inner.is_empty() {
                        buf.push('`');
                        buf.push_str(&inner);
                        buf.push('`');
                    }
                }
                // ── strong / bold ──
                "strong" | "b" => {
                    buf.push_str("**");
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    trim_buf_end(buf);
                    buf.push_str("**");
                }
                // ── emphasis / italic ──
                "em" | "i" => {
                    buf.push('*');
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    trim_buf_end(buf);
                    buf.push('*');
                }
                // ── lists ──
                "li" => {
                    // Determine if parent is <ol> or <ul>
                    let is_ordered = node.parent()
                        .and_then(|p| p.value().as_element())
                        .map(|e| e.name.local.as_ref() == "ol")
                        .unwrap_or(false);
                    let bullet = if is_ordered { "1. " } else { "- " };
                    buf.push_str(bullet);
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    trim_buf_end(buf);
                    buf.push('\n');
                }
                "ul" | "ol" => {
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    // Ensure trailing newline after list
                    trim_buf_end(buf);
                    if !buf.ends_with('\n') {
                        buf.push('\n');
                    }
                }
                // ── blockquote ──
                "blockquote" => {
                    let start = buf.len();
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    let quote_content = buf[start..].trim().to_string();
                    buf.truncate(start);
                    for line in quote_content.lines() {
                        buf.push_str("> ");
                        buf.push_str(line);
                        buf.push('\n');
                    }
                }
                // ── table (simple) ──
                "table" => {
                    // Collect all text in a simple pipe-separated format
                    let start = buf.len();
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    let table_text = buf[start..].trim().to_string();
                    buf.truncate(start);
                    if !table_text.is_empty() {
                        buf.push_str("\n[table]\n");
                        buf.push_str(&table_text);
                        buf.push_str("\n[/table]\n");
                    }
                }
                "tr" => {
                    buf.push('|');
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    trim_buf_end(buf);
                    buf.push_str(" |\n");
                }
                "td" | "th" => {
                    buf.push_str(" | ");
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    // Don't trim — content stays
                }
                // ── paragraph and generic block containers ──
                "p" => {
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    trim_buf_end(buf);
                    buf.push_str("\n\n");
                }
                "div" | "section" | "article" | "main" | "header" | "footer" | "aside" | "nav" => {
                    // These are treated as structural containers; they already
                    // trigger a block newline above. Recurse children.
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                    trim_buf_end(buf);
                    if !buf.ends_with('\n') {
                        buf.push('\n');
                    }
                }
                // ── label, span, and other inline containers ──
                "label" | "span" | "small" | "sub" | "sup" | "u" | "s" | "del" | "ins"
                | "mark" | "cite" | "q" | "abbr" | "time" | "kbd" | "var" | "samp" => {
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                }
                // ── void elements ──
                _ if tag_in_list(&tag_lower, VOID_ELEMENTS) => {
                    // already handled (br, hr, img above), others ignored
                }
                // ── everything else: recurse children ──
                _ => {
                    for child in node.children() {
                        append_node_markdown(&child, buf);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Minimum content length to consider a direct fetch successful.
const MIN_CONTENT_LEN: usize = 200;
/// Direct fetch per-request timeout.
const DIRECT_FETCH_TIMEOUT_SECS: u64 = 8;
/// Jina Reader per-request timeout.
const JINA_TIMEOUT_SECS: u64 = 12;

/// Domains known to require JS rendering — try Jina Reader directly first.
const JINA_PREFERRED_DOMAINS: &[&str] = &[
    "developer.aliyun.com",
    "cloud.tencent.com",
    "mp.weixin.qq.com",
    "juejin.cn",
    "zhuanlan.zhihu.com",
    // SPA / JS-rendered documentation sites
    "code.visualstudio.com",
    "docs.github.com",
    "react.dev",
    "nextjs.org",
    "vuejs.org",
    "angular.io",
    "svelte.dev",
    "tailwindcss.com",
    "vercel.com",
    "netlify.com",
    "developer.mozilla.org",
    // Other JS-heavy sites
    "medium.com",
    "dev.to",
    "hashnode.dev",
    "notion.so",
    "figma.com",
    "docusaurus.io",
    // Discourse-based forums (have CloudFlare, static HTML needs JS)
    "linux.do",
];

/// Domains that are hopeless even with Jina (strong anti-bot).
/// Note: zhihu.com is handled by signed API, not blocked.
const BLOCKED_DOMAINS: &[&str] = &[
];

/// Result of a fetch operation
pub struct FetchResult {
    pub url: String,
    pub title: String,
    pub content: String,
    pub content_type: String,
    pub site_name: String,
    #[allow(dead_code)]
    pub via: String, // "direct" or "jina"
}

/// Fetch a URL and extract its readable content.
///
/// Strategy:
/// 1. Domain-specific: Discourse API, CSDN direct, Zhihu signed API.
/// 2. If domain is JS-heavy (JINA_PREFERRED_DOMAINS), try Jina Reader directly.
/// 3. Otherwise try direct HTTP fetch first.
/// 4. If direct fetch returns low-quality content (SPA shell), fallback to Jina.
/// 5. If direct fetch fails (403/empty/timeout), fallback to Jina Reader.
/// 6. If domain is blocked, return error immediately.
pub async fn fetch_url(url: &str, config: &Config) -> Result<FetchResult, FetchError> {
    let parsed = url::Url::parse(url).map_err(|e| FetchError::InvalidUrl(e.to_string()))?;
    let host = parsed.host_str().unwrap_or("");

    // ── Discourse forum (e.g. linux.do) → API-first ──
    let is_discourse = host.contains("linux.do");
    if is_discourse {
        match fetch_discourse(url, host, config).await {
            Ok(r) => return Ok(r),
            Err(_) => {
                // API failed (CloudFlare?), fall through to Jina
            }
        }
    }

    // ── CSDN (blog.csdn.net) → direct fetch with CSDN-specific extractor ──
    // CSDN articles are server-rendered static HTML; no JS needed.
    let is_csdn = host.contains("blog.csdn.net");
    if is_csdn {
        match fetch_csdn(url, host, config).await {
            Ok(r) => return Ok(r),
            Err(_) => {
                // Direct fetch failed, fall through to Jina
            }
        }
    }

    // ── Zhihu → signed API ──
    // Zhihu requires x-zse-96 signature computed from d_c0 cookie.
    if host.contains("zhihu.com") {
        match fetch_zhihu(url, host, config).await {
            Ok(r) => return Ok(r),
            Err(e) => {
                // zhuanlan can fallback to Jina (useful when no cookies configured)
                if host.contains("zhuanlan") {
                    if let Ok(r) = fetch_via_jina(url, config).await {
                        return Ok(r);
                    }
                }
                return Err(e);
            }
        }
    }

    // Determine strategy: Jina-first or direct-first
    // Check JINA_PREFERRED first (more specific match) before BLOCKED
    let prefer_jina = JINA_PREFERRED_DOMAINS.iter().any(|d| host.contains(d));

    // Quick check for known blocked domains (skip if jina-preferred)
    if !prefer_jina && BLOCKED_DOMAINS.iter().any(|d| host.contains(d)) {
        if !config.fetch_cookies.is_empty() {
            // User provided cookies, try direct fetch
        } else {
            return Err(FetchError::Blocked(format!(
                "{} is blocked by anti-bot measures. Try setting FETCH_COOKIES with your login cookies.",
                host
            )));
        }
    }

    if prefer_jina {
        // JS-heavy site → try Jina first, fallback to direct
        match fetch_via_jina(url, config).await {
            Ok(r) => return Ok(r),
            Err(_) => fetch_direct(url, config).await,
        }
    } else {
        // Normal site → try direct first, fallback to Jina
        match fetch_direct(url, config).await {
            Ok(r) => return Ok(r),
            Err(FetchError::LowQuality(_, _)) | Err(_) => {
                // Low quality or other error → try Jina as fallback
                match fetch_via_jina(url, config).await {
                    Ok(r) => Ok(r),
                    Err(jina_err) => Err(jina_err),
                }
            }
        }
    }
}

// ─── Discourse API Fetcher ───
//
// Discourse (e.g. linux.do) exposes a full JSON API — no JS, no anti-bot, no cookies needed.
// - Topic: https://{host}/t/{slug}/{id}.json or /t/{id}.json
// - Homepage: https://{host}/latest.json

/// Fetch content from a Discourse forum via its native JSON API.
async fn fetch_discourse(url: &str, host: &str, config: &Config) -> Result<FetchResult, FetchError> {
    let parsed = url::Url::parse(url).map_err(|e| FetchError::InvalidUrl(e.to_string()))?;
    let site_name = detect_site(host);

    // Build the API URL
    let api_url = build_discourse_api_url(&parsed, host);

    let client = config.build_reqwest_client()
        .map_err(|e| FetchError::Config(e.to_string()))?;

    let resp = client.get(&api_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() { FetchError::Timeout }
            else { FetchError::Http(format!("Discourse API request failed: {}", e)) }
        })?;

    let status = resp.status();
    if !status.is_success() {
        return Err(FetchError::HttpStatus(status.as_u16(), status.as_str().to_string()));
    }

    let bytes = resp.bytes().await
        .map_err(|e| FetchError::Http(format!("Failed to read Discourse API response: {}", e)))?;

    let json: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Parse(format!("Discourse API JSON: {}", e)))?;

    if let Some(title) = json.get("title").and_then(|v| v.as_str()) {
        // Single topic — extract posts as markdown
        let md = discourse_topic_to_markdown(&json);
        let content = if md.trim().len() >= MIN_CONTENT_LEN { md } else {
            return Err(FetchError::EmptyContent);
        };
        Ok(FetchResult {
            url: url.to_string(),
            title: title.to_string(),
            content,
            content_type: "text/markdown".into(),
            site_name,
            via: "discourse-api".into(),
        })
    } else {
        // Topic list (homepage / latest / category)
        let md = discourse_topic_list_to_markdown(&json);
        let content = if md.trim().len() >= MIN_CONTENT_LEN { md } else {
            return Err(FetchError::EmptyContent);
        };
        Ok(FetchResult {
            url: url.to_string(),
            title: format!("{} - Latest", site_name),
            content,
            content_type: "text/markdown".into(),
            site_name,
            via: "discourse-api".into(),
        })
    }
}

/// Build the correct Discourse JSON API URL from the page URL.
fn build_discourse_api_url(parsed: &url::Url, host: &str) -> String {
    let path = parsed.path().trim_end_matches('/');

    // Extract topic ID from path patterns:
    //   /t/topic/847468  or  /t/slug/847468
    //   /t/847468
    if path.contains("/t/") {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        // Find the numeric topic ID — it's the last segment that is a number
        for seg in segments.iter().rev() {
            if seg.chars().all(|c| c.is_ascii_digit()) {
                return format!("https://{}/t/{}.json", host, seg);
            }
        }
    }

    // Fallback: latest.json
    format!("https://{}/latest.json", host)
}

/// Convert a single Discourse topic JSON into markdown.
fn discourse_topic_to_markdown(json: &serde_json::Value) -> String {
    let title = json.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let mut md = format!("# {}\n\n", title);

    if let Some(posts) = json.get("post_stream").and_then(|v| v.get("posts")).and_then(|v| v.as_array()) {
        for post in posts.iter().take(20) { // limit: first 20 posts
            let username = post.get("username").and_then(|v| v.as_str()).unwrap_or("");
            let name = post.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let display_name = if !name.is_empty() && name != username {
                format!("{} (@{})", name, username)
            } else {
                format!("@{}", username)
            };

            let created = post.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
            let created_short = &created[..created.len().min(10)]; // YYYY-MM-DD

            let cooked = post.get("cooked").and_then(|v| v.as_str()).unwrap_or("");
            let body = cooked_html_to_text(cooked);

            if body.trim().is_empty() { continue; }

            md.push_str(&format!("---\n**{}** · {}\n\n{}\n\n", display_name, created_short, body));
        }
    }

    md
}

/// Convert a Discourse topic list JSON (latest/category) into markdown.
fn discourse_topic_list_to_markdown(json: &serde_json::Value) -> String {
    let mut md = String::from("# Latest Topics\n\n");

    if let Some(topic_list) = json.get("topic_list") {
        if let Some(topics) = topic_list.get("topics").and_then(|v| v.as_array()) {
            for topic in topics.iter().take(30) {
                let id = topic.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                let title = topic.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let slug = topic.get("slug").and_then(|v| v.as_str()).unwrap_or("");
                let excerpt = topic.get("excerpt").and_then(|v| v.as_str()).unwrap_or("");
                let posts_count = topic.get("posts_count").and_then(|v| v.as_u64()).unwrap_or(0);
                let reply_count = posts_count.saturating_sub(1);
                let views = topic.get("views").and_then(|v| v.as_u64()).unwrap_or(0);

                let url = format!("https://linux.do/t/{}/{}", slug, id);

                md.push_str(&format!("- [{}]({})  \n", title, url));
                if !excerpt.is_empty() {
                    let clean_excerpt = excerpt.replace('\n', " ");
                    md.push_str(&format!("  {}  \n", clean_excerpt));
                }
                md.push_str(&format!("  💬 {} · 👁 {}\n\n", reply_count, views));
            }
        }
    }

    md
}

/// Convert Discourse's `cooked` HTML to readable plain text.
fn cooked_html_to_text(html: &str) -> String {
    // Wrap in a document so scraper can parse it properly
    let full_html = format!("<html><body>{}</body></html>", html);
    let doc = Html::parse_document(&full_html);
    let content = extract_general_content(&doc);
    collapse_blank_lines(&content)
}

/// Collapse 3+ consecutive blank lines to 2, and trim.
fn collapse_blank_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut result = String::with_capacity(text.len());
    let mut blank_streak = 0;
    for line in &lines {
        if line.trim().is_empty() {
            blank_streak += 1;
            if blank_streak <= 2 {
                result.push('\n');
            }
        } else {
            blank_streak = 0;
            result.push_str(line);
            result.push('\n');
        }
    }
    result.trim().to_string()
}

/// Direct HTTP fetch with browser-like headers and HTML content extraction.

// ─── Zhihu Signed API Fetcher ───
//
// Zhihu requires x-zse-96 header computed from d_c0 cookie.
// Uses the zhihu_sign crate (pure Rust SM4 implementation).
// Requires d_c0 (and optionally z_c0) in FETCH_COOKIES.

/// Extract a specific cookie value from a semicolon-separated cookie string.
fn extract_cookie_value(cookies: &str, name: &str) -> String {
    let target = format!("{}=", name);
    for part in cookies.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&target) {
            return value.trim().to_string();
        }
    }
    String::new()
}

/// Convert a Zhihu page URL to its JSON API URL.
fn zhihu_page_to_api_url(parsed: &url::Url) -> Result<String, FetchError> {
    let path = parsed.path().trim_end_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    match segments.as_slice() {
        ["p", id] => {
            // zhuanlan.zhihu.com/p/{id} → /api/v4/articles/{id}
            Ok(format!("https://www.zhihu.com/api/v4/articles/{}", id))
        }
        ["question", _qid, "answer", aid] => {
            // www.zhihu.com/question/{qid}/answer/{aid} → /api/v4/answers/{aid}
            Ok(format!("https://www.zhihu.com/api/v4/answers/{}", aid))
        }
        ["question", qid] => {
            // www.zhihu.com/question/{qid} → /api/v4/questions/{qid}
            Ok(format!("https://www.zhihu.com/api/v4/questions/{}", qid))
        }
        ["people", url_token] => {
            // www.zhihu.com/people/{url_token} → /api/v4/members/{url_token}
            Ok(format!("https://www.zhihu.com/api/v4/members/{}", url_token))
        }
        _ => Err(FetchError::InvalidUrl(format!(
            "Unrecognized Zhihu URL pattern: {}", parsed.path()
        ))),
    }
}

/// Convert Zhihu API JSON response to readable markdown.
fn zhihu_json_to_markdown(json: &serde_json::Value) -> String {
    let mut md = String::new();

    // Title
    if let Some(title) = json.get("title").and_then(|v| v.as_str()) {
        md.push_str(&format!("# {}\n\n", title));
    }

    // Author info
    if let Some(author) = json.get("author") {
        let name = author.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if !name.is_empty() {
            md.push_str(&format!("**作者：{}**\n\n", name));
        }
    }

    // Question context (for answers)
    if let Some(question) = json.get("question") {
        if let Some(q_title) = question.get("title").and_then(|v| v.as_str()) {
            md.push_str(&format!("> 问题：{}\n\n", q_title));
        }
    }

    // Content (HTML to markdown via existing infra)
    if let Some(content) = json.get("content").and_then(|v| v.as_str()) {
        if !content.trim().is_empty() {
            let text_md = cooked_html_to_text(content);
            md.push_str(&text_md);
        }
    }

    // Fallback: excerpt / description
    if md.trim().is_empty() {
        if let Some(excerpt) = json.get("excerpt").and_then(|v| v.as_str()) {
            md.push_str(excerpt);
        }
    }

    md
}

/// Fetch Zhihu content via signed API requests.
///
/// Uses `zhihu_sign::sign_zhihu_request` to compute the x-zse-96 header,
/// then calls Zhihu's JSON API. Requires `d_c0` in FETCH_COOKIES.
async fn fetch_zhihu(url: &str, host: &str, config: &Config) -> Result<FetchResult, FetchError> {
    let site_name = detect_site(host);
    let parsed = url::Url::parse(url).map_err(|e| FetchError::InvalidUrl(e.to_string()))?;

    // Convert page URL to API URL
    let api_url = zhihu_page_to_api_url(&parsed)?;

    // Extract d_c0 from cookies
    let d_c0 = extract_cookie_value(&config.fetch_cookies, "d_c0");
    if d_c0.is_empty() {
        return Err(FetchError::Blocked(
            "Zhihu requires d_c0 cookie. Set FETCH_COOKIES env var with d_c0=... \n\
             Get it from browser DevTools → Application → Cookies → zhihu.com".into()
        ));
    }

    // Compute signed headers using zhihu_sign crate
    let signed_headers = zhihu_sign::sign_zhihu_request(&api_url, &d_c0, None);

    // Build HTTP client
    let client = build_fetch_client(config, host)?;
    let mut request = client.get(&api_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, build_user_agent(host))
        .header(reqwest::header::REFERER, "https://www.zhihu.com/")
        .timeout(Duration::from_secs(DIRECT_FETCH_TIMEOUT_SECS));

    // Add signed headers (x-zse-93, x-zse-96, x-requested-with)
    for (name, value) in &signed_headers {
        if let Ok(hv) = reqwest::header::HeaderValue::from_str(value) {
            if let Ok(hname) = reqwest::header::HeaderName::from_bytes(name.as_bytes()) {
                request = request.header(hname, hv);
            }
        }
    }

    // Add cookies
    if !config.fetch_cookies.is_empty() {
        if let Ok(cookie_header) = reqwest::header::HeaderValue::from_str(&config.fetch_cookies) {
            request = request.header(reqwest::header::COOKIE, cookie_header);
        }
    }

    // Send request
    let resp = request.send().await.map_err(|e| {
        if e.is_timeout() { FetchError::Timeout }
        else if e.is_connect() { FetchError::Network(format!("Connection failed: {}", e)) }
        else { FetchError::Http(format!("Zhihu API request failed: {}", e)) }
    })?;

    let status = resp.status();
    if status == 403 {
        return Err(FetchError::Blocked(
            "Zhihu API returned 403. d_c0 may be expired or you need z_c0 login cookie. \
             Refresh cookies in browser and update your FETCH_COOKIES env var.".into()
        ));
    }
    if !status.is_success() {
        return Err(FetchError::HttpStatus(status.as_u16(), status.as_str().to_string()));
    }

    let bytes = resp.bytes().await
        .map_err(|e| FetchError::Http(format!("Failed to read Zhihu API response: {}", e)))?;

    let json: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Parse(format!("Zhihu API JSON: {}", e)))?;

    // Extract title from JSON
    let title = json.get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            json.get("question")
                .and_then(|q| q.get("title"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    // Convert JSON to markdown
    let content = zhihu_json_to_markdown(&json);
    if content.trim().len() < MIN_CONTENT_LEN {
        return Err(FetchError::EmptyContent);
    }

    Ok(FetchResult {
        url: url.to_string(),
        title,
        content,
        content_type: "text/markdown".into(),
        site_name,
        via: "zhihu-api".into(),
    })
}

/// CSDN blog article fetch — server-rendered static HTML, no JS required.
/// Direct fetch is faster than Jina and our extract_csdn_content has tuned selectors.
async fn fetch_csdn(url: &str, host: &str, config: &Config) -> Result<FetchResult, FetchError> {
    let _parsed = url::Url::parse(url).map_err(|e| FetchError::InvalidUrl(e.to_string()))?;
    let site_name = detect_site(host);

    let client = build_fetch_client(config, host)?;

    let mut request = client.get(url)
        .headers(build_headers(host))
        .timeout(Duration::from_secs(DIRECT_FETCH_TIMEOUT_SECS));

    if !config.fetch_cookies.is_empty() {
        if let Ok(cookie_header) = reqwest::header::HeaderValue::from_str(&config.fetch_cookies) {
            request = request.header(reqwest::header::COOKIE, cookie_header);
        }
    }

    let resp = request.send().await.map_err(|e| {
        if e.is_timeout() { FetchError::Timeout }
        else if e.is_connect() { FetchError::Network(format!("Connection failed: {}", e)) }
        else { FetchError::Http(format!("Request failed: {}", e)) }
    })?;

    let status = resp.status();
    if !status.is_success() {
        return Err(FetchError::HttpStatus(status.as_u16(), status.as_str().to_string()));
    }

    let content_type = resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let bytes = resp.bytes().await
        .map_err(|e| FetchError::Http(format!("Failed to read body: {}", e)))?;

    let html_body = decode_html(&bytes, &content_type);
    let title = extract_title(&html_body);
    let doc = Html::parse_document(&html_body);
    let content = extract_csdn_content(&doc);

    if content.trim().len() < MIN_CONTENT_LEN {
        return Err(FetchError::EmptyContent);
    }

    let content = post_process_content(&content, host);

    Ok(FetchResult { url: url.to_string(), title, content, content_type, site_name, via: "csdn-direct".into() })
}

async fn fetch_direct(url: &str, config: &Config) -> Result<FetchResult, FetchError> {
    let parsed = url::Url::parse(url).map_err(|e| FetchError::InvalidUrl(e.to_string()))?;
    let domain = parsed.host_str().unwrap_or("unknown").to_string();
    let site_name = detect_site(&domain);

    let client = build_fetch_client(config, &domain)?;

    let mut request = client.get(url)
        .headers(build_headers(&domain))
        .timeout(Duration::from_secs(DIRECT_FETCH_TIMEOUT_SECS));

    if !config.fetch_cookies.is_empty() {
        if let Ok(cookie_header) = reqwest::header::HeaderValue::from_str(&config.fetch_cookies) {
            request = request.header(reqwest::header::COOKIE, cookie_header);
        }
    }

    let resp = request.send().await.map_err(|e| {
        if e.is_timeout() { FetchError::Timeout }
        else if e.is_connect() { FetchError::Network(format!("Connection failed: {}", e)) }
        else { FetchError::Http(format!("Request failed: {}", e)) }
    })?;

    let status = resp.status();
    if !status.is_success() {
        return Err(FetchError::HttpStatus(status.as_u16(), status.as_str().to_string()));
    }

    let content_type = resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let bytes = resp.bytes().await
        .map_err(|e| FetchError::Http(format!("Failed to read body: {}", e)))?;

    let html = decode_html(&bytes, &content_type);

    // SPA shell detection: if raw HTML looks JS-rendered, skip extraction
    // and fallback to Jina Reader which can execute JavaScript.
    if detect_spa_shell(&html) {
        return Err(FetchError::LowQuality(
            "Page appears to be a JS-rendered SPA shell; try Jina reader".into(),
            String::new(),
        ));
    }

    let title = extract_title(&html);
    let content = extract_readable_content(&html, &domain);

    if content.trim().len() < MIN_CONTENT_LEN {
        return Err(FetchError::EmptyContent);
    }

    // Quality check: if content looks like an SPA shell (nav, sidebar, no body text),
    // return LowQuality so the caller can fallback to Jina.
    let quality = content_quality_score(&content);
    if quality < 0.15 {
        return Err(FetchError::LowQuality(
            format!("Quality score {:.3} below threshold", quality),
            content,
        ));
    }

    // Clean nav/sidebar junk from extracted content
    let content = post_process_content(&content, &domain);

    Ok(FetchResult { url: url.to_string(), title, content, content_type, site_name, via: "direct".into() })
}

/// Fetch via Jina Reader API — handles JS rendering server-side.
/// Returns content as markdown. Zero extra memory, just an HTTP request.
async fn fetch_via_jina(url: &str, config: &Config) -> Result<FetchResult, FetchError> {
    let parsed = url::Url::parse(url).map_err(|e| FetchError::InvalidUrl(e.to_string()))?;
    let domain = parsed.host_str().unwrap_or("unknown").to_string();
    let site_name = detect_site(&domain);

    let client = config.build_reqwest_client()
        .map_err(|e| FetchError::Config(e.to_string()))?;

    let jina_url = format!("https://r.jina.ai/{}", url);

    let mut req = client.get(&jina_url)
        .header(reqwest::header::ACCEPT, "text/markdown")
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (compatible; omni-search/1.0)")
        .header("X-Timeout", "30")
        .header("X-Return-Format", "markdown");

    // Forward user cookies to Jina so authenticated content is accessible
    if !config.fetch_cookies.is_empty() {
        req = req.header("X-Set-Cookie", &config.fetch_cookies);
    }

    let resp = req
        .timeout(Duration::from_secs(JINA_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() { FetchError::Timeout }
            else { FetchError::Http(format!("Jina request failed: {}", e)) }
        })?;

    let status = resp.status();
    if !status.is_success() {
        return Err(FetchError::HttpStatus(status.as_u16(), format!("Jina returned {}", status)));
    }

    let bytes = resp.bytes().await
        .map_err(|e| FetchError::Http(format!("Failed to read Jina response: {}", e)))?;
    let text = String::from_utf8_lossy(&bytes).to_string();

    // Parse Jina response format:
    // Title: xxx
    // URL Source: xxx
    // Markdown Content:
    // ...
    let title = text.lines()
        .find(|l| l.starts_with("Title: "))
        .map(|l| l.trim_start_matches("Title: ").trim().to_string())
        .unwrap_or_default();

    // Extract markdown content (after "Markdown Content:" or "Markdown Content:\n")
    let content = if let Some(idx) = text.find("Markdown Content:\n") {
        let after = &text[idx + "Markdown Content:\n".len()..];
        after.trim().to_string()
    } else if let Some(idx) = text.find("Markdown Content:") {
        let after = &text[idx + "Markdown Content:".len()..];
        after.trim().to_string()
    } else {
        text.clone()
    };

    // Check for error indicators in Jina response
    // Only reject if the ENTIRE content is just a warning/error message.
    // Many pages return a short warning line followed by real content.
    let trimmed_content = content.trim();
    if trimmed_content.len() < MIN_CONTENT_LEN {
        // Little to no content — check if it's an error response
        if trimmed_content.contains("Target URL returned error")
            || trimmed_content.contains("Warning: This page maybe")
            || trimmed_content.is_empty()
        {
            return Err(FetchError::EmptyContent);
        }
    }

    // Clean nav/sidebar junk — Jina often includes full page nav
    let content = post_process_content(&content, &domain);

    Ok(FetchResult {
        url: url.to_string(),
        title,
        content,
        content_type: "text/markdown".into(),
        site_name,
        via: "jina".into(),
    })
}

/// Build a reqwest client specifically for fetching (longer timeout, redirect handling)
fn build_fetch_client(config: &Config, domain: &str) -> Result<reqwest::Client, FetchError> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.fetch_timeout_secs))
        .connect_timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(0) // don't reuse connections for fetch
        .redirect(reqwest::redirect::Policy::limited(10))
        .danger_accept_invalid_certs(false)
        .user_agent(build_user_agent(domain));

    // Enable cookies for sites that need session continuity
    builder = builder.cookie_store(true);

    // Gzip/deflate/brotli are auto-enabled by default in reqwest

    if config.use_proxy {
        if let Some(ref url) = config.proxy_url {
            let proxy = reqwest::Proxy::all(url).map_err(|e| {
                FetchError::Config(format!("Invalid proxy URL: {}", e))
            })?;
            builder = builder.proxy(proxy);
        }
    }

    builder.build().map_err(|e| FetchError::Config(e.to_string()))
}

/// Build domain-appropriate headers to bypass anti-bot measures
fn build_headers(domain: &str) -> reqwest::header::HeaderMap {
    // NOTE: Cookie header is not set here because reqwest manages cookies
    // via its cookie_store. For custom cookies, pass them via FETCH_COOKIES env var
    // or set the Cookie header on the request directly.
    let mut headers = reqwest::header::HeaderMap::new();

    // Common headers for all requests
    headers.insert(reqwest::header::ACCEPT,
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8".parse().unwrap());
    headers.insert(reqwest::header::ACCEPT_LANGUAGE,
        "zh-CN,zh;q=0.9,en;q=0.8".parse().unwrap());
    headers.insert(reqwest::header::CACHE_CONTROL,
        "no-cache".parse().unwrap());
    headers.insert(reqwest::header::PRAGMA,
        "no-cache".parse().unwrap());
    headers.insert(reqwest::header::HeaderName::from_static("sec-ch-ua"),
        "\"Google Chrome\";v=\"136\", \"Chromium\";v=\"136\", \"Not?A_Brand\";v=\"99\"".parse().unwrap());
    headers.insert(reqwest::header::HeaderName::from_static("sec-ch-ua-mobile"),
        "?0".parse().unwrap());
    headers.insert(reqwest::header::HeaderName::from_static("sec-ch-ua-platform"),
        "\"Windows\"".parse().unwrap());
    headers.insert(reqwest::header::UPGRADE_INSECURE_REQUESTS,
        "1".parse().unwrap());
    headers.insert(reqwest::header::HeaderName::from_static("sec-fetch-site"),
        "none".parse().unwrap());
    headers.insert(reqwest::header::HeaderName::from_static("sec-fetch-mode"),
        "navigate".parse().unwrap());
    headers.insert(reqwest::header::HeaderName::from_static("sec-fetch-user"),
        "?1".parse().unwrap());
    headers.insert(reqwest::header::HeaderName::from_static("sec-fetch-dest"),
        "document".parse().unwrap());
    match domain {
        d if d.contains("zhihu") => {
            headers.insert(reqwest::header::REFERER,
                "https://www.zhihu.com/".parse().unwrap());
            headers.insert(reqwest::header::HeaderName::from_static("x-requested-with"), "fetch".parse().unwrap());
        }
        d if d.contains("csdn") => {
            headers.insert(reqwest::header::REFERER,
                "https://blog.csdn.net/".parse().unwrap());
        }
        d if d.contains("baijiahao") | d.contains("baidu") => {
            headers.insert(reqwest::header::REFERER,
                "https://baijiahao.baidu.com/".parse().unwrap());
        }
        d if d.contains("juejin") => {
            headers.insert(reqwest::header::REFERER,
                "https://juejin.cn/".parse().unwrap());
        }
        d if d.contains("bilibili") => {
            headers.insert(reqwest::header::REFERER,
                "https://www.bilibili.com/".parse().unwrap());
        }
        d if d.contains("xiaohongshu") | d.contains("xhs") => {
            headers.insert(reqwest::header::REFERER,
                "https://www.xiaohongshu.com/".parse().unwrap());
        }
        _ => {
            headers.insert(reqwest::header::REFERER,
                "https://www.google.com/".parse().unwrap());
        }
    }

    headers
}

/// Build a realistic browser User-Agent for the target domain
fn build_user_agent(domain: &str) -> String {
    match domain {
        d if d.contains("zhihu") => {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".to_string()
        }
        d if d.contains("csdn") => {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0".to_string()
        }
        d if d.contains("baidu") | d.contains("baijiahao") => {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".to_string()
        }
        _ => {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".to_string()
        }
    }
}

/// Detect the site name from domain
fn detect_site(domain: &str) -> String {
    if domain.contains("zhihu") { "知乎".to_string() }
    else if domain.contains("csdn") { "CSDN".to_string() }
    else if domain.contains("juejin") { "掘金".to_string() }
    else if domain.contains("bilibili") { "B站".to_string() }
    else if domain.contains("xiaohongshu") { "小红书".to_string() }
    else if domain.contains("baijiahao") { "百家号".to_string() }
    else if domain.contains("baidu") { "百度".to_string() }
    else if domain.contains("github") { "GitHub".to_string() }
    else if domain.contains("zhuanlan") { "知乎专栏".to_string() }
    else if domain.contains("linuxdo") { "LinuxDo".to_string() }
    else { domain.to_string() }
}

/// Decode HTML bytes to string with charset detection
fn decode_html(bytes: &[u8], content_type: &str) -> String {
    // Try to find charset from Content-Type header
    let charset = if let Some(idx) = content_type.to_lowercase().find("charset=") {
        let rest = &content_type[idx + 8..];
        let charset = rest.split(';').next().unwrap_or("").trim().to_lowercase();
        if charset == "utf-8" || charset == "utf8" { "utf-8" }
        else if charset == "gbk" || charset == "gb2312" || charset == "gb18030" { "gbk" }
        else if charset == "iso-8859-1" { "iso-8859-1" }
        else { "utf-8" }
    } else {
        // Try to detect from HTML content
        let html_start = std::str::from_utf8(bytes).unwrap_or("");
        if html_start.contains("charset=gbk") || html_start.contains("charset=gb2312") || html_start.contains("charset=gb18030") {
            "gbk"
        } else {
            "utf-8"
        }
    };

    match charset {
        "gbk" | "gb2312" | "gb18030" => {
            use encoding_rs::GBK;
            let (decoded, _, _) = GBK.decode(bytes);
            decoded.to_string()
        }
        _ => {
            // Try UTF-8 first, fallback to lossy
            String::from_utf8_lossy(bytes).to_string()
        }
    }
}

/// Extract page title from HTML
fn extract_title(html: &str) -> String {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("title").unwrap();
    if let Some(el) = doc.select(&sel).next() {
        let text = el.text().collect::<Vec<_>>().join("");
        let text = text.trim().to_string();
        if !text.is_empty() { return text; }
    }
    // Fallback: try og:title
    let sel = Selector::parse("meta[property='og:title']").unwrap();
    if let Some(el) = doc.select(&sel).next() {
        if let Some(content) = el.value().attr("content") {
            return content.to_string();
        }
    }
    String::new()
}

/// Extract readable content from HTML, tailored for Chinese sites
fn extract_readable_content(html: &str, domain: &str) -> String {
    let doc = Html::parse_document(html);

    // Domain-specific content extraction strategies
    match domain {
        d if d.contains("zhihu") => extract_zhihu_content(&doc),
        d if d.contains("csdn") => extract_csdn_content(&doc),
        d if d.contains("juejin") => extract_juejin_content(&doc),
        d if d.contains("bilibili") => extract_bilibili_content(&doc),
        d if d.contains("baijiahao") => extract_baijiahao_content(&doc),
        _ => extract_general_content(&doc),
    }
}

/// Extract Zhihu article content
fn extract_zhihu_content(doc: &Html) -> String {
    // Try multiple selectors for Zhihu content
    let selectors = [
        "article.RichText",                          // 专栏文章
        ".Post-RichText",                             // 专栏
        ".RichText ztext",                            // 知乎回答
        ".AnswerCard .RichText",                      // 回答
        ".ContentItem .RichText",                     // 通用
        "article .RichText",
        "meta[itemprop='description']",
    ];

    for selector_str in &selectors {
        if selector_str.starts_with("meta") {
            // Meta description — extract attribute directly
            if let Ok(sel) = Selector::parse(selector_str) {
                if let Some(el) = doc.select(&sel).next() {
                    if let Some(content) = el.value().attr("content") {
                        return content.to_string();
                    }
                }
            }
        } else if let Ok(sel) = Selector::parse(selector_str) {
            let texts: Vec<String> = doc.select(&sel)
                .map(|el| element_to_markdown(&el))
                .filter(|t| !t.trim().is_empty())
                .collect();
            if !texts.is_empty() {
                let joined = texts.join("\n\n");
                if joined.trim().len() > 50 {
                    return joined;
                }
            }
        }
    }

    // Fallback: try to find article content
    extract_general_content(doc)
}

/// Extract CSDN blog content — handles both legacy and new page layouts
fn extract_csdn_content(doc: &Html) -> String {
    // CSDN new version: #content_views > article (with baidu-level JSON-LD)
    // CSDN old version: #article_content or .markdown_views
    let selectors = [
        "#content_views",         // CSDN 2024+ new layout    
        "article.baidu_pl",       // CSDN Baidu partnership layout
        "#article_content",        // CSDN legacy
        "article .markdown_views",
        ".markdown_views",
        ".article_content",
        "article",
    ];

    for selector_str in &selectors {
        if let Ok(sel) = Selector::parse(selector_str) {
            let texts: Vec<String> = doc.select(&sel)
                .map(|el| element_to_markdown(&el))
                .filter(|t| !t.trim().is_empty())
                .collect();
            if !texts.is_empty() {
                let joined = texts.join("\n\n");
                if joined.trim().len() > 50 {
                    return joined;
                }
            }
        }
    }

    extract_general_content(doc)
}

/// Extract Juejin article content
fn extract_juejin_content(doc: &Html) -> String {
    let selectors = [
        ".article-content",
        ".markdown-body",
        "article",
        ".content-container",
    ];

    for selector_str in &selectors {
        if let Ok(sel) = Selector::parse(selector_str) {
            let texts: Vec<String> = doc.select(&sel)
                .map(|el| element_to_markdown(&el))
                .filter(|t| !t.trim().is_empty())
                .collect();
            if !texts.is_empty() {
                let joined = texts.join("\n\n");
                if joined.trim().len() > 50 {
                    return joined;
                }
            }
        }
    }

    extract_general_content(doc)
}

/// Extract Bilibili video page content
fn extract_bilibili_content(doc: &Html) -> String {
    let selectors = [
        ".video-title",            // Video title
        ".video-desc",             // Video description
        ".desc-text",              // Alternative description
        ".article-content",        // For article-type content
        "meta[name='description']",
    ];

    for selector_str in &selectors {
        if let Ok(sel) = Selector::parse(selector_str) {
            if selector_str.starts_with("meta") {
                if let Some(el) = doc.select(&sel).next() {
                    if let Some(content) = el.value().attr("content") {
                        if !content.is_empty() {
                            return content.to_string();
                        }
                    }
                }
            } else {
                let texts: Vec<String> = doc.select(&sel)
                    .map(|el| element_to_markdown(&el))
                    .filter(|t| !t.trim().is_empty())
                    .collect();
                if !texts.is_empty() {
                    return texts.join("\n\n");
                }
            }
        }
    }

    extract_general_content(doc)
}

/// Extract Baijiahao article content
fn extract_baijiahao_content(doc: &Html) -> String {
    let selectors = [
        ".article-content",
        "article",
        ".content",
        ".rich-content",
    ];

    for selector_str in &selectors {
        if let Ok(sel) = Selector::parse(selector_str) {
            let texts: Vec<String> = doc.select(&sel)
                .map(|el| { let m = element_to_markdown(&el); m })
                .filter(|t| t.trim().len() > 5)
                .collect();
            if !texts.is_empty() {
                let joined = texts.join("\n\n");
                if joined.trim().len() > 50 {
                    return joined;
                }
            }
        }
    }

    extract_general_content(doc)
}

/// General content extraction: try to find the main content area
fn extract_general_content(doc: &Html) -> String {
    // Try common content selectors (ordered by specificity)
    let content_selectors = [
        "article",
        "main",
        ".post-content",
        ".article-content",
        ".article-content markdown-body",
        ".entry-content",
        ".content",
        ".post",
        ".article",
        "#content",
        "#article",
        ".markdown-body",
        ".rich-text",
        ".RichText",
        ".markdown_views",
        "#article_content",
        "#content_views",
        "[role='main']",
        ".doc-content",
        ".detail-content",
        ".news-content",
        ".article-wrapper",
        ".article-box",
    ];

    for selector_str in &content_selectors {
        if let Ok(sel) = Selector::parse(selector_str) {
            let texts: Vec<String> = doc.select(&sel)
                .map(|el| element_to_markdown(&el))
                .filter(|t| !t.trim().is_empty())
                .collect();
            let joined = texts.join(" ");
            if !texts.is_empty() && joined.trim().len() > 100 {
                return texts.join("\n\n");
            }
        }
    }

    // Try meta description as fallback
    if let Ok(sel) = Selector::parse("meta[name='description']") {
        if let Some(el) = doc.select(&sel).next() {
            if let Some(content) = el.value().attr("content") {
                if content.len() > 20 {
                    return content.to_string();
                }
            }
        }
    }
    if let Ok(sel) = Selector::parse("meta[property='og:description']") {
        if let Some(el) = doc.select(&sel).next() {
            if let Some(content) = el.value().attr("content") {
                if content.len() > 20 {
                    return content.to_string();
                }
            }
        }
    }

    // Try to find JSON-LD with article body
    if let Ok(sel) = Selector::parse("script[type='application/ld+json']") {
        for el in doc.select(&sel) {
            let text = el.text().collect::<String>();
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(article_body) = val.get("articleBody").and_then(|v| v.as_str()) {
                    if article_body.len() > 50 {
                        return article_body.to_string();
                    }
                }
            }
        }
    }

    // Try Next.js __NEXT_DATA__ script
    if let Ok(sel) = Selector::parse("script#__NEXT_DATA__") {
        for el in doc.select(&sel) {
            let text = el.text().collect::<String>();
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                // Recursively search for any large text content
                if let Some(content) = find_text_in_json(&val, 200) {
                    return content;
                }
            }
        }
    }

    // Ultra fallback: get all paragraph text
    if let Ok(p_sel) = Selector::parse("p") {
        let texts: Vec<String> = doc.select(&p_sel)
            .map(|el| element_to_markdown(&el))
            .filter(|t| t.trim().len() > 10) // filter out short/navigation text
            .collect();
        if !texts.is_empty() {
            return texts.join("\n\n");
        }
    }

    // Try headings + paragraphs together
    if let Ok(sel) = Selector::parse("h1, h2, h3, h4, h5, h6, p, li") {
        let texts: Vec<String> = doc.select(&sel)
            .map(|el| element_to_markdown(&el))
            .filter(|t| t.trim().len() > 8)
            .collect();
        if !texts.is_empty() {
            let joined = texts.join(" ");
            if joined.trim().len() > 100 {
                return texts.join("\n\n");
            }
        }
    }

    // Last resort: get body text
    if let Ok(body_sel) = Selector::parse("body") {
        let text = element_to_markdown(&doc.select(&body_sel).next().unwrap());
        if !text.trim().is_empty() {
            return text;
        }
    }

    String::new()
}

/// Recursively search JSON for large text content (for Next.js/SSR sites)
fn find_text_in_json(val: &serde_json::Value, min_len: usize) -> Option<String> {
    match val {
        serde_json::Value::String(s) => {
            if s.len() > min_len {
                Some(s.clone())
            } else {
                None
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values() {
                if let Some(found) = find_text_in_json(value, min_len) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(found) = find_text_in_json(item, min_len) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum FetchError {
    InvalidUrl(String),
    Network(String),
    Http(String),
    HttpStatus(u16, String),
    Timeout,
    EmptyContent,
    Parse(String),
    LowQuality(String, String), // (message, partial_content)
    Blocked(String),
    Config(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::InvalidUrl(msg) => write!(f, "Invalid URL: {}", msg),
            FetchError::Network(msg) => write!(f, "Network error: {}", msg),
            FetchError::Http(msg) => write!(f, "HTTP error: {}", msg),
            FetchError::HttpStatus(code, msg) => write!(f, "HTTP {}: {}", code, msg),
            FetchError::Timeout => write!(f, "Request timed out"),
            FetchError::EmptyContent => write!(f, "No readable content found on the page"),
            FetchError::Parse(msg) => write!(f, "Parse error: {}", msg),
            FetchError::LowQuality(msg, _) => write!(f, "Low quality content: {}", msg),
            FetchError::Blocked(msg) => write!(f, "Blocked: {}", msg),
            FetchError::Config(msg) => write!(f, "Configuration error: {}", msg),
        }
    }
}

impl std::error::Error for FetchError {}

// ─── SPA Shell Detection ───

/// Quick detection of JS-rendered SPA shells from raw HTML before extraction.
///
/// Checks text-to-HTML ratio, content tag density, and SPA framework markers.
/// Returns true when the page is likely a client-side rendered shell that needs
/// a JS-enabled renderer (Jina Reader) to produce meaningful content.
fn detect_spa_shell(html: &str) -> bool {
    let total = html.len();
    if total < 5000 {
        return false; // Small pages are likely simple content
    }

    let lower = html.to_lowercase();

    // ── Count structural content tags as a proxy for real server-side content ──
    let p_count = lower.matches("<p>").count() + lower.matches("<p ").count();
    let h_count = lower.matches("<h1>").count() + lower.matches("<h1 ").count()
                + lower.matches("<h2>").count() + lower.matches("<h2 ").count();
    let li_count = lower.matches("<li>").count() + lower.matches("<li ").count();
    let td_count = lower.matches("<td>").count() + lower.matches("<td ").count();
    let content_elements = p_count + h_count + li_count + td_count;

    // ── SPA mount points ──
    let has_spa_mount = ["id=\"root\"", "id=\"app\"", "id=\"__next\"",
        "id=\"__nuxt\"", "id=\"app-mount\"", "id=\"react-root\""]
        .iter().any(|p| lower.contains(p));

    // ── "Enable JavaScript" / loading messages ──
    let has_js_warning = ["enable javascript", "enable-javascript", "请启用 javascript",
        "您的浏览器不支持", "javascript is required", "loading..."]
        .iter().any(|w| lower.contains(w));

    // ── Alphanumeric text-to-HTML ratio ──
    let alpha_count: usize = lower.chars().filter(|&c| c.is_alphanumeric()).count();
    let ratio = if total > 0 { alpha_count * 100 / total } else { 0 };

    // ── Decision logic ──
    // Very few content tags + SPA mount point = almost certainly SPA
    if content_elements < 3 && has_spa_mount {
        return true;
    }
    // Very few content tags + JS warning = loading shell
    if content_elements < 3 && has_js_warning {
        return true;
    }
    // Extremely low text ratio (< 0.3%) on large pages = shell
    if total > 20000 && ratio < 1 {
        return true;
    }
    // Low text ratio + SPA mount on medium pages
    if total > 10000 && ratio < 2 && has_spa_mount {
        return true;
    }
    // JS warning with minimal content
    if content_elements < 5 && has_js_warning {
        return true;
    }

    false
}

// ─── Content Quality Scoring ───

/// Score how likely the extracted content is real article text vs SPA shell / nav junk.
/// Returns 0.0 (pure junk) to 1.0 (high-quality article).
fn content_quality_score(text: &str) -> f64 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0.0;
    }

    let lines: Vec<&str> = trimmed.lines().collect();
    let total_lines = lines.len() as f64;
    if total_lines == 0.0 {
        return 0.0;
    }

    // ── Score 1: Average meaningful line length ──
    let avg_line_len: f64 = lines.iter()
        .map(|l| l.trim().len() as f64)
        .sum::<f64>() / total_lines;
    let len_score = (avg_line_len / 40.0).min(1.0); // 40+ chars avg = perfect

    // ── Score 2: Ratio of "long" lines (>30 chars) to total ──
    let long_lines = lines.iter().filter(|l| l.trim().len() > 30).count() as f64;
    let long_ratio = long_lines / total_lines;

    // ── Score 3: Link density — too many markdown links = navigation ──
    let link_count = trimmed.matches("](http").count() as f64;
    let char_count = trimmed.len() as f64;
    let link_density = if char_count > 0.0 { link_count / (char_count / 100.0) } else { 0.0 };
    let link_penalty = if link_density > 5.0 { (5.0 / link_density).min(1.0) } else { 1.0 };

    // ── Score 4: Presence of content-indicative patterns ──
    let has_headings = trimmed.contains('#') as u8 as f64;
    let has_paragraphs = (trimmed.matches("\n\n").count() >= 2) as u8 as f64;
    let has_lists = (trimmed.contains("\n- ") || trimmed.contains("\n1. ")) as u8 as f64;
    let has_code = trimmed.contains("```") as u8 as f64;
    let structure_score = (has_headings * 0.3 + has_paragraphs * 0.3
        + has_lists * 0.2 + has_code * 0.2).min(1.0);

    // ── Score 5: URL-like lines ratio (navigation items) ──
    let url_lines = lines.iter().filter(|l| {
        let t = l.trim();
        t.starts_with('[') && t.contains("](http")
    }).count() as f64;
    let url_ratio = url_lines / total_lines;
    let url_penalty = (1.0 - url_ratio.min(0.5) * 2.0).max(0.0);

    // ── Combined score ──
    let raw = len_score * 0.15 + long_ratio * 0.20 + link_penalty * 0.10
        + structure_score * 0.20 + url_penalty * 0.15 + 0.20;

    // Clamp to [0, 1]
    raw.clamp(0.0, 1.0)
}

// ─── Content Cleanup ───

/// Check if a line is a pure bullet-list link — handles nested bullets and extra icons.
///
/// Matches patterns like:
///   `* [text](url)`
///   `- [text](url)`
///   `*   [text](url)`  (extra whitespace)
///   `*   中文 [text](url)`  (language label before link)
///   `*   - [x] [text](url)`  (nested bullet with checkbox)
///   `* [text](url) | [text](url)`  (2-column nav)
///
/// Returns true if the line is primarily a navigation link, not substantive content.
fn is_bulleted_link_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() { return false; }

    // Must start with a bullet marker
    if !t.starts_with('-') && !t.starts_with('*') && !t.starts_with('+') {
        return false;
    }

    // Strip leading bullet markers AND any nested markers recursively.
    // Example: "*   - [x] [text](url)" → after stripping bullets → "[text](url)"
    let mut rest: &str = t;
    loop {
        if rest.starts_with('-') || rest.starts_with('*') || rest.starts_with('+') {
            rest = rest[1..].trim_start(); // skip bullet char + whitespace
            // Skip checkbox prefixes like `[x]` or `[ ]`
            if rest.starts_with("[x]") || rest.starts_with("[ ]") {
                rest = rest[3..].trim_start();
            }
            continue;
        }
        break;
    }

    // At this point, rest should be the link or text content

    // Pattern A: pure standalone link — `[text](url)` (possibly with trailing text < 5 chars)
    if rest.starts_with('[') && rest.contains("](http") {
        if let Some(paren_idx) = rest.rfind(')') {
            let after_link = rest[paren_idx+1..].trim();
            if after_link.len() < 5 {
                return true;
            }
        }
    }

    // Pattern B: text followed by link with nothing else — `English [](url)` or `中文 [](url)`
    // The text before `[` should be short (language name, icon text, etc.)
    if let Some(bracket_idx) = rest.find('[') {
        let before = rest[..bracket_idx].trim();
        let after_bracket = &rest[bracket_idx..];
        if before.len() <= 10 && after_bracket.starts_with('[')
            && after_bracket.contains("](http") && after_bracket.ends_with(')') {
            // Only match if the text before link is clearly non-content (short, no punctuation)
            let is_label = before.chars().all(|c| c.is_alphabetic() || c.is_whitespace()
                || c == '·' || c == '|' || c == '/' || c == '（' || c == '）'
                || c == '(' || c == ')');
            if is_label {
                return true;
            }
        }
    }

    // Pattern C: two-column nav — `[text](url) | [text](url)`
    let link_count = rest.matches("](http").count();
    if link_count >= 2 && rest.len() < 120 {
        // Two links on one line with a separator — typical mega-nav
        return true;
    }

    false
}

/// Check if a line is a horizontal nav bar: 3+ `[text](url)` links concatenated
/// on one line, e.g. `[Docs](a)[Blog](b)[Showcase](c)`.
fn is_inline_nav_line(line: &str) -> bool {
    let t = line.trim();
    let link_count = t.matches("](http").count();
    if link_count < 3 { return false; }
    // Remove all [text](url) patterns; if little remains, it's pure nav
    let mut stripped = t.to_string();
    while let Some(start) = stripped.find("](http") {
        if let Some(open) = stripped[..=start].rfind('[') {
            if let Some(close) = stripped[start..].find(')') {
                stripped.replace_range(open..=start + close, "");
                continue;
            }
        }
        break;
    }
    stripped.trim().len() < 15
}

/// Post-process fetched content: domain-specific rules + nav cleanup + link threshold.
fn post_process_content(text: &str, domain: &str) -> String {
    let cleaned = clean_markdown_content(text);

    // ── Domain-specific rules ──
    // On docs sites the sidebar/nav always precedes the first h1.
    // Strip everything before "# " to keep only the article body.
    if domain.contains("developer.mozilla.org")
        || domain.contains("docs.github.com")
        || domain.contains("kubernetes.io")
        || domain.contains("supabase.com")
        || domain.contains("code.visualstudio.com")
    {
        if let Some(h1_pos) = cleaned.find("\n# ") {
            let body = cleaned[h1_pos..].trim().to_string();
            if body.len() > 500 {
                return body;
            }
        }
    }

    // If there are > 200 hyperlinks, the page is mostly navigation — strip all links
    if cleaned.matches("](http").count() > 200 {
        return strip_markdown_links(&cleaned);
    }
    cleaned
}

/// Strip ALL markdown links, preserving only the link text. `[text](url)` → `text`.
fn strip_markdown_links(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(bracket) = remaining.find('[') {
        // Push everything before the `[`
        result.push_str(&remaining[..bracket]);
        let after_bracket = &remaining[bracket + 1..];
        // Try to find `](http` after the `[`
        if let Some(link_start) = after_bracket.find("](http") {
            // link_text is between [ and ]
            let link_text = &after_bracket[..link_start];
            // Find closing `)` of the URL
            let after_url = &after_bracket[link_start + 2..]; // skip `](`
            if let Some(close_paren) = after_url.find(')') {
                result.push_str(link_text);
                remaining = &after_url[close_paren + 1..];
                continue;
            }
        }
        // Not a valid markdown link, keep the `[`
        result.push('[');
        remaining = after_bracket;
    }
    result.push_str(remaining);
    result
}

/// Clean up markdown content by removing navigation/boilerplate lines.
///
/// Heuristics for "nav junk":
/// - Lines that are purely a bullet-list link: `* [text](url)` or `- [text](url)`
/// - Dense clusters of 2+ nav-like lines are stripped
/// - Isolated nav links far from content paragraphs are also stripped
fn clean_markdown_content(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total == 0 {
        return text.to_string();
    }

    // ── Classify each line ──
    #[derive(PartialEq)]
    enum Kind { NavLink, Heading, Content, Blank, CodeFence }

    let kinds: Vec<Kind> = lines.iter().map(|l| {
        let t = l.trim();
        if t.is_empty() { return Kind::Blank; }
        if t.starts_with("```") { return Kind::CodeFence; }
        if t.starts_with('#') { return Kind::Heading; }

        // Pure bulleted link?
        if is_bulleted_link_line(l) {
            return Kind::NavLink;
        }

        // Inline nav bar: `[Docs](a)[Blog](b)[Showcase](c)` — 3+ links, no content
        if is_inline_nav_line(l) {
            return Kind::NavLink;
        }

        // Standalone link on its own line: `[text](url)` with nothing else
        if t.starts_with('[') && t.ends_with(')') && t.contains("](http") && t.len() < 50 {
            return Kind::NavLink;
        }

        Kind::Content
    }).collect();

    // ── Mark clusters of 2+ consecutive NavLink lines for removal ──
    let mut to_remove = vec![false; total];
    let mut i = 0;
    while i < total {
        if kinds[i] == Kind::NavLink {
            let start = i;
            while i < total && kinds[i] == Kind::NavLink { i += 1; }
            if i - start >= 2 {
                for j in start..i { to_remove[j] = true; }
            }
        } else {
            i += 1;
        }
    }

    // ── Also remove isolated NavLink lines not adjacent to Content ──
    for i in 0..total {
        if to_remove[i] || kinds[i] != Kind::NavLink { continue; }
        let prev_content = i > 0 && kinds[i-1] == Kind::Content;
        let next_content = i + 1 < total && kinds[i+1] == Kind::Content;
        if !prev_content && !next_content {
            to_remove[i] = true;
        }
    }

    // ── Rebuild output, collapsing consecutive blank lines ──
    let mut result = String::with_capacity(text.len());
    let mut last_was_blank = false;
    for i in 0..total {
        if to_remove[i] { continue; }
        let is_blank = lines[i].trim().is_empty();
        if is_blank && last_was_blank { continue; }
        result.push_str(lines[i]);
        result.push('\n');
        last_was_blank = is_blank;
    }

    result.trim().to_string()
}
