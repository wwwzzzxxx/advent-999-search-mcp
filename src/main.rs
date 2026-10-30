mod config;
mod models;
mod filters;
mod dedupe;
mod snippets;
mod engines;
mod local_search;
mod fetch;
mod python_embed;
mod anubis;
mod cookie_cache;
mod browser_cookies;
mod fetch_log;
mod search_log;
mod throttle;
mod engram;
mod x;

use config::Config;
use filters::{Freshness, FreshnessTier, SearchOptions, Topic};
use models::{SearchResponse, PartialFailure, normalize_engine};
use local_search::{find_es_exe, search_local, LocalSearchOptions};
use engines::SearchEngine;
use engines::create_engine_map;

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use serde_json::Value;

#[tokio::main]
async fn main() {
    // Log to stderr
    eprintln!("🔍 advent-999-search-mcp Rust MCP server starting...");

    let config = std::sync::Arc::new(Config::from_env());
    let engine_map: std::sync::Arc<Vec<std::sync::Arc<dyn SearchEngine>>> =
        std::sync::Arc::new(create_engine_map(&config));

    if config.allowed_search_engines.is_empty() {
        eprintln!("🔍 No engine restrictions, all {} engines available", engine_map.len());
    } else {
        eprintln!("🔍 Allowed engines: {}", config.allowed_search_engines.join(", "));
    }
    eprintln!("🔍 Default engine: {}", config.default_search_engine);
    if let Some(path) = fetch_log::path() {
        eprintln!("📝 get_page fetch log: {}", path.display());
    }
    if let Some(path) = search_log::path() {
        eprintln!("📝 web search log: {}", path.display());
    }
    if config.use_proxy {
        if let Some(ref url) = config.proxy_url {
            eprintln!("🌐 Using proxy: {}", url);
        }
        if config.direct_domains.is_empty() {
            eprintln!("🔓 DIRECT_DOMAINS=none: all requests go through the proxy");
        } else {
            eprintln!("🔓 Direct (no proxy) for: {}", config.direct_domains.join(", "));
        }
    }

    // Serialized stdout: concurrent request tasks share one JSON-RPC line
    // stream, so each response write holds this lock. Responses may arrive
    // out of order; clients match them by `id`, which JSON-RPC allows.
    let stdout_lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    // Top-level request tasks: stdin is read sequentially, requests run
    // concurrently (this is what lets N parallel `web` calls from a model
    // actually overlap; per-engine-family throttling still applies inside).
    let mut pending = tokio::task::JoinSet::new();

    let stdin = io::stdin();
    let reader = stdin.lock();
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("❌ Error reading stdin: {}", e);
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("⚠️ Invalid JSON-RPC message: {} | raw: {}", e, trimmed.chars().take(200).collect::<String>());
                continue;
            }
        };

        let method = msg["method"].as_str().unwrap_or("").to_string();
        let id = msg["id"].clone();

        match method.as_str() {
            "initialize" => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": {}
                        },
                        "serverInfo": {
                            "name": "advent-999-search-mcp",
                            // Never hard-code this again: it silently drifted
                            // from Cargo.toml through v0.11-v0.13.
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                });
                write_response_locked(&resp, &stdout_lock).await;
            }
            "notifications/initialized" => {
                // No response needed for notifications
            }
            "notifications/cancelled" => {
                // No response needed
            }
            "tools/list" => {
                let tools = list_tools(&config);
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tools }
                });
                write_response_locked(&resp, &stdout_lock).await;
            }
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("").to_string();
                let args = msg["params"]["arguments"].clone();
                let cfg = config.clone();
                let engines = engine_map.clone();
                let lock = stdout_lock.clone();
                pending.spawn(async move {
                    let result = handle_tool_call(&name, &args, &cfg, &engines).await;
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{
                                "type": "text",
                                "text": result
                            }]
                        }
                    });
                    write_response_locked(&resp, &lock).await;
                });
            }
            _ => {
                // Unknown method - respond with error if it has an id
                if !id.is_null() {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32601,
                            "message": format!("Method not found: {}", method)
                        }
                    });
                    write_response_locked(&resp, &stdout_lock).await;
                }
            }
        }
    }

    // stdin closed: drain in-flight requests before exiting.
    while pending.join_next().await.is_some() {}
}

fn write_response(resp: &Value) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let json = serde_json::to_string(resp).unwrap_or_default();
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{}", json);
    let _ = stdout.flush();
}

async fn write_response_locked(resp: &Value, lock: &std::sync::Arc<tokio::sync::Mutex<()>>) {
    let _guard = lock.lock().await;
    write_response(resp);
}

/// Label shown for an engine in the `web` tool description.
///
/// `AA` is the Artificial Analysis Search Index (data 2026-10-19, higher is
/// better); engines AA does not benchmark are marked `unscored`. The use-case
/// hints combine the AA sub-scores, provider docs, and live probes of every
/// engine on 2026-10-30 (Chinese news, Rust docs, OOM troubleshooting, papers).
fn engine_label(name: &str) -> String {
    let label = match name {
        "exa" => "Exa (AA 80; default: general/factual + semantic discovery, honours dates/domains)",
        "perplexity" => "Perplexity (AA 80; news + multi-hop, needs key)",
        "octen" => "Octen (AA 77; cheap highlights, fast, occasional off-topic)",
        "parallel" => "Parallel (AA 73 fast / 75 advanced; technical docs + multi-hop research, needs key)",
        "brave" => "Brave (AA 75 LLM context / 65 web; independent index, needs key)",
        "firecrawl" => "Firecrawl (AA 73; long pages and full-text scrape/markdown, needs key)",
        "serper" => "Serper (unscored; real Google SERP, best for keyword/news/exact entity lookup, needs key)",
        "apify" => "Apify (unscored; real Google SERP via hosted actor, $5/month free keeps refilling, slow 10-60s, needs key)",
        "deepseek" => "DeepSeek (AA 69; factual Q&A, needs key)",
        "bing" => "Bing (unscored; poor relevance, Chinese queries return homepages — avoid)",
        "duckduckgo" => "DuckDuckGo (unscored; free, forums/community; often anti-bot blocked)",
        "sogou" => "Sogou(搜狗) (unscored; Chinese news; needs fresh SNUID, IP rate-limited)",
        "weixin" => "Weixin(微信公众号) (unscored; WeChat articles for Chinese news)",
        "csdn" => "CSDN (unscored; Chinese dev blogs and troubleshooting)",
        "juejin" => "Juejin(掘金) (unscored; Chinese dev posts, relevance noisy)",
        "startpage" => "Startpage (unscored; Google results via proxy, usually Anubis-blocked)",
        "dblp" => "DBLP(计算机文献) (unscored; CS papers)",
        "cnki" => "CNKI(知网) (unscored; Chinese papers)",
        "ieee" => "IEEE Xplore (unscored; IEEE papers)",
        "semantic_scholar" => "Semantic Scholar (unscored; CS papers)",
        "x" => "X/Twitter (unscored; posts from X)",
        _ => return name.chars().next().map(|c| c.to_uppercase().to_string() + &name[1..]).unwrap_or_default(),
    };
    label.to_string()
}

