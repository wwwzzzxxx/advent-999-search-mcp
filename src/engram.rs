//! Optional bridge to an Engram MCP server over stdio.
//!
//! The web-search MCP cannot call sibling MCP tools directly. When enabled,
//! it starts a separately configured Engram MCP child process and reuses that
//! stdio session for searches.

use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tokio::time::timeout;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const DEFAULT_ARGS: &str = r#"["mcp","--tools=agent"]"#;

#[derive(Clone)]
pub struct EngramClient {
    command: Arc<String>,
    args: Arc<Vec<String>>,
    args_error: Arc<Option<String>>,
    session: Arc<Mutex<Option<EngramSession>>>,
}

impl std::fmt::Debug for EngramClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngramClient")
            .field("command", &self.command)
            .field("args", &"[configured]")
            .finish_non_exhaustive()
    }
}

struct EngramSession {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl EngramClient {
    pub fn from_env() -> Self {
        let command = std::env::var("ENGRAM_MCP_COMMAND")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "engram".to_string());
        let raw_args = std::env::var("ENGRAM_MCP_ARGS_JSON")
            .unwrap_or_else(|_| DEFAULT_ARGS.to_string());
        let (args, args_error) = match serde_json::from_str::<Vec<String>>(&raw_args) {
            Ok(args) => (args, None),
            Err(error) => (Vec::new(), Some(format!(
                "ENGRAM_MCP_ARGS_JSON must be a JSON array of strings: {error}"
            ))),
        };
        Self {
            command: Arc::new(command),
            args: Arc::new(args),
            args_error: Arc::new(args_error),
            session: Arc::new(Mutex::new(None)),
        }
    }

    /// Search all Engram projects and return at most two title summaries.
    /// Failures are returned to the caller, which keeps web results intact.
    pub async fn search(&self, query: &str) -> Result<Vec<Value>, String> {
        if let Some(error) = self.args_error.as_ref() {
            return Err(error.clone());
        }

        let mut state = self.session.lock().await;
        if state.is_none() {
            *state = Some(self.start().await?);
        }
        let result = state.as_mut().expect("session initialized").search(query).await;
        if result.is_err() {
            // A timed-out or broken stdio stream must not poison future calls.
            state.take();
        }
        result
    }

    async fn start(&self) -> Result<EngramSession, String> {
        let mut child = Command::new(self.command.as_str())
            .args(self.args.iter())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("could not start Engram MCP: {error}"))?;
        let stdin = child.stdin.take().ok_or("Engram MCP stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("Engram MCP stdout unavailable")?;
        let mut session = EngramSession {
            _child: child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            next_id: 1,
        };

        let init = session.request("initialize", json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "advent-999-search-mcp", "version": env!("CARGO_PKG_VERSION") }
        })).await?;
        if let Some(error) = init.get("error") {
            return Err(format!("Engram MCP initialize failed: {error}"));
        }
        session.notify_initialized().await?;
        Ok(session)
    }
}

impl EngramSession {
    async fn search(&mut self, query: &str) -> Result<Vec<Value>, String> {
        let response = self.request("tools/call", json!({
            "name": "mem_search",
            "arguments": {
                "query": query,
                "limit": 2,
                "match_mode": "any",
                "all_projects": true,
                "response_format": "compact"
            }
        })).await?;
        let result = response.get("result").ok_or("Engram MCP search returned no result")?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(extract_text(result).unwrap_or_else(|| "Engram MCP search failed".to_string()));
        }
        let structured = result.get("structuredContent").cloned();
        if let Some(items) = structured.as_ref()
            .and_then(|value| value.get("results"))
            .and_then(Value::as_array)
        {
            return Ok(items.iter().take(2).map(compact_memory).collect());
        }
        let text = extract_text(result).ok_or("Engram MCP search returned no text")?;
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|_| "Engram MCP search returned an unexpected response format".to_string())?;
        let items = parsed.get("results").and_then(Value::as_array)
            .ok_or("Engram MCP search response did not contain a results array")?;
        Ok(items.iter().take(2).map(compact_memory).collect())
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut encoded = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        encoded.push(b'\n');

        timeout(REQUEST_TIMEOUT, async {
            self.stdin.write_all(&encoded).await
                .map_err(|e| format!("Engram MCP write failed: {e}"))?;
            self.stdin.flush().await
                .map_err(|e| format!("Engram MCP flush failed: {e}"))?;
            loop {
                let line = self.stdout.next_line().await
                    .map_err(|e| format!("Engram MCP read failed: {e}"))?
                    .ok_or("Engram MCP closed its stdio stream")?;
                let message: Value = serde_json::from_str(&line)
                    .map_err(|e| format!("Engram MCP sent invalid JSON-RPC: {e}"))?;
                if message.get("id") == Some(&json!(id)) {
                    return Ok(message);
                }
                // Ignore notifications and server requests; this client only
                // needs the response that matches its outstanding request.
            }
        }).await.map_err(|_| "Engram MCP request timed out".to_string())?
    }

    async fn notify_initialized(&mut self) -> Result<(), String> {
        let mut encoded = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        })).map_err(|e| e.to_string())?;
        encoded.push(b'\n');
        self.stdin.write_all(&encoded).await
            .map_err(|e| format!("Engram MCP initialization notification failed: {e}"))?;
        self.stdin.flush().await
            .map_err(|e| format!("Engram MCP flush failed: {e}"))
    }
}

fn extract_text(result: &Value) -> Option<String> {
    result.get("content")?.as_array()?.iter()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("text"))?
        .get("text")?.as_str().map(str::to_string)
}

fn compact_memory(memory: &Value) -> Value {
    let id = memory.get("id").and_then(Value::as_u64)
        .map(|id| format!("#{id} "))
        .unwrap_or_default();
    let title = memory.get("title").and_then(Value::as_str).unwrap_or("(untitled)");
    Value::String(format!("{id}{title}"))
}

