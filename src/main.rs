mod config;
mod models;
mod engines;
mod local_search;
mod fetch;

use config::Config;
use models::{SearchResponse, PartialFailure, normalize_engine};
use local_search::{find_es_exe, search_local, LocalSearchOptions};
use engines::SearchEngine;
use engines::create_engine_map;

use std::io::{self, BufRead, Write};
use serde_json::Value;

#[tokio::main]
async fn main() {
    // Log to stderr
    eprintln!("🔍 advent-999-search-mcp Rust MCP server starting...");

    let config = Config::from_env();
    let engine_map = create_engine_map(&config);

    if config.allowed_search_engines.is_empty() {
        eprintln!("🔍 No engine restrictions, all {} engines available", engine_map.len());
    } else {
        eprintln!("🔍 Allowed engines: {}", config.allowed_search_engines.join(", "));
    }
    eprintln!("🔍 Default engine: {}", config.default_search_engine);
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
                            "version": "0.2.0"
                        }
                    }
                });
                write_response(&resp);
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
                write_response(&resp);
            }
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("");
                let args = &msg["params"]["arguments"];
                let result = handle_tool_call(name, args, &config, &engine_map).await;
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
                write_response(&resp);
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
                    write_response(&resp);
                }
            }
        }
    }
}

fn write_response(resp: &Value) {
    let json = serde_json::to_string(resp).unwrap_or_default();
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{}", json);
    let _ = stdout.flush();
}

fn list_tools(config: &Config) -> Vec<Value> {
    // All engines that are actually available right now (free ones + any
    // enabled by env credentials). Keeps the tools/list schema honest.
    let engine_map = create_engine_map(config);
    let mut engine_names: Vec<String> = engine_map.iter().map(|e| e.name().to_string()).collect();
    engine_names.sort();

    // If ALLOWED_SEARCH_ENGINES restricts the list, respect that for the description
    let engines_desc = if config.allowed_search_engines.is_empty() {
        let labeled: Vec<String> = engine_names.iter().map(|e| match e.as_str() {
            "juejin" => "Juejin(掘金)".to_string(),
            "startpage" => "Startpage".to_string(),
            "sogou" => "Sogou(搜狗)".to_string(),
            "weixin" => "Weixin(微信公众号)".to_string(),
            "dblp" => "DBLP(计算机文献)".to_string(),
            "semantic_scholar" => "Semantic Scholar".to_string(),
            "ieee" => "IEEE Xplore".to_string(),
            "cnki" => "CNKI(知网)".to_string(),
            _ => e.chars().next().map(|c| c.to_uppercase().to_string() + &e[1..]).unwrap_or_default(),
        }).collect();
        labeled.join(", ")
    } else {
        config.allowed_search_engines.iter()
            .map(|e| match e.as_str() {
                "juejin" => "Juejin(掘金)".to_string(),
                "startpage" => "Startpage".to_string(),
                "sogou" => "Sogou(搜狗)".to_string(),
                "weixin" => "Weixin(微信公众号)".to_string(),
                "dblp" => "DBLP(计算机文献)".to_string(),
                "semantic_scholar" => "Semantic Scholar".to_string(),
                "ieee" => "IEEE Xplore".to_string(),
                "cnki" => "CNKI(知网)".to_string(),
                _ => e.chars().next().map(|c| c.to_uppercase().to_string() + &e[1..]).unwrap_or_default(),
            })
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
            "Search the web using these engines: {} (no API key required). searchMode meanings: omit or set auto to use the server configured SEARCH_MODE; request forces request-based search; playwright forces browser-based search.",
            engines_desc
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "limit": {
                    "type": "number",
                    "description": "Max results (1-50)",
                    "default": 10
                },
                "engines": {
                    "type": "array",
                    "items": engine_schema,
                    "description": "Search engines to use",
                    "default": [config.default_search_engine]
                },
                "searchMode": {
                    "type": "string",
                    "enum": ["request", "auto", "playwright"],
                    "description": "Search mode override (optional)"
                }
            },
            "required": ["query"]
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
        "description": "Fetch the content of a web page and extract its readable text. Supports Chinese websites like Zhihu, CSDN, Juejin, Bilibili, WeChat articles (mp.weixin.qq.com), etc. Also accepts an arXiv paper ID (e.g. 2401.12345, arXiv:2401.12345, or an arxiv.org/abs/... URL) and returns the paper's HTML content. Uses proxy if configured. Returns the page title and extracted content as markdown.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch"
                },
                "maxLength": {
                    "type": "number",
                    "description": "Maximum characters to return (default: 50000, max: 200000)",
                    "default": 50000
                }
            },
            "required": ["url"]
        }
    });

    let mut tools = vec![search_tool, fetch_tool];
    if find_es_exe().is_some() {
        tools.insert(1, local_tool);
    }
    tools
}