fn list_tools(config: &Config) -> Vec<Value> {
    // All engines that are actually available right now (free ones + any
    // enabled by env credentials). Keeps the tools/list schema honest.
    let engine_map = create_engine_map(config);
    let mut engine_names: Vec<String> = engine_map.iter().map(|e| e.name().to_string()).collect();
    engine_names.sort();

    // If ALLOWED_SEARCH_ENGINES restricts the list, respect that for the description
    let engines_desc = if config.allowed_search_engines.is_empty() {
        engine_names.iter().map(|e| engine_label(e)).collect::<Vec<_>>().join(", ")
    } else {
        config.allowed_search_engines.iter()
            .map(|e| engine_label(e))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let allowed = engine_names;

    let engine_schema = serde_json::json!({
        "type": "string",
        "enum": allowed
    });

    let search_tool = serde_json::json!({
        "name": "web",
        "description": format!(
            "Search the web using these engines (AA = Artificial Analysis Search Index, data 2026-10-19, 0-100 higher is better; `unscored` = AA does not benchmark it): {}. Pick engines by task: default exa; Google keyword/news/exact entity lookup serper (fast) or apify (monthly free quota, slow); Chinese breaking news add sogou/weixin; Chinese dev troubleshooting add csdn; papers dblp/cnki/ieee; forums and community opinions duckduckgo; long pages firecrawl; multi-hop research parallel. `limit` is shared evenly across the requested engines: if limit is smaller than the engine count the tail engines get no slot (the response lists them in `warnings`), so pass limit >= 3x engines when you want broad coverage. Never point get_page at a search results page. Academic search tips: use dblp for CS papers, cnki for Chinese papers, ieee for IEEE Xplore; then pass any arXiv ID / URL or paper links from results to get_page to read the full text. WeChat article results (weixin.sogou.com/link?url=...) can also be passed directly to get_page. searchMode: auto uses the configured mode; request/playwright force one. For several topics pass `queries` (up to 10) in ONE call instead of several web calls: the server fans out (query x engine) concurrently under one per-engine throttle, which saves round-trips and, on clients that run tool calls one at a time, is simply faster. Each topic's results come back separately; a bad topic does not lose the others.",
            engines_desc
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query for a single topic. Use either `query` or `queries`, not both."
                },
                "queries": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1,
                    "maxItems": 10,
                    "description": "Several topics in one call (1-10); one result object per topic, in order."
                },
                "limit": {
                    "type": "number",
                    "description": "Max results per query (1-50)",
                    "default": 10
                },
                "engines": {
                    "type": "array",
                    "items": engine_schema.clone(),
                    "description": "Search engines to use",
                    "default": [config.default_search_engine]
                },
                "searchMode": {
                    "type": "string",
                    "enum": ["request", "auto", "playwright"],
                    "description": "Search mode override (optional)"
                },
                "freshness": {
                    "type": "string",
                    "description": "day | week | month | year, or YYYY-MM-DD..YYYY-MM-DD. Native for exa/sogou/csdn/ieee/serper/apify; see filters.freshnessByEngine for the rest."
                },
                "topic": {
                    "type": "string",
                    "enum": ["news", "general"],
                    "description": "news | general. Only exa honours it natively."
                },
                "includeDomains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only these domains (and subdomains)."
                },
                "excludeDomains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Drop these domains (and subdomains)."
                },
                "dedupe": {
                    "type": "boolean",
                    "description": "Fold duplicate URLs across engines into one result.",
                    "default": true
                },
                "includeEngram": {
                    "type": "boolean",
                    "description": "Attach relevant Engram memories (default when enabled).",
                    "default": true
                }
            },
            "required": []
        }
    });

    let local_tool = serde_json::json!({
        "name": "local",
        "description": "Search local filesystem using Everything (voidtools). Supports Everything query syntax: wildcards (*, ?), regex, date/size/path filters, and content: search (including indexed content search in Everything 1.5+).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "maxResults": {
                    "type": "number",
                    "description": "Max results (1-1000)"
                },
                "sort": {
                    "type": "string",
                    "enum": ["name", "path", "size", "extension", "date-created", "date-modified", "date-accessed", "date-recently-changed", "attributes", "run-count"]
                },
                "matchCase": { "type": "boolean" },
                "matchWholeWord": { "type": "boolean" },
                "matchRegex": { "type": "boolean" },
                "matchPath": { "type": "boolean" },
                "offset": { "type": "number" },
                "path": { "type": "string", "description": "Limit search to a specific path" },
                "parentPath": { "type": "string", "description": "Search files with the specified parent path" },
                "filesOnly": { "type": "boolean", "description": "Show files only (exclude folders)" },
                "foldersOnly": { "type": "boolean", "description": "Show folders only (exclude files)" },
                "contentSearch": { "type": "boolean", "description": "When true, searches file contents instead of filenames (prepends content: to query). Requires Everything content indexing to be enabled in Everything options for fast results." },
                "sizeFormat": { "type": "string", "enum": ["auto", "bytes", "kb", "mb"] },
                "dateFormat": { "type": "string", "enum": ["auto", "iso-8601", "iso-8601-utc"] }
            },
            "required": ["query"]
        }
    });

    let fetch_tool = serde_json::json!({
        "name": "get_page",
        "description": "Fetch the content of a web page and extract its readable text. Zhihu requires a logged-in session in the cookie store (d_c0 + z_c0; import it once with import_cookies); question pages are expanded with their top answers. One call can fetch several URLs: pass `urls` (up to 10) instead of `url`; each URL comes back as its own object, a failing URL does not abort the others, and the default maxLength is split into a total budget of ~60k characters so a batch cannot blow up the context. Supports: Chinese websites (Zhihu, CSDN, Juejin, Bilibili), WeChat articles (mp.weixin.qq.com URLs or weixin.sogou.com/link?url=... redirects from web search), and arXiv papers — pass a paper ID directly (2401.12345, arXiv:2401.12345, math/0501001, or an arxiv.org/abs/... URL) to get the paper's HTML content (official arxiv.org/html conversion, falling back to ar5iv for older papers). Bilibili video URLs (bilibili.com/video/BVxxx, optional ?p=N for multi-part videos) return the subtitle track as SRT — requires stored SESSDATA cookies, since Bilibili only serves the subtitle list to logged-in users. YouTube video URLs (youtube.com/watch?v=ID, /shorts/ID, /live/ID, /embed/ID, youtu.be/ID) return the caption track as SRT — anonymous, no login needed (needs proxy egress outside mainland). For academic papers: search first with web (engines: dblp, cnki, ieee, exa), then use this tool to read the full text. To read long content in chunks, pass startChar/endChar (0-based character offsets, applied per URL; the response includes totalLength so you can continue with startChar=endChar of the previous call). To avoid reading the full text, pass find (a literal substring): the server searches the fetched text and returns only the matching contexts — much cheaper than pulling the whole page and scanning it yourself. In find mode startChar/endChar/maxLength are ignored; the response carries matches (merged context windows, each with the hits inside) plus totalMatches instead of content. The response includes statusCode — non-200 (e.g. 404) means the target returned an error page. XML (RSS/Atom/sitemap) and text/plain (markdown/robots.txt) are returned verbatim without HTML extraction. Uses proxy if configured. Returns the page title and extracted content as markdown. Hyperlinks are stripped to their visible text by default; pass includeLinks:true to keep [text](url). X/Twitter links get a special rule: with x.com login cookies in the store, x.com/<user> returns the profile plus recent posts, and x.com/<user>/status/<id> returns the post plus its replies (rendered through twscrape); without a login it falls back to the public SSR page, where /search is an empty login wall.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch, or an arXiv paper ID. Use either `url` or `urls`, not both."
                },
                "urls": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1,
                    "maxItems": 10,
                    "description": "Several URLs (or arXiv IDs) to fetch in one call. Fetched concurrently (one request per host at a time). Returns {count, results:[...]} with one object per URL in input order."
                },
                "maxLength": {
                    "type": "number",
                    "description": "Maximum characters to return per URL (max 200000). Default: 50000 for one URL, and a ~60000-character total budget split across `urls` for several.",
                    "default": 50000
                },
                "startChar": {
                    "type": "number",
                    "description": "Start reading from this character offset (0-based, default 0)",
                    "default": 0
                },
                "endChar": {
                    "type": "number",
                    "description": "Read up to this character offset (exclusive). Defaults to startChar + maxLength"
                },
                "find": {
                    "type": "string",
                    "description": "Literal substring to search for in the fetched text. When set, the server returns only merged context windows around matches (no full content), saving tokens."
                },
                "contextChars": {
                    "type": "number",
                    "description": "Context characters kept before/after each match in find mode (default: 200)",
                    "default": 200
                },
                "maxMatches": {
                    "type": "number",
                    "description": "Max context windows returned in find mode (default: 20, max: 50). totalMatches still reports the full count.",
                    "default": 20
                },
                "matchCase": {
                    "type": "boolean",
                    "description": "Case-sensitive matching in find mode (ASCII only; default: false)",
                    "default": false
                },
                "highlights": {
                    "type": "string",
                    "description": "A query to rank passages against. When set, the server returns only the most relevant snippets (no full content) — use this to pull the useful parts of a long page without reading all of it. Ranks by how many distinct query terms a passage covers. ASCII words and CJK bigrams are matched. In this mode startChar/endChar/maxLength/find are ignored."
                },
                "maxHighlights": {
                    "type": "number",
                    "description": "Max snippets returned in highlights mode (default 5, max 20)",
                    "default": 5
                },
                "includeLinks": {
                    "type": "boolean",
                    "description": "Keep markdown hyperlinks [text](url) in the returned content. Default false: links are stripped to their visible text to save tokens.",
                    "default": false
                }
            },
            "required": []
        }
    });

    // Per-domain cookie store. Two jobs:
    //  - short-lived anti-bot cookies (Sogou SNUID) refreshed without a restart;
    //  - long-lived login cookies (Bilibili SESSDATA, Zhihu d_c0) used by
    //    get_page, scoped so they are only sent to their own domain.
    let set_cookies_tool = serde_json::json!({
        "name": "set_cookies",
        "description": "Manage the local, domain-scoped cookie store used by search engines and get_page. action=set (default): paste a Cookie header value from the browser for ONE domain — required after solving a Sogou captcha (SNUID) or to give get_page a login session (bilibili.com SESSDATA, zhihu.com d_c0). action=list: show which domains have cookies (names only, never values). action=clear: forget one domain, or every domain with domain=\"*\". Values are cached outside the repo and take effect on the next request — no restart. With ADVENT_ALLOW_COOKIE_IMPORT=1 an `import_cookies` tool is also available to pull cookies from a dedicated browser profile automatically.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["set", "list", "clear"],
                    "description": "set (default) | list | clear"
                },
                "domain": {
                    "type": "string",
                    "description": "Domain that owns the cookies, e.g. \"sogou.com\", \"bilibili.com\", \"zhihu.com\" (subdomains match too). Short aliases sogou/weixin/bilibili/zhihu are accepted. Required for set and clear; use \"*\" with clear to remove everything."
                },
                "site": {
                    "type": "string",
                    "description": "Deprecated alias for domain (default: sogou.com, shared by the sogou and weixin engines)."
                },
                "cookies": {
                    "type": "string",
                    "description": "Cookie header value, e.g. \"SNUID=...; SUV=...\" or \"SESSDATA=...\". A whole document.cookie dump is fine. Required for action=set."
                }
            },
            "required": []
        }
    });

    // Optional (opt-in) browser import. Registered only when the user set
    // ADVENT_ALLOW_COOKIE_IMPORT=1: it is a credential-access capability and
    // should be gated separately from pasting a cookie.
    let import_cookies_tool = serde_json::json!({
        "name": "import_cookies",
        "description": "Read cookies for specific domains from a dedicated Edge profile via the DevTools protocol. It probes the profile HEADLESSLY first (no window at all): if every requested domain already has a usable session (zhihu z_c0, bilibili SESSDATA, sogou SNUID, or any cookie for other sites) the cookies are stored and nothing is opened. Only when a login is missing and waitSeconds > 0 does it show an Edge window for the user to log in, waits, then stores the cookies and closes it. It never touches the user's everyday browser profile and only keeps the requested domains. Disabled unless the server was started with ADVENT_ALLOW_COOKIE_IMPORT=1. Warn the user before calling it in a way that can open a window (waitSeconds > 0 and a login may be missing).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1,
                    "maxItems": 5,
                    "description": "Domains to import, e.g. [\"bilibili.com\"], [\"zhihu.com\", \"sogou.com\"]. No wildcards; each is matched with its subdomains."
                },
                "loginUrl": {
                    "type": "string",
                    "description": "Page to open for logging in (default: https://<domain>/ for each domain)."
                },
                "waitSeconds": {
                    "type": "number",
                    "description": "How long to wait for the login before giving up (default 90, max 300). Cookies already present are returned immediately.",
                    "default": 90
                },
                "keepOpen": {
                    "type": "boolean",
                    "description": "Leave the Edge window open after importing (default false).",
                    "default": false
                }
            },
            "required": ["domains"]
        }
    });

    let mut tools = vec![search_tool, fetch_tool, set_cookies_tool];
    if config.cookie_import_enabled {
        tools.push(import_cookies_tool);
    }
    if find_es_exe().is_some() {
        tools.insert(1, local_tool);
    }
    tools
}

