# advent-999-search-mcp 🔍

> [中文文档](README.zh-CN.md)

A Rust-based MCP server that gives your AI assistant **web search**, **local file search**, and **web page fetching** abilities.

- **`web`** — Search with Exa, Bing, CSDN, Juejin, Startpage, Sogou
- **`local`** — Search your local files via Everything (voidtools)
- **`get_page`** — Fetch and extract readable content from any web page

### Why advent?

- 🪶 **Extremely low memory** — Written in Rust, ~5 MB binary, minimal runtime footprint
- ⚙️ **Minimal configuration** — Works out of the box with sensible defaults, no heavy dependencies
- 🌐 **Smart `get_page`** — Fetches and renders content from almost any site: handles JS-rendered pages, authenticated pages (Zhihu, Discourse), and complex HTML, all with automatic fallback strategies

---

> **💡 Tip**: You can ask your AI agent to follow these installation steps for you.

## Installation

### 1. Get the binary

**Option A — Download** (recommended): Get the latest `.exe` from the [Releases page](https://github.com/wwwzzzxxx/advent-999-search-mcp/releases).

**Option B — Build from source**:

```bash
git clone https://github.com/wwwzzzxxx/advent-999-search-mcp.git
cd advent-999-search-mcp
cargo build --release
# Binary at: target/release/advent-999-search-mcp.exe
```

### 2. (Optional) Install Everything for local file search

If you want local file search, install Everything by voidtools:

- Run `vendor/Everything-1.5.0.1418b.x64-Setup.exe` (bundled in this repo)
- Or download from [voidtools.com](https://www.voidtools.com/)

After installation, copy `vendor/Everything.ini` over your existing Everything.ini to apply recommended settings that prevent excessive memory usage.

The config file location varies by installation. To find yours, open Everything and type `about:config` in the search bar, then press Enter — it will open the file directly.

Then set the `EVERYTHING_ES_PATH` environment variable to point to `es.exe` (the bundled one at `vendor/es.exe`, or download the latest from [ES releases](https://github.com/voidtools/ES/releases)).

> Skip this step if you don't need local search — the `local` tool will not appear when this env var is unset.

#### Recommended Everything Settings

Content indexing in Everything 1.5 can consume **several GB of RAM** if left unrestricted. The bundled `vendor/Everything.ini` limits content indexing to small text/code files only (under 20 KB), which keeps memory usage minimal. Key settings:

- `content_indexing_include_only_files` — Only index code and text files (no binaries, images, videos)
- `content_indexing_max_size=20` (KB) — Skip files larger than 20 KB
- `content_indexing_exclude_recall_on_data_access=1` — Skip cloud-only files

You can adjust these settings in Everything under **Tools → Options → Advanced**.

### 3. Configure environment variables

Set these in your OS user environment (or pass them via MCP config `env`):

```bash
# Required for proxy (recommended)
PROXY_URL=http://127.0.0.1:7890
USE_PROXY=true

# Optional — Exa API key (free $10/month at https://dashboard.exa.ai/api-keys)
EXA_API_KEY=your_key_here

# Optional — browser cookies for fetching authenticated pages (e.g. zhihu.com)
# Same format as the HTTP Cookie header:
FETCH_COOKIES="d_c0=ABC...; z_c0=DEF...; SESSIONID=GHI..."

# Optional — for local file search (see step 2)
EVERYTHING_ES_PATH=C:\path\to\es.exe
```

### 4. Register as an MCP server

#### VS Code

Edit `%APPDATA%\Code\User\mcp.json` (all projects) or `.vscode\mcp.json` (current project):

```json
{
  "servers": {
    "advent": {
      "type": "stdio",
      "command": "C:\\path\\to\\advent-999-search-mcp.exe",
      "args": [],
      "env": {
        "PROXY_URL": "http://127.0.0.1:7890",
        "USE_PROXY": "true",
        "EXA_API_KEY": "your_key_here",
        "FETCH_COOKIES": "d_c0=ABC...; z_c0=DEF..."
      }
    }
  }
}
```

#### OpenCode

Edit `opencode.json`:

```json
{
  "mcp": {
    "advent": {
      "command": ["C:\\path\\to\\advent-999-search-mcp.exe"],
      "enabled": true,
      "env": {
        "PROXY_URL": "http://127.0.0.1:7890",
        "USE_PROXY": "true",
        "EXA_API_KEY": "your_key_here",
        "FETCH_COOKIES": "d_c0=ABC...; z_c0=DEF..."
      },
      "type": "local"
    }
  }
}
```

---

## Environment Variables Reference

| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `PROXY_URL` | No | — | Proxy address (e.g. `http://127.0.0.1:7890`) |
| `USE_PROXY` | No | `true` | Enable/disable proxy |
| `EXA_API_KEY` | No | — | Exa API key — get one [here](https://dashboard.exa.ai/api-keys) |
| `FETCH_COOKIES` | No | — | Browser cookies for authenticated pages. Format: `key=value; key2=value2` |
| `EVERYTHING_ES_PATH` | No | — | Path to ES.exe (required to enable local search) |
| `DEFAULT_SEARCH_ENGINE` | No | `exa` | Default search engine |
| `ALLOWED_SEARCH_ENGINES` | No | *(all)* | Comma-separated list of allowed engines |
| `FETCH_TIMEOUT` | No | `30` | Fetch timeout in seconds |

---

## Tools

### `web` — Web Search

```
query      (string, required)  — Search query
limit      (number, default 10) — Results per engine (1-50)
engines    (string[])           — Which engine(s) to use
searchMode (string)             — "request" | "auto" | "playwright"
```

Supported engines: Exa (default), Bing, CSDN, Juejin, Startpage, Sogou.

### `local` — Local File Search

```
query         (string, required) — Everything query syntax
maxResults    (number, 1-1000)   — Max results
sort          (string)           — Sort field
path          (string)           — Limit to directory
contentSearch (boolean)          — Search file contents
matchCase     (boolean)          — Case-sensitive
matchRegex    (boolean)          — Regex mode
filesOnly     (boolean)          — Files only
foldersOnly   (boolean)          — Folders only
```

### `get_page` — Fetch Web Page

```
url       (string, required)    — URL to fetch
maxLength (number, max 200000)  — Max content length
```

Smart fetching strategy:
- **Zhihu** → signed API (requires `d_c0` in `FETCH_COOKIES`)
- **CSDN** → direct HTML extraction
- **Discourse** forums → JSON API
- **JS-heavy sites** → Jina Reader fallback
- **Normal sites** → direct HTTP with HTML-to-Markdown

---

## Acknowledgements

- **[Aas-ee/open-webSearch](https://github.com/aas-ee/open-websearch)** — The original Node.js multi-engine search MCP that this project was inspired by.
- **[code-yeongyu/oh-my-openagent](https://github.com/code-yeongyu/oh-my-openagent)** — Referenced for the Exa MCP integration pattern.
- **[voidtools](https://www.voidtools.com/)** — Everything and ES.exe, the backbone of local file search.
- **[Exa](https://exa.ai/)** — AI-powered web search API.

## License

MIT