async fn handle_tool_call(
    name: &str,
    args: &Value,
    config: &Config,
    engine_map: &[Box<dyn SearchEngine>],
) -> String {
    match name {
        "web" => handle_web_search(args, config, engine_map).await,
        "local" => handle_local_search(args),
        "get_page" => handle_fetch(args, config).await,
        _ => {
            let err = serde_json::json!({
                "error": format!("Unknown tool: {}", name)
            });
            serde_json::to_string_pretty(&err).unwrap_or_default()
        }
    }
}

async fn handle_web_search(
    args: &Value,
    config: &Config,
    engine_map: &[Box<dyn SearchEngine>],
) -> String {
    let query = args["query"].as_str().unwrap_or("").to_string();
    if query.trim().is_empty() {
        return serde_json::to_string_pretty(&serde_json::json!({
            "error": "Query must not be empty"
        })).unwrap_or_default();
    }

    let limit = args["limit"].as_f64().unwrap_or(10.0) as usize;
    let limit = std::cmp::min(limit, 50);

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

    eprintln!("Searching for \"{}\" using engines: {}", query, resolved_engines.join(", "));

    let mut all_results = Vec::new();
    let mut partial_failures = Vec::new();
    let engine_count = resolved_engines.len();
    let per_engine = if engine_count > 0 { limit / engine_count } else { limit };
    let remainder = if engine_count > 0 { limit % engine_count } else { 0 };

    for (i, engine_name) in resolved_engines.iter().enumerate() {
        let engine_limit = per_engine + if i < remainder { 1 } else { 0 };
        if engine_limit == 0 { continue; }

        let engine = engine_map.iter().find(|e| e.name() == engine_name);
        match engine {
            Some(e) => {
                match e.search(&query, engine_limit, config).await {
                    Ok(results) => {
                        eprintln!("✅ {} returned {} results", engine_name, results.len());
                        all_results.extend(results);
                    }
                    Err(err) => {
                        eprintln!("⚠️ {} search failed: {}", engine_name, err);
                        partial_failures.push(PartialFailure {
                            engine: engine_name.clone(),
                            code: "engine_error".to_string(),
                            message: err.to_string(),
                        });
                    }
                }
            }
            None => {
                partial_failures.push(PartialFailure {
                    engine: engine_name.clone(),
                    code: "unsupported_engine".to_string(),
                    message: format!("Unsupported search engine: {}", engine_name),
                });
            }
        }
    }

    // Sort results by relevance? Keep original order.
    all_results.truncate(limit);

    let response = SearchResponse {
        query: query.clone(),
        engines: resolved_engines,
        total_results: all_results.len(),
        results: all_results,
        partial_failures,
    };

    serde_json::to_string_pretty(&response).unwrap_or_default()
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

async fn handle_fetch(args: &Value, config: &Config) -> String {
    let url = args["url"].as_str().unwrap_or("").to_string();
    if url.trim().is_empty() {
        return serde_json::to_string_pretty(&serde_json::json!({
            "error": "URL must not be empty"
        })).unwrap_or_default();
    }

    let max_length = args["maxLength"].as_f64().unwrap_or(50000.0) as usize;
    let max_length = std::cmp::min(max_length, 200000);

    eprintln!("🌐 Fetching URL: {}", url);

    match fetch::fetch_url(&url, config).await {
        Ok(result) => {
            let mut content = result.content;

            // Truncate if too long
            if content.len() > max_length {
                content = content.chars().take(max_length).collect::<String>()
                    + &format!("\n\n... [内容过长，已截断至 {} 字符，原始长度 {} 字符]", max_length, content.len());
            }

            let response = serde_json::json!({
                "url": result.url,
                "title": result.title,
                "site": result.site_name,
                "content": content,
                "contentType": result.content_type,
            });

            serde_json::to_string_pretty(&response).unwrap_or_default()
        }
        Err(e) => {
            eprintln!("❌ Fetch failed for {}: {}", url, e);
            serde_json::to_string_pretty(&serde_json::json!({
                "url": url,
                "error": e.to_string()
            })).unwrap_or_default()
        }
    }
}