/// Manage the domain-scoped cookie store: set / list / clear.
fn handle_set_cookies(args: &Value) -> String {
    use crate::cookie_cache;

    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("set")
        .trim()
        .to_lowercase();

    match action.as_str() {
        "list" => {
            let rows = cookie_cache::list();
            let out = serde_json::json!({
                "status": "ok",
                "count": rows.len(),
                "entries": rows,
                "cache_file": cookie_cache::cache_path().display().to_string(),
                "note": "Values are never returned. Use action=clear with a domain to forget one.",
            });
            serde_json::to_string_pretty(&out).unwrap_or_default()
        }
        "clear" => {
            let raw_domain = args
                .get("domain")
                .or_else(|| args.get("site"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if raw_domain.is_empty() {
                return json_error("set_cookies(action=clear) requires a `domain` (or \"*\" for everything)");
            }
            if raw_domain == "*" {
                return match cookie_cache::clear_all() {
                    Ok(n) => serde_json::to_string_pretty(&serde_json::json!({
                        "status": "ok",
                        "cleared": "*",
                        "removedEntries": n,
                    }))
                    .unwrap_or_default(),
                    Err(e) => json_error(&format!("Failed to update the cookie store: {}", e)),
                };
            }
            let domain = match cookie_cache::canonical_domain(raw_domain) {
                Some(d) => d,
                None => return json_error(&format!("'{}' is not a valid cookie domain", raw_domain)),
            };
            match cookie_cache::clear(&domain) {
                Ok(removed) => serde_json::to_string_pretty(&serde_json::json!({
                    "status": "ok",
                    "domain": domain,
                    "removed": removed,
                }))
                .unwrap_or_default(),
                Err(e) => json_error(&format!("Failed to update the cookie store: {}", e)),
            }
        }
        _ => set_cookie_entry(args),
    }
}

fn set_cookie_entry(args: &Value) -> String {
    use crate::cookie_cache;

    let raw = args.get("cookies").and_then(|v| v.as_str()).unwrap_or("").trim();
    let raw_domain = args
        .get("domain")
        .or_else(|| args.get("site"))
        .and_then(|v| v.as_str())
        .unwrap_or("sogou")
        .trim();

    if raw.is_empty() {
        return json_error("set_cookies requires a non-empty `cookies` argument");
    }
    let domain = match cookie_cache::canonical_domain(raw_domain) {
        Some(d) => d,
        None => {
            return json_error(&format!(
                "'{}' is not a valid cookie domain — pass one like \"bilibili.com\" or \"zhihu.com\"",
                raw_domain
            ))
        }
    };

    // Sogou's trust check only needs its own handful of names; a 30-cookie
    // grab-bag is itself a bot signal. Other domains keep everything the user
    // pasted (still capped), because we cannot know what their site needs.
    let stored = if crate::cookie_cache::domain_matches("sogou.com", &domain) {
        const SOGOU_NAMES: &[&str] =
            &["SNUID", "SUV", "SUID", "ABTEST", "IPLOC", "cuid", "PHPSESSID"];
        let filtered = match cookie_cache::filter_cookie_header(raw, SOGOU_NAMES) {
            Some(f) => f,
            None => {
                return json_error(
                    "None of the expected Sogou cookies were found. Sogou needs at least `SNUID` \
                     (issued only after solving the captcha in a browser) plus `SUV`/`SUID`. \
                     Copy `document.cookie` from a page you just passed the captcha on.",
                )
            }
        };
        if !filtered.to_uppercase().contains("SNUID=") {
            return json_error(
                "No `SNUID` in the supplied cookies. SNUID is issued only after the \
                 captcha is solved — make sure you copied document.cookie from the \
                 browser session that passed it.",
            );
        }
        filtered
    } else {
        let pairs = cookie_cache::cookie_pairs(raw);
        if pairs.is_empty() {
            return json_error("`cookies` contains no name=value pairs");
        }
        if pairs.len() > 50 {
            return json_error("Refusing to store more than 50 cookies for one domain; \
                               paste only what that site needs.");
        }
        let header: Vec<String> = pairs
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        let header = header.join("; ");
        if header.len() > 8192 {
            return json_error("Cookie header exceeds 8 KB — paste only the cookies that site needs");
        }
        header
    };

    let domain = match cookie_cache::put(&domain, &stored, "paste") {
        Ok(d) => d,
        Err(e) => return json_error(&format!("Failed to write the cookie store: {}", e)),
    };

    let names = cookie_cache::cookie_names(&stored);
    let sogou = crate::cookie_cache::domain_matches("sogou.com", &domain);
    let out = serde_json::json!({
        "status": "ok",
        "domain": domain,
        "site": domain,
        "stored": names,
        "cache_file": cookie_cache::cache_path().display().to_string(),
        "note": if sogou {
            "Takes effect on the next search — no restart needed. \
             Sogou's SNUID typically lasts ~20 minutes."
        } else {
            "Takes effect on the next request to this domain or its subdomains — no restart needed. \
             Cookies are never sent to any other host."
        },
    });
    serde_json::to_string_pretty(&out).unwrap_or_default()
}

/// Handle `import_cookies` (only registered when ADVENT_ALLOW_COOKIE_IMPORT=1).
async fn handle_import_cookies(args: &Value, config: &Config) -> String {
    use crate::cookie_cache;

    if !config.cookie_import_enabled {
        return json_error(
            "import_cookies is disabled. Set ADVENT_ALLOW_COOKIE_IMPORT=1 in the MCP server \
             environment and restart the client.",
        );
    }

    let mut domains: Vec<String> = Vec::new();
    for raw in string_list(&args["domains"]) {
        match cookie_cache::canonical_domain(&raw) {
            Some(d) => {
                if !domains.contains(&d) {
                    domains.push(d);
                }
            }
            None => return json_error(&format!("'{}' is not a valid domain", raw)),
        }
    }
    if domains.is_empty() {
        return json_error("import_cookies requires `domains` (1-5 entries)");
    }
    if domains.len() > 5 {
        return json_error("At most 5 domains per import; run it twice for more");
    }

    let req = crate::browser_cookies::ImportRequest {
        domains: domains.clone(),
        login_url: args
            .get("loginUrl")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        wait_secs: (args["waitSeconds"].as_f64().unwrap_or(90.0) as u64).min(300),
        keep_open: args["keepOpen"].as_bool().unwrap_or(false),
    };
    eprintln!(
        "🔓 import_cookies: opening Edge profile for {} (wait {}s)",
        domains.join(", "),
        req.wait_secs
    );

    let outcome = match tokio::task::spawn_blocking(move || crate::browser_cookies::import(&req)).await
    {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return json_error(&format!("Cookie import failed: {}", e)),
        Err(e) => return json_error(&format!("Cookie import task failed: {}", e)),
    };

    // Group by the requested domain, first value wins on duplicates.
    let mut written: Vec<Value> = Vec::new();
    for domain in &domains {
        let mut seen: Vec<String> = Vec::new();
        let mut pairs: Vec<String> = Vec::new();
        for c in &outcome.cookies {
            let host = c.domain.trim_start_matches('.');
            if !cookie_cache::domain_matches(host, domain) {
                continue;
            }
            if seen.iter().any(|n| n == &c.name) {
                continue;
            }
            let pair = format!("{}={}", c.name, c.value);
            if reqwest::header::HeaderValue::from_str(&pair).is_err() {
                continue;
            }
            seen.push(c.name.clone());
            pairs.push(pair);
        }
        let key = crate::browser_cookies::key_cookie(domain);
        let usable = match key {
            Some(k) => seen.iter().any(|n| n.eq_ignore_ascii_case(k)),
            None => !pairs.is_empty(),
        };
        if pairs.is_empty() {
            written.push(serde_json::json!({
                "domain": domain,
                "count": 0,
                "stored": [],
                "usable": false,
                "missingKey": key,
                "note": "No cookies found — were you logged in in the window that opened?",
            }));
            continue;
        }
        match cookie_cache::put(domain, &pairs.join("; "), "browser") {
            Ok(_) => {
                let mut row = serde_json::json!({
                    "domain": domain,
                    "count": pairs.len(),
                    "stored": seen,
                    "usable": usable,
                });
                if !usable {
                    if let Some(k) = key {
                        row["missingKey"] = serde_json::json!(k);
                    }
                }
                written.push(row);
            }
            Err(e) => written.push(serde_json::json!({
                "domain": domain,
                "error": e,
            })),
        }
    }

    let has_missing = written.iter().any(|v| {
        v["count"].as_u64() == Some(0) || v["usable"].as_bool() == Some(false)
    });

    let out = serde_json::json!({
        "status": "ok",
        "profile": outcome.profile.display().to_string(),
        "domains": written,
        "cache_file": cookie_cache::cache_path().display().to_string(),
        "note": if !has_missing {
            "Cookies are stored per domain and take effect immediately. \
             The import window closes automatically unless keepOpen was set."
        } else {
            "Some domains still need a login (see usable/missingKey). Call import_cookies again \
             with waitSeconds > 0 and log in in the Edge window it opens; the profile keeps the \
             session for next time."
        },
    });
    serde_json::to_string_pretty(&out).unwrap_or_default()
}

fn json_error(message: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({"error": message})).unwrap_or_default()
}

async fn handle_tool_call(
    name: &str,
    args: &Value,
    config: &Config,
    engine_map: &[Arc<dyn SearchEngine>],
) -> String {
    match name {
        "web" => handle_web_search(args, config, engine_map).await,
        "local" => handle_local_search(args),
        "get_page" => handle_fetch(args, config).await,
        "set_cookies" => handle_set_cookies(args),
        "import_cookies" => handle_import_cookies(args, config).await,
        _ => {
            let err = serde_json::json!({
                "error": format!("Unknown tool: {}", name)
            });
            serde_json::to_string_pretty(&err).unwrap_or_default()
        }
    }
}

/// Read a JSON array of strings, trimming blanks.
fn string_list(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

async fn handle_web_search(
    args: &Value,
    config: &Config,
    engine_map: &[Arc<dyn SearchEngine>],
) -> String {
    let response = handle_web_search_without_engram(args, config, engine_map).await;
    if !config.engram_search_enabled || args["includeEngram"].as_bool() == Some(false) {
        return response;
    }
    attach_engram_memories(response, args, config).await
}

async fn attach_engram_memories(response: String, args: &Value, config: &Config) -> String {
    let mut parsed: Value = match serde_json::from_str(&response) {
        Ok(value) => value,
        Err(_) => return response,
    };

    if parsed.get("count").is_some() {
        if let Some(results) = parsed.get_mut("results").and_then(Value::as_array_mut) {
            let mut unavailable: Option<String> = None;
            for item in results {
                let Some(query) = item.get("query").and_then(Value::as_str) else { continue };
                if query.starts_with("(item ") || item.get("error").is_some() { continue; }
                if let Some(error) = &unavailable {
                    item["engramError"] = Value::String(error.clone());
                    continue;
                }
                match config.engram.search(query).await {
                    Ok(memories) => item["engramMemories"] = Value::Array(memories),
                    Err(error) => {
                        item["engramError"] = Value::String(error.clone());
                        unavailable = Some(error);
                    }
                }
            }
        }
        return serde_json::to_string_pretty(&parsed).unwrap_or(response);
    }

    if let Some(query) = args.get("query").and_then(Value::as_str).filter(|q| !q.trim().is_empty()) {
        match config.engram.search(query).await {
            Ok(memories) => parsed["engramMemories"] = Value::Array(memories),
            Err(error) => parsed["engramError"] = Value::String(error),
        }
    }
    serde_json::to_string_pretty(&parsed).unwrap_or(response)
}

async fn handle_web_search_without_engram(
    args: &Value,
    config: &Config,
    engine_map: &[Arc<dyn SearchEngine>],
) -> String {
    // One tool, two shapes: `queries` for several topics in one call, `query`
    // for the common single-topic case.
    let batch = args
        .get("queries")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if batch {
        if args.get("query").and_then(|v| v.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false) {
            return json_error("Use either `query` (single) or `queries` (several), not both");
        }
        return handle_web_queries(args, config, engine_map).await;
    }

    let query = args["query"].as_str().unwrap_or("").to_string();
    if query.trim().is_empty() {
        return json_error("Query must not be empty (pass `query`, or `queries` for several topics)");
    }

    let limit = std::cmp::min(args["limit"].as_f64().unwrap_or(10.0) as usize, 50);

    let requested_engines: Vec<String> = args["engines"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| normalize_engine(s))
                .collect()
        })
        .unwrap_or_else(|| vec![config.default_search_engine.clone()]);

    let resolved_engines = config.resolve_engines(&requested_engines);

    // ── filters ────────────────────────────────────────────────────────────
    let freshness = match args.get("freshness").and_then(|v| v.as_str()) {
        Some(raw) => match Freshness::parse(raw) {
            Some(f) => Some(f),
            None => {
                return json_error(
                    "freshness must be one of day|week|month|year, or a range like 2026-01-01..2026-02-01",
                )
            }
        },
        None => None,
    };
    let topic = match args.get("topic").and_then(|v| v.as_str()) {
        Some(raw) => match Topic::parse(raw) {
            Some(t) => Some(t),
            None => return json_error("topic must be news or general"),
        },
        None => None,
    };
    let include_domains = string_list(&args["includeDomains"]);
    let exclude_domains = string_list(&args["excludeDomains"]);
    let do_dedupe = args["dedupe"].as_bool().unwrap_or(true);

    let opts = SearchOptions {
        freshness: freshness.clone(),
        topic,
        include_domains: include_domains.clone(),
        exclude_domains: exclude_domains.clone(),
    };

    eprintln!(
        "Searching for \"{}\" using engines: {}{}",
        query,
        resolved_engines.join(", "),
        match &freshness {
            Some(f) => format!(" (freshness={})", f.label()),
            None => String::new(),
        }
    );

    // ── dispatch engine searches concurrently ──────────────────────────────
    let engine_count = resolved_engines.len();
    let per_engine = if engine_count > 0 { limit / engine_count } else { limit };
    let remainder = if engine_count > 0 { limit % engine_count } else { 0 };

    // Engines that get no share of `limit` are skipped (not an error). They are
    // logged and explained in a `warnings` entry so the caller can tell "this
    // engine found nothing" from "this engine was never asked".
    let mut skipped_engines: Vec<String> = Vec::new();

    let mut partial_failures: Vec<PartialFailure> = Vec::new();
    // engine name -> its results (or error). Collected by name so the final
    // ordering stays deterministic regardless of completion order.
    let mut collected: std::collections::HashMap<String, Result<Vec<models::SearchResult>, String>> =
        std::collections::HashMap::new();

    let mut set = tokio::task::JoinSet::new();
    for (i, engine_name) in resolved_engines.iter().enumerate() {
        let engine_limit = per_engine + if i < remainder { 1 } else { 0 };
        if engine_limit == 0 {
            skipped_engines.push(engine_name.clone());
            search_log::write(&search_log::Entry {
                query: &query,
                engine: engine_name,
                ok: false,
                results: 0,
                duration_ms: 0,
                limit,
                share: 0,
                batch: 0,
                code: "skipped",
                error: Some("no share of limit: more engines than limit"),
            });
            continue;
        }
        match engine_map.iter().find(|e| e.name() == engine_name) {
            Some(engine) => {
                let engine = engine.clone();
                let q = query.clone();
                let o = opts.clone();
                let c = config.clone();
                let n = engine_name.clone();
                set.spawn(async move {
                    let started = std::time::Instant::now();
                    // Per-family throttling: held for the whole engine call
                    // (sogou/weixin serialize, dblp/startpage capped).
                    let _permits = crate::throttle::acquire(&n).await;
                    let res = engine.search(&q, engine_limit, &o, &c).await;
                    (n, engine_limit, res, started.elapsed().as_millis())
                });
            }
            None => partial_failures.push(PartialFailure {
                engine: engine_name.clone(),
                code: "unsupported_engine".to_string(),
                message: format!("Unsupported search engine: {}", engine_name),
            }),
        }
    }

    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((name, share, res, ms)) => {
                let (ok, count, code, err) = match &res {
                    Ok(results) => (true, results.len(), "ok", None),
                    Err(e) => (false, 0, "error", Some(e.to_string())),
                };
                search_log::write(&search_log::Entry {
                    query: &query,
                    engine: &name,
                    ok,
                    results: count,
                    duration_ms: ms,
                    limit,
                    share,
                    batch: 0,
                    code,
                    error: err.as_deref(),
                });
                match res {
                    Ok(results) => {
                        eprintln!("✅ {} returned {} results", name, results.len());
                        collected.insert(name, Ok(results));
                    }
                    Err(err) => {
                        eprintln!("⚠️ {} search failed: {}", name, err);
                        collected.insert(name, Err(err.to_string()));
                    }
                }
            }
            Err(join_err) => {
                eprintln!("⚠️ engine task panicked: {}", join_err);
                partial_failures.push(PartialFailure {
                    engine: "unknown".to_string(),
                    code: "task_panic".to_string(),
                    message: join_err.to_string(),
                });
            }
        }
    }

    // Reassemble in requested-engine order to keep ordering deterministic.
    let mut all_results = Vec::new();
    for name in &resolved_engines {
        match collected.remove(name) {
            Some(Ok(results)) => all_results.extend(results),
            Some(Err(message)) => partial_failures.push(PartialFailure {
                engine: name.clone(),
                code: "engine_error".to_string(),
                message,
            }),
            None => {}
        }
    }

    // ── de-duplicate, then apply domain filters ────────────────────────────
    let duplicates_removed = if do_dedupe {
        let outcome = dedupe::dedupe(all_results);
        all_results = outcome.results;
        outcome.removed
    } else {
        0
    };

    if !include_domains.is_empty() || !exclude_domains.is_empty() {
        all_results.retain(|r| filters::domain_allowed(&r.url, &include_domains, &exclude_domains));
    }

    all_results.truncate(limit);

    // ── filter metadata (honest per-engine reporting) ──────────────────────
    let filters_meta = build_filters_meta(
        engine_map,
        &resolved_engines,
        &freshness,
        topic,
        &include_domains,
        &exclude_domains,
    );

    let mut warnings = Vec::new();
    if let Some(w) = limit_share_warning(limit, engine_count, &skipped_engines) {
        warnings.push(w);
    }

    let response = SearchResponse {
        query: query.clone(),
        engines: resolved_engines,
        total_results: all_results.len(),
        results: all_results,
        partial_failures,
        warnings,
        duplicates_removed,
        filters: filters_meta,
    };

    serde_json::to_string_pretty(&response).unwrap_or_default()
}

