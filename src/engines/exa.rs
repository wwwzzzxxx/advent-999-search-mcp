use async_trait::async_trait;
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

pub struct ExaEngine;

#[async_trait]
impl SearchEngine for ExaEngine {
    fn name(&self) -> &'static str { "exa" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let client = config.build_reqwest_client()
            .map_err(|e| SearchError::Http(e.to_string()))?;

        // Initialize Exa MCP session
        let init = mcp_call(&client, "initialize", serde_json::json!({
            "protocolVersion": "2024-11-05",
            "clientInfo": { "name": "advent-999-search-mcp", "version": "0.1.0" },
            "capabilities": {}
        }), None).await?;

        let session_id = init.1;

        // Call web_search_exa
        let (call_resp, _) = mcp_call(&client, "tools/call", serde_json::json!({
            "name": "web_search_exa",
            "arguments": {
                "query": query,
                "numResults": limit as u64,
                "type": "auto"
            }
        }), session_id).await?;

        // Extract text content from response
        let text = call_resp["result"]["content"]
            .as_array()
            .and_then(|arr| arr.iter().find(|c| c["type"] == "text"))
            .and_then(|c| c["text"].as_str())
            .or_else(|| {
                // Try double-nested (result.result.content)
                call_resp["result"]["result"]["content"]
                    .as_array()
                    .and_then(|arr| arr.iter().find(|c| c["type"] == "text"))
                    .and_then(|c| c["text"].as_str())
            })
            .unwrap_or("");

        if text.is_empty() {
            return Ok(vec![]);
        }

        // Parse results from Exa's text format
        let mut results = Vec::new();
        for block in text.split("\n\n") {
            if !block.starts_with("Title:") { continue; }

            let mut title = String::new();
            let mut url = String::new();
            let mut description = String::new();

            for line in block.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("Title: ") {
                    title = rest.to_string();
                } else if let Some(rest) = line.strip_prefix("URL: ") {
                    url = rest.to_string();
                } else if line == "Highlights:" {
                    // Description follows
                } else if !title.is_empty() && !url.is_empty() && !line.is_empty() {
                    if !description.is_empty() { description.push('\n'); }
                    description.push_str(line);
                }
            }

            if url.is_empty() { continue; }

            let source = extract_hostname(&url).unwrap_or_default();
            results.push(SearchResult {
                title: title.trim().to_string(),
                url: url.trim().to_string(),
                description: description.trim().to_string(),
                source,
                engine: "exa".to_string(),
                summary: None,
            });

            if results.len() >= limit { break; }
        }

        Ok(results)
    }
}

async fn mcp_call(
    client: &reqwest::Client,
    method: &str,
    params: serde_json::Value,
    session_id: Option<String>,
) -> Result<(serde_json::Value, Option<String>), SearchError> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    });

    let mut req = client.post("https://mcp.exa.ai/mcp?tools=web_search_exa")
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&body);

    // Support EXA_API_KEY env var (matching OhMyOpenAgent behavior)
    if let Ok(api_key) = std::env::var("EXA_API_KEY") {
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }
    }

    if let Some(ref sid) = session_id {
        req = req.header("Mcp-Session-Id", sid);
    }

    let resp = req.send().await
        .map_err(|e| SearchError::Http(format!("Exa MCP request failed: {}", e)))?;

    let new_session_id = resp.headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let text = resp.text().await
        .map_err(|e| SearchError::Http(format!("Exa read response failed: {}", e)))?;

    // Handle SSE or JSON
    let json = if let Some(data_line) = text.lines().find(|l| l.starts_with("data: ")) {
        let json_str = data_line.strip_prefix("data: ").unwrap_or("");
        serde_json::from_str(json_str).unwrap_or_else(|_| serde_json::Value::Null)
    } else {
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
    };

    if json.is_null() {
        return Err(SearchError::Parse("Exa returned invalid response".to_string()));
    }
    if json.get("error").is_some() {
        return Err(SearchError::Engine(format!("Exa error: {}", json["error"])));
    }

    Ok((json, new_session_id.or(session_id)))
}

fn extract_hostname(url: &str) -> Option<String> {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .map(|s| s.to_string())
}
