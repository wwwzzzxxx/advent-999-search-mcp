# advent-999-search-mcp 🔍

> [中文文档](README.zh-CN.md)

A Rust-based MCP (Model Context Protocol) server that provides **web search**, **local file search**, and **web page fetching** capabilities for AI assistants.

## Features

- **`web`** — Multi-engine web search (Exa, Bing, CSDN, Juejin, Startpage, Sogou)
- **`local`** — Local filesystem search via Everything (voidtools) ES.exe
- **`get_page`** — Web page content extraction with HTML-to-Markdown conversion

## Quick Start

### Prerequisites

- [Rust](https://rustup.rs/) (edition 2021)
- For local search: [Everything](https://www.voidtools.com/) by voidtools

### Install

```bash
git clone https://github.com/wwwzzzxxx/advent-999-search-mcp.git
cd advent-999-search-mcp
cargo build --release
```

The binary will be at `target/release/advent-999-search-mcp.exe`.

### Everything Setup (Local Search)

Local file search uses Everything's command-line tool `es.exe`. Bundled files:

- `vendor/Everything-1.5.0.1418b.x64-Setup.exe` — Everything installer
- `vendor/es.exe` — Command-line search tool

To enable local search:

1. Run `vendor/Everything-1.5.0.1418b.x64-Setup.exe` to install Everything, or download the latest from [voidtools.com](https://www.voidtools.com/)
2. Set `EVERYTHING_ES_PATH` environment variable to point to `es.exe` (the bundled one at `vendor/es.exe`, or download the latest from [ES releases](https://github.com/voidtools/ES/releases))

If this env var is not set, the `local` tool will not be available.

## MCP Configuration

### VS Code (`mcp.json`)

Add to your `.vscode/mcp.json` or user-level MCP config:

```json
{
  "servers": {
    "advent": {
      "type": "stdio",
      "command": "path/to/advent-999-search-mcp.exe",
      "args": [],
      "env": {
        "DEFAULT_SEARCH_ENGINE": "exa",
        "PROXY_URL": "http://127.0.0.1:7890",
        "USE_PROXY": "true"
      }
    }
  }
}
```

### OpenCode (`opencode.json`)

```json
{
  "mcp": {
    "advent": {
      "command": ["path/to/advent-999-search-mcp.exe"],
      "enabled": true,
      "type": "local"
    }
  }
}
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DEFAULT_SEARCH_ENGINE` | `exa` | Default search engine |
| `ALLOWED_SEARCH_ENGINES` | *(all)* | Comma-separated list of allowed engines |
| `PROXY_URL` | — | Proxy URL (e.g. `http://127.0.0.1:7890`) |
| `USE_PROXY` | `true` | Enable/disable proxy |
| `EXA_API_KEY` | — | Exa API key for higher rate limits ([get one](https://dashboard.exa.ai/api-keys)) |
| `FETCH_COOKIES` | — | Browser cookies for authenticated page fetching (e.g. zhihu.com) |
| `FETCH_TIMEOUT` | `30` | Fetch timeout in seconds |
| `EVERYTHING_ES_PATH` | — | Custom path to ES.exe |

## Search Engines

| Engine | Type | Requires API Key | Notes |
|--------|------|:---------------:|-------|
| Exa | MCP API | Optional (free $10/month) | Default engine |
| Bing | Scraping | No | cn.bing.com |
| CSDN | API | No | so.csdn.net |
| Juejin | API | No | juejin.cn |
| Startpage | Scraping | No | Privacy-focused |
| Sogou | Scraping | No | sogou.com |

## Tools

### `web` — Web Search
```
query: string (required)      — Search query
limit: number (default: 10)   — Max results (1-50)
engines: string[]             — Engines to use
searchMode: string            — "request" | "auto" | "playwright"
```

### `local` — Local File Search
```
query: string (required)      — Everything query syntax
maxResults: number (1-1000)   — Max results
sort: string                  — Sort field
path: string                  — Limit to directory
contentSearch: boolean        — Search file contents
... more options
```

### `get_page` — Fetch Web Page
```
url: string (required)        — URL to fetch
maxLength: number (max 200k)  — Max content length
```

Supports: Zhihu (signed API), CSDN, Discourse forums, Jina fallback for JS-heavy sites.

## License

MIT