/// Explain engines that received no share of `limit`. The call itself still
/// succeeds: this is a note, not an error.
fn limit_share_warning(limit: usize, engine_count: usize, skipped: &[String]) -> Option<String> {
    if skipped.is_empty() {
        return None;
    }
    Some(format!(
        "limit={} is split across {} engines, so {} engine(s) got no slot and were skipped: {}. Raise limit to at least {} (or pass fewer engines) so every engine runs; engines are served in the order given.",
        limit,
        engine_count,
        skipped.len(),
        skipped.join(", "),
        engine_count
    ))
}

/// Batched parallel search: N topics in one call.
///
/// Fans out (query × engine) into one `JoinSet`; every task goes through
/// `throttle::acquire`, so sogou/weixin serialize on their shared permit
/// while stateless engines overlap freely. Per-query assembly (ordering,
/// dedupe, domain filters, `filters` meta) mirrors `handle_web_search`.
async fn handle_web_queries(
    args: &Value,
    config: &Config,
    engine_map: &[Arc<dyn SearchEngine>],
) -> String {
    let raw_items = args.get("queries").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if raw_items.is_empty() {
        return json_error("`queries` must not be empty");
    }
    if raw_items.len() > 10 {
        return json_error("`queries` supports at most 10 topics per call; split into two calls");
    }

    struct SubQuery {
        query: String,
        limit: usize,
        engines: Vec<String>,
        opts: SearchOptions,
        include_domains: Vec<String>,
        exclude_domains: Vec<String>,
        do_dedupe: bool,
    }

    /// One entry of the batch: either a runnable query or a per-item error.
    struct Prepared {
        label: String,
        sub: Option<SubQuery>,
        error: Option<String>,
    }

    // A query item may be a bare string (the common form) or an object that
    // overrides any top-level option. Invalid items become per-item errors so
    // one typo cannot lose the other nine topics.
    let mut subs: Vec<Prepared> = Vec::with_capacity(raw_items.len());
    for (i, item) in raw_items.iter().enumerate() {
        let pick = |key: &str| -> Value {
            item.get(key)
                .cloned()
                .or_else(|| args.get(key).cloned())
                .unwrap_or(Value::Null)
        };
        let query = match item {
            Value::String(s) => s.clone(),
            _ => pick("query").as_str().unwrap_or("").to_string(),
        };
        let label = if query.trim().is_empty() {
            format!("(item {})", i)
        } else {
            query.clone()
        };
        if query.trim().is_empty() {
            subs.push(Prepared {
                label,
                sub: None,
                error: Some(format!("queries[{}].query must not be empty", i)),
            });
            continue;
        }

        let limit = std::cmp::min(
            pick("limit").as_f64().unwrap_or(10.0) as usize,
            50,
        )
        .max(1);
        let requested: Vec<String> = pick("engines")
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| normalize_engine(s))
                    .collect()
            })
            .unwrap_or_else(|| vec![config.default_search_engine.clone()]);
        let resolved = config.resolve_engines(&requested);

        let freshness = match pick("freshness").as_str() {
            Some(raw) => match Freshness::parse(raw) {
                Some(f) => Some(f),
                None => {
                    subs.push(Prepared {
                        label,
                        sub: None,
                        error: Some(format!(
                            "queries[{}].freshness must be day|week|month|year, or a range like 2026-01-01..2026-02-01",
                            i
                        )),
                    });
                    continue;
                }
            },
            None => None,
        };
        let topic = match pick("topic").as_str() {
            Some(raw) => match Topic::parse(raw) {
                Some(t) => Some(t),
                None => {
                    subs.push(Prepared {
                        label,
                        sub: None,
                        error: Some(format!("queries[{}].topic must be news or general", i)),
                    });
                    continue;
                }
            },
            None => None,
        };

        let include_domains = string_list(&pick("includeDomains"));
        let exclude_domains = string_list(&pick("excludeDomains"));
        subs.push(Prepared {
            label,
            sub: Some(SubQuery {
                query,
                limit,
                engines: resolved,
                opts: SearchOptions {
                    freshness,
                    topic,
                    include_domains: include_domains.clone(),
                    exclude_domains: exclude_domains.clone(),
                },
                include_domains,
                exclude_domains,
                do_dedupe: pick("dedupe").as_bool().unwrap_or(true),
            }),
            error: None,
        });
    }

    eprintln!(
        "Batch searching {} topics: {}",
        subs.iter().filter(|p| p.sub.is_some()).count(),
        subs.iter()
            .filter_map(|p| p.sub.as_ref())
            .map(|s| format!("{:?}({})", s.query, s.engines.join("+")))
            .collect::<Vec<_>>()
            .join("; ")
    );

    // ── fan out (query × engine) ─────────────────────────────────────────
    let mut set = tokio::task::JoinSet::new();
    // qi -> engines that got no share of that topic's limit
    let mut skipped_by_query: Vec<Vec<String>> = (0..subs.len()).map(|_| Vec::new()).collect();
    for (qi, entry) in subs.iter().enumerate() {
        let Some(sub) = entry.sub.as_ref() else { continue };
        let n_engines = sub.engines.len();
        let per_engine = if n_engines > 0 { sub.limit / n_engines } else { sub.limit };
        let remainder = if n_engines > 0 { sub.limit % n_engines } else { 0 };
        for (i, engine_name) in sub.engines.iter().enumerate() {
            let engine_limit = per_engine + if i < remainder { 1 } else { 0 };
            if engine_limit == 0 {
                skipped_by_query[qi].push(engine_name.clone());
                search_log::write(&search_log::Entry {
                    query: &sub.query,
                    engine: engine_name,
                    ok: false,
                    results: 0,
                    duration_ms: 0,
                    limit: sub.limit,
                    share: 0,
                    batch: qi + 1,
                    code: "skipped",
                    error: Some("no share of limit: more engines than limit"),
                });
                continue;
            }
            match engine_map.iter().find(|e| e.name() == engine_name) {
                Some(engine) => {
                    let engine = engine.clone();
                    let q = sub.query.clone();
                    let o = sub.opts.clone();
                    let c = config.clone();
                    let n = engine_name.clone();
                    set.spawn(async move {
                        let started = std::time::Instant::now();
                        let _permits = crate::throttle::acquire(&n).await;
                        let res = engine.search(&q, engine_limit, &o, &c).await;
                        (qi, n, engine_limit, res, started.elapsed().as_millis())
                    });
                }
                None => {
                    // Unsupported engines are reported per-query below via a
                    // placeholder task result: push a synthetic failure now by
                    // spawning an immediately-ready task to keep the collect
                    // path uniform.
                    let qi_c = qi;
                    let n_c = engine_name.clone();
                    set.spawn(async move {
                        (
                            qi_c,
                            n_c,
                            engine_limit,
                            Err(models::SearchError::Engine("unsupported_engine".to_string())),
                            0u128,
                        )
                    });
                }
            }
        }
    }

    // qi -> engine -> results (or error string)
    let mut collected: Vec<std::collections::HashMap<String, Result<Vec<models::SearchResult>, String>>> =
        (0..subs.len()).map(|_| std::collections::HashMap::new()).collect();
    // qi -> unsupported-engine failures that never ran
    let mut unsupported: Vec<Vec<PartialFailure>> = (0..subs.len()).map(|_| Vec::new()).collect();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((qi, name, share, res, ms)) => {
                let (ok, count, code, err) = match &res {
                    Ok(results) => (true, results.len(), "ok", None),
                    Err(e) => (false, 0, "error", Some(e.to_string())),
                };
                if let Some(sub) = subs[qi].sub.as_ref() {
                    search_log::write(&search_log::Entry {
                        query: &sub.query,
                        engine: &name,
                        ok,
                        results: count,
                        duration_ms: ms,
                        limit: sub.limit,
                        share,
                        batch: qi + 1,
                        code,
                        error: err.as_deref(),
                    });
                }
                match res {
                    Ok(results) => {
                        eprintln!(
                            "✅ [{}] {} returned {} results",
                            subs[qi].label,
                            name,
                            results.len()
                        );
                        collected[qi].insert(name, Ok(results));
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        if msg.contains("unsupported_engine") {
                            unsupported[qi].push(PartialFailure {
                                engine: name.clone(),
                                code: "unsupported_engine".to_string(),
                                message: format!("Unsupported search engine: {}", name),
                            });
                        } else {
                            eprintln!("⚠️ [{}] {} search failed: {}", subs[qi].label, name, msg);
                            collected[qi].insert(name, Err(msg));
                        }
                    }
                }
            }
            Err(join_err) => {
                eprintln!("⚠️ batch engine task panicked: {}", join_err);
            }
        }
    }

    // ── reassemble per query, in input order ─────────────────────────────
    let mut out_queries = Vec::with_capacity(subs.len());
    for (qi, entry) in subs.iter().enumerate() {
        let sub = match entry.sub.as_ref() {
            Some(s) => s,
            None => {
                out_queries.push(serde_json::json!({
                    "query": entry.label,
                    "error": entry.error.clone().unwrap_or_default(),
                }));
                continue;
            }
        };
        let mut all_results = Vec::new();
        let mut partial_failures = unsupported[qi].clone();
        for name in &sub.engines {
            match collected[qi].remove(name) {
                Some(Ok(results)) => all_results.extend(results),
                Some(Err(message)) => partial_failures.push(PartialFailure {
                    engine: name.clone(),
                    code: "engine_error".to_string(),
                    message,
                }),
                None => {}
            }
        }
        let duplicates_removed = if sub.do_dedupe {
            let outcome = dedupe::dedupe(all_results);
            all_results = outcome.results;
            outcome.removed
        } else {
            0
        };
        if !sub.include_domains.is_empty() || !sub.exclude_domains.is_empty() {
            all_results.retain(|r| {
                filters::domain_allowed(&r.url, &sub.include_domains, &sub.exclude_domains)
            });
        }
        all_results.truncate(sub.limit);
        let filters_meta = build_filters_meta(
            engine_map,
            &sub.engines,
            &sub.opts.freshness,
            sub.opts.topic,
            &sub.include_domains,
            &sub.exclude_domains,
        );
        let mut obj = serde_json::json!({
            "query": sub.query,
            "engines": sub.engines,
            "totalResults": all_results.len(),
            "results": all_results,
            "partialFailures": partial_failures,
        });
        if let Some(w) = limit_share_warning(
            sub.limit,
            sub.engines.len(),
            &skipped_by_query[qi],
        ) {
            obj["warnings"] = serde_json::json!([w]);
        }
        if sub.do_dedupe && duplicates_removed > 0 {
            obj["duplicatesRemoved"] = serde_json::json!(duplicates_removed);
        }
        if let Some(f) = filters_meta {
            obj["filters"] = f;
        }
        out_queries.push(obj);
    }

    serde_json::to_string_pretty(&serde_json::json!({
        "count": out_queries.len(),
        "results": out_queries,
    }))
    .unwrap_or_default()
}

/// Describe the filters that were requested, including exactly how well each
/// engine could honour them. Never claims a filter was applied if it wasn't.
fn build_filters_meta(
    engine_map: &[Arc<dyn SearchEngine>],
    engines: &[String],
    freshness: &Option<Freshness>,
    topic: Option<Topic>,
    include_domains: &[String],
    exclude_domains: &[String],
) -> Option<Value> {
    if freshness.is_none()
        && topic.is_none()
        && include_domains.is_empty()
        && exclude_domains.is_empty()
    {
        return None;
    }

    let find = |n: &str| engine_map.iter().find(|e| e.name() == n);
    let mut meta = serde_json::Map::new();

    if let Some(f) = freshness {
        let (start, end) = f.iso_range(filters::today_days());
        meta.insert("freshness".to_string(), serde_json::json!(f.label()));
        meta.insert(
            "freshnessWindow".to_string(),
            serde_json::json!(format!("{}..{}", start, end)),
        );
        let mut tiers = serde_json::Map::new();
        for name in engines {
            let tier = find(name)
                .map(|e| e.freshness_tier())
                .unwrap_or(FreshnessTier::Unsupported);
            tiers.insert(name.clone(), serde_json::json!(tier.as_str()));
        }
        meta.insert("freshnessByEngine".to_string(), Value::Object(tiers));
    }

    if let Some(t) = topic {
        meta.insert("topic".to_string(), serde_json::json!(t.as_str()));
        let applied: Vec<String> = engines
            .iter()
            .filter(|n| find(n).map(|e| e.supports_topic()).unwrap_or(false))
            .cloned()
            .collect();
        meta.insert("topicAppliedBy".to_string(), serde_json::json!(applied));
    }

    if !include_domains.is_empty() {
        meta.insert(
            "includeDomains".to_string(),
            serde_json::json!(include_domains),
        );
    }
    if !exclude_domains.is_empty() {
        meta.insert(
            "excludeDomains".to_string(),
            serde_json::json!(exclude_domains),
        );
    }

    Some(Value::Object(meta))
}

fn handle_local_search(args: &Value) -> String {
    let query = args["query"].as_str().unwrap_or("").to_string();
    if query.trim().is_empty() {
        return serde_json::to_string_pretty(&serde_json::json!({
            "error": "Query must not be empty"
        })).unwrap_or_default();
    }

    let options = LocalSearchOptions {
        query,
        max_results: args["maxResults"].as_f64().map(|v| v as u32),
        sort: args["sort"].as_str().map(|s| s.to_string()),
        match_case: args["matchCase"].as_bool(),
        match_whole_word: args["matchWholeWord"].as_bool(),
        match_regex: args["matchRegex"].as_bool(),
        match_path: args["matchPath"].as_bool(),
        offset: args["offset"].as_f64().map(|v| v as u32),
        path: args["path"].as_str().map(|s| s.to_string()),
        parent_path: args["parentPath"].as_str().map(|s| s.to_string()),
        files_only: args["filesOnly"].as_bool(),
        folders_only: args["foldersOnly"].as_bool(),
        content_search: args["contentSearch"].as_bool(),
        size_format: args["sizeFormat"].as_str().map(|s| s.to_string()),
        date_format: args["dateFormat"].as_str().map(|s| s.to_string()),
    };

    match search_local(&options) {
        Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
        Err(e) => {
            serde_json::to_string_pretty(&serde_json::json!({
                "error": e
            })).unwrap_or_default()
        }
    }
}

/// Per-URL token budget, so a 10-URL call cannot dump half a million
/// characters into the caller's context.
struct FetchBudget {
    max_length: usize,
    max_matches: usize,
    max_highlights: usize,
}

/// Collect `url` / `urls`, de-duplicated and capped.
fn collect_urls(args: &Value) -> Result<Vec<String>, String> {
    let mut urls: Vec<String> = Vec::new();
    if let Some(u) = args.get("url").and_then(|v| v.as_str()) {
        let u = u.trim();
        if !u.is_empty() {
            urls.push(u.to_string());
        }
    }
    if let Some(arr) = args.get("urls").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(u) = v.as_str() {
                let u = u.trim();
                if !u.is_empty() && !urls.iter().any(|x| x == u) {
                    urls.push(u.to_string());
                }
            }
        }
    }
    if urls.is_empty() {
        return Err("URL must not be empty (pass `url`, or `urls` for several)".to_string());
    }
    if urls.len() > 10 {
        return Err(format!(
            "At most 10 URLs per call ({} given); split into two calls",
            urls.len()
        ));
    }
    Ok(urls)
}

/// Concurrency limits for multi-URL fetches: at most one request per host and
/// a small global cap, so ten links to one site do not look like an attack.
static FETCH_GLOBAL_SEM: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> =
    std::sync::OnceLock::new();
static FETCH_HOST_SEMS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Semaphore>>>,
> = std::sync::OnceLock::new();

async fn fetch_permits(
    url: &str,
) -> (
    tokio::sync::OwnedSemaphorePermit,
    tokio::sync::OwnedSemaphorePermit,
) {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| url.to_string());
    let host_sem = {
        let mut guard = FETCH_HOST_SEMS
            .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard
            .entry(host)
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(1)))
            .clone()
    };
    let host_permit = host_sem
        .acquire_owned()
        .await
        .expect("fetch host semaphore closed");
    let global_sem = FETCH_GLOBAL_SEM
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(6)))
        .clone();
    let global_permit = global_sem
        .acquire_owned()
        .await
        .expect("fetch global semaphore closed");
    (host_permit, global_permit)
}

async fn handle_fetch(args: &Value, config: &Config) -> String {
    let urls = match collect_urls(args) {
        Ok(u) => u,
        Err(e) => return json_error(&e),
    };

    // Validate these once: they would otherwise fail identically in every
    // per-URL task.
    if args.get("find").is_some() && args["find"].as_str().unwrap_or("").is_empty() {
        return json_error("find must not be empty");
    }
    if args.get("highlights").is_some() && args["highlights"].as_str().unwrap_or("").trim().is_empty()
    {
        return json_error("highlights must not be empty");
    }

    let n = urls.len();
    let explicit_max = args["maxLength"].as_f64().map(|v| v as usize);
    let max_length = match explicit_max {
        Some(m) => m.clamp(1, 200_000),
        // Several URLs: split a single ~60k-character budget instead of
        // handing each page the full 50k default.
        None if n > 1 => (60_000 / n).clamp(4_000, 50_000),
        None => 50_000,
    };
    let max_matches = match args["maxMatches"].as_f64() {
        Some(m) => (m as usize).clamp(1, 50),
        None if n > 1 => (20 / n).max(3),
        None => 20,
    };
    let max_highlights = match args["maxHighlights"].as_f64() {
        Some(m) => (m as usize).clamp(1, 20),
        None if n > 1 => 2,
        None => 5,
    };
    let budget = FetchBudget { max_length, max_matches, max_highlights };

    if n == 1 {
        let value = fetch_one(&urls[0], args, config, &budget, 1, 1).await;
        return serde_json::to_string_pretty(&value).unwrap_or_default();
    }

    eprintln!("🌐 Fetching {} URLs concurrently", n);
    let mut set = tokio::task::JoinSet::new();
    for (i, url) in urls.iter().cloned().enumerate() {
        let args = args.clone();
        let config = config.clone();
        let budget_len = budget.max_length;
        let budget_matches = budget.max_matches;
        let budget_highlights = budget.max_highlights;
        set.spawn(async move {
            let _permits = fetch_permits(&url).await;
            let b = FetchBudget {
                max_length: budget_len,
                max_matches: budget_matches,
                max_highlights: budget_highlights,
            };
            (i, fetch_one(&url, &args, &config, &b, n, i + 1).await)
        });
    }

    let mut slots: Vec<Option<Value>> = (0..n).map(|_| None).collect();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((i, value)) => slots[i] = Some(value),
            Err(e) => eprintln!("⚠️ fetch task panicked: {}", e),
        }
    }

    let results: Vec<Value> = slots
        .into_iter()
        .enumerate()
        .map(|(i, slot)| {
            slot.unwrap_or_else(|| {
                serde_json::json!({
                    "url": urls[i],
                    "error": "fetch task failed",
                })
            })
        })
        .collect();

    serde_json::to_string_pretty(&serde_json::json!({
        "count": results.len(),
        "results": results,
    }))
    .unwrap_or_default()
}

/// Fetch and shape one URL. The returned object matches the single-URL
/// response shape exactly (content / find / highlights modes), plus per-URL
/// errors which never abort a batch.
async fn fetch_one(url: &str, args: &Value, config: &Config, budget: &FetchBudget, batch: usize, index: usize) -> Value {
    let max_length = budget.max_length;

    // Optional character-range reading (0-based, half-open [start, end)).
    let start_char = args["startChar"].as_f64().unwrap_or(0.0) as usize;
    let end_char = args["endChar"].as_f64().map(|v| v as usize);
    // Hyperlinks are stripped by default to save tokens; pass
    // `includeLinks: true` to keep `[text](url)` in the output.
    let keep_links = args["includeLinks"].as_bool().unwrap_or(false);

    eprintln!("🌐 Fetching URL: {}", url);

    // One automatic retry on timeout — Sogou redirect chains occasionally
    // exceed the fetch timeout on the first attempt.
    let mode = if args.get("find").is_some() {
        "find"
    } else if args.get("highlights").is_some() {
        "highlights"
    } else {
        "content"
    };
    let started = std::time::Instant::now();

    // Stored cookie names for this host (names only — the log must never
    // contain a usable credential).
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_default();
    let cookie_names = crate::cookie_cache::cookie_names(&config.cookies_for(&host));

    let retried = std::sync::atomic::AtomicBool::new(false);
    let log = |site: &str,
               ok: bool,
               status: Option<u16>,
               error: Option<&str>,
               total_chars: usize,
               returned_chars: usize,
               truncated: bool,
               via: &str| {
        crate::fetch_log::write(&crate::fetch_log::Entry {
            url,
            host: &host,
            site,
            mode,
            ok,
            status,
            error,
            total_chars,
            returned_chars,
            truncated,
            duration_ms: started.elapsed().as_millis(),
            via,
            cookie_names: cookie_names.clone(),
            index,
            batch,
            retried: retried.load(std::sync::atomic::Ordering::Relaxed),
        });
    };

    // One automatic retry on timeout — Sogou redirect chains occasionally
    // exceed the fetch timeout on the first attempt.
    // X/Twitter: a logged-in session lets twscrape render the profile or post
    // as structured markdown. `None` means "not ours" (guest, other URL, or
    // twscrape absent) and the normal HTTP path runs unchanged.
    let mut fetch_result = match crate::x::fetch_page(url, config).await {
        Some(Ok(result)) => Ok(result),
        Some(Err(e)) => {
            eprintln!("⚠️ x: {} — falling back to normal fetch for {}", e, url);
            fetch::fetch_url(url, config).await
        }
        None => fetch::fetch_url(url, config).await,
    };
    if matches!(fetch_result, Err(fetch::FetchError::Timeout)) {
        eprintln!("⏱️ Timeout on first attempt, retrying once...");
        retried.store(true, std::sync::atomic::Ordering::Relaxed);
        fetch_result = fetch::fetch_url(url, config).await;
    }

    let result = match fetch_result {
        Ok(result) => result,
        Err(e) => {
            eprintln!("❌ Fetch failed for {}: {}", url, e);
            log("", false, None, Some(&e.to_string()), 0, 0, false, "");
            return serde_json::json!({
                "url": url,
                "error": e.to_string()
            });
        }
    };

    let full = fetch::apply_link_policy(result.content, &result.content_type, keep_links);
    let total = full.chars().count();

    // ── find mode: literal substring search over the fetched text ──
    // Returns only merged context windows around matches, so callers don't
    // have to pull the full page and scan it with attention.
    // startChar/endChar/maxLength are ignored in this mode.
    if args.get("find").is_some() {
        let needle = args["find"].as_str().unwrap_or("");
        if needle.is_empty() {
            return serde_json::json!({ "url": result.url, "error": "find must not be empty" });
        }
        let context_chars = (args["contextChars"].as_f64().unwrap_or(200.0) as usize).min(2000);
        let max_matches = budget.max_matches;
        let match_case = args["matchCase"].as_bool().unwrap_or(false);

        let hay: Vec<char> = full.chars().collect();
        let pat: Vec<char> = needle.chars().collect();
        let mut hits: Vec<(usize, usize)> = Vec::new();
        if pat.len() <= hay.len() {
            for i in 0..=hay.len() - pat.len() {
                let mut ok = true;
                for j in 0..pat.len() {
                    let (c, p) = (hay[i + j], pat[j]);
                    if c != p && (match_case || !c.eq_ignore_ascii_case(&p)) {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    hits.push((i, i + pat.len()));
                }
            }
        }
        let total_matches = hits.len();
        eprintln!(
            "🔎 find {:?} → {} matches (case={})",
            needle, total_matches, match_case
        );

        // Merge overlapping/adjacent context windows.
        let mut windows: Vec<(usize, usize)> = Vec::new();
        for (s, e) in &hits {
            let ws = s.saturating_sub(context_chars);
            let we = std::cmp::min(e + context_chars, total);
            if let Some(last) = windows.last_mut() {
                if ws <= last.1 {
                    last.1 = last.1.max(we);
                    continue;
                }
            }
            windows.push((ws, we));
        }
        let total_windows = windows.len();
        let truncated = total_windows > max_matches;
        windows.truncate(max_matches);

        // Attribute hits to their (possibly merged) window.
        let mut matches = Vec::new();
        let mut hi = 0;
        for (ws, we) in &windows {
            let mut win_hits = Vec::new();
            while hi < hits.len() && hits[hi].0 < *we {
                // Defensive: skip any hit ending before this window.
                if hits[hi].1 > *ws {
                    win_hits.push(serde_json::json!({
                        "startChar": hits[hi].0,
                        "endChar": hits[hi].1,
                    }));
                }
                hi += 1;
            }
            let context: String = hay[*ws..*we].iter().collect();
            matches.push(serde_json::json!({
                "startChar": ws,
                "endChar": we,
                "hits": win_hits,
                "context": context,
            }));
        }

        let mut response = serde_json::json!({
            "url": result.url,
            "title": result.title,
            "site": result.site_name,
            "contentType": result.content_type,
            "statusCode": result.status_code,
            "find": needle,
            "matchCase": match_case,
            "contextChars": context_chars,
            "totalLength": total,
            "totalMatches": total_matches,
            "totalWindows": total_windows,
            "truncated": truncated,
            "matches": matches,
        });
        if total_matches == 0 {
            response["hint"] = serde_json::json!(
                "No matches found. Try different keywords (find is a literal substring, case-insensitive by default) or fetch the full content without find."
            );
        }
        let returned: usize = matches
            .iter()
            .filter_map(|m| m["context"].as_str().map(|c| c.chars().count()))
            .sum();
        log(
            &result.site_name,
            true,
            Some(result.status_code),
            None,
            total,
            returned,
            truncated,
            &result.via,
        );
        return response;
    }

    // ── highlights mode: rank passages against a query ──
    // Like find, but instead of an exact literal it takes a query and returns
    // the passages that cover the most distinct query terms. Purely local;
    // startChar/endChar/maxLength/find are ignored.
    if args.get("highlights").is_some() {
        let hq = args["highlights"].as_str().unwrap_or("");
        if hq.trim().is_empty() {
            return serde_json::json!({ "url": result.url, "error": "highlights must not be empty" });
        }
        let context_chars = (args["contextChars"].as_f64().unwrap_or(200.0) as usize).min(2000);
        let max_highlights = budget.max_highlights;

        let snips = snippets::highlight(&full, hq, context_chars, max_highlights);
        eprintln!("✨ highlights {:?} → {} snippets", hq, snips.len());

        let snippets_json: Vec<Value> = snips
            .iter()
            .map(|s| {
                serde_json::json!({
                    "startChar": s.start_char,
                    "endChar": s.end_char,
                    "score": s.score,
                    "terms": s.terms,
                    "context": s.context,
                })
            })
            .collect();

        let mut response = serde_json::json!({
            "url": result.url,
            "title": result.title,
            "site": result.site_name,
            "contentType": result.content_type,
            "statusCode": result.status_code,
            "highlights": hq,
            "contextChars": context_chars,
            "totalLength": total,
            "totalSnippets": snips.len(),
            "snippets": snippets_json,
        });
        if snips.is_empty() {
            response["hint"] = serde_json::json!(
                "No query terms matched this page. Try different keywords, or fetch the full content without highlights."
            );
        }
        let returned: usize = snips.iter().map(|s| s.context.chars().count()).sum();
        log(
            &result.site_name,
            true,
            Some(result.status_code),
            None,
            total,
            returned,
            false,
            &result.via,
        );
        return response;
    }

    let start = std::cmp::min(start_char, total);
    let end = match end_char {
        Some(e) => std::cmp::min(e, total),
        None => std::cmp::min(start + max_length, total),
    };
    let window_len = end.saturating_sub(start);
    let window_len = std::cmp::min(window_len, max_length);

    let mut content: String = full.chars().skip(start).take(window_len).collect();

    // Truncate if the window itself exceeds max_length (defensive).
    let chars_len = content.chars().count();
    if chars_len > max_length {
        content = content.chars().take(max_length).collect::<String>();
    }

    // Note when we are not at the end of the document.
    let actual_end = start + content.chars().count();
    let truncated = actual_end < total;
    if truncated {
        content.push_str(&format!(
            "\n\n... [内容较长，已读取第 {}–{} 字符，共 {} 字符。继续读取请调用 get_page 并传 startChar={}]\n",
            start, actual_end, total, actual_end
        ));
    }

    let returned = content.chars().count();
    // 404/410/451 carry the server's error page as content (useful: a missing
    // release asset vs. a deleted page), but it must not look like a success —
    // otherwise the caller quotes "404: Not Found" as if it were the document.
    let http_error = (result.status_code >= 400).then(|| {
        format!(
            "HTTP {} — the server returned an error page, not the requested content",
            result.status_code
        )
    });
    log(
        &result.site_name,
        http_error.is_none(),
        Some(result.status_code),
        http_error.as_deref(),
        total,
        returned,
        truncated,
        &result.via,
    );

    let mut response = serde_json::json!({
        "url": result.url,
        "title": result.title,
        "site": result.site_name,
        "content": content,
        "contentType": result.content_type,
        "statusCode": result.status_code,
        "startChar": start,
        "endChar": actual_end,
        "totalLength": total,
        "truncated": truncated,
    });
    if let Some(err) = http_error {
        response["ok"] = serde_json::json!(false);
        response["error"] = serde_json::json!(err);
        response["hint"] = serde_json::json!(
            "The URL may be wrong, deleted, or private. Verify the link before quoting this text: the content field holds the server's error page, not the document."
        );
    }
    response
}
