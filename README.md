# advent-999-search-mcp 🔍

> [中文文档](README.zh-CN.md)

[![GitHub](https://img.shields.io/badge/GitHub-wwwzzzxxx%2Fadvent--999--search--mcp-181717?logo=github&style=flat)](https://github.com/wwwzzzxxx/advent-999-search-mcp)
[![Gitee](https://img.shields.io/badge/Gitee-pzwzx%2Fadvent--999--search--mcp-C71D23?logo=gitee&style=flat)](https://gitee.com/pzwzx/advent-999-search-mcp)
[![Release](https://img.shields.io/github/v/release/wwwzzzxxx/advent-999-search-mcp?label=Release)](https://github.com/wwwzzzxxx/advent-999-search-mcp/releases)

A Rust-based MCP server that gives your AI assistant **web search**, **local file search**, and **web page fetching** abilities.

- **`web`** — Search with 10 engines (Exa, Bing, CSDN, Juejin, Startpage, Sogou, Weixin, DBLP, CNKI + credential-gated DeepSeek)
- **`local`** — Search your local files via Everything (voidtools)
- **`get_page`** — Fetch and extract readable content from any web page

### Why advent?

- 🪶 **Extremely low memory** — Written in Rust. Single binary: ~18 MB on Windows (includes embedded Python runtime), ~7 MB on Linux/macOS (uses system Python 3). Minimal runtime footprint
- ⚙️ **Minimal configuration** — Works out of the box with sensible defaults, no heavy dependencies
- 🌐 **Smart `get_page`** — Fetches and renders content from almost any site: handles JS-rendered pages, authenticated pages (Zhihu, Discourse), and complex HTML, all with automatic fallback strategies

---

> **💡 Tip**: You can ask your AI agent to follow these installation steps for you.

## Installation

### 1. Get the binary

**Option A — Download** (recommended): Get the binary for your platform from the [Releases page](https://github.com/wwwzzzxxx/advent-999-search-mcp/releases):
- `advent-999-search-mcp.exe` — Windows
- `advent-999-search-mcp-linux` — Linux (make it executable with `chmod +x`)

**Option B — Build from source**:

```bash
git clone https://github.com/wwwzzzxxx/advent-999-search-mcp.git
cd advent-999-search-mcp

# Windows only (Step 1, required): build the embedded Python runtime zip
# (downloads Python 3.12 embeddable + installs requests, ~12 MB, gitignored)
./scripts/prepare_python_embed.ps1

# Step 2: build
cargo build --release
# Binary at: target/release/advent-999-search-mcp.exe (~18 MB)
```

**Linux/macOS build**: no embedded Python zip is needed (the binary does not embed it). Install build deps and build normally:

```bash
sudo apt install pkg-config libssl-dev   # Debian/Ubuntu; adjust for your distro
cargo build --release
# Binary at: target/release/advent-999-search-mcp (~7 MB)
```

> **Why the embedded Python?** The `deepseek` engine's OpenCode Go backend calls
> `opencode.ai`, which sits behind Cloudflare TLS-fingerprint detection — it
> blocks reqwest/curl fingerprints but passes Python's urllib3/OpenSSL one.
> **Windows**: the embedded runtime (auto-extracted to `%LOCALAPPDATA%\advent-mcp-python`
> on first use) makes this work with zero external Python installation.
> **Linux/macOS**: the binary uses the **system `python3`** instead — just make sure
> Python 3 with the `requests` library is installed (`apt install python3-requests`
> or `pip install requests`). The go backend is disabled with an install hint if missing.

### 2. (Optional) Install Everything for local file search

> **Windows only** — Everything has no Linux/macOS version; the `local` tool is automatically hidden on those platforms (when `EVERYTHING_ES_PATH` is unset).

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

# Optional — domains that must bypass the proxy (direct connection)
# Defaults to mainland-China domains (sogou, weixin, baidu, ...) which block
# proxy/datacenter IPs with captchas. Set to `none` to proxy everything
# (recommended for users outside mainland China), or provide a custom list:
DIRECT_DOMAINS=sogou,weixin,baidu

# Optional — Exa API key (free $10/month at https://dashboard.exa.ai/api-keys)
EXA_API_KEY=your_key_here

# Optional — DeepSeek web-search key (enables the `deepseek` engine).
# Accepts EITHER a DeepSeek official API key (sk- + 32 hex, 35 chars,
# https://platform.deepseek.com) OR an OpenCode Go subscription key
# (sk- + 64 chars). The backend is auto-detected from the key format;
# override with DEEPSEEK_API_MODE=official|go. The key is read ONLY
# from this env var (never from any file). Model override: DEEPSEEK_MODEL
# (default deepseek-v4-flash).
DEEPSEEK_API_KEY=your_key_here
DEEPSEEK_API_MODE=official   # optional: force backend
DEEPSEEK_MODEL=deepseek-v4-flash  # optional: override model

# Optional — restrict which engines are available (comma-separated).
# IMPORTANT: if you set this, include every engine you want — including
# deepseek once its key is configured:
ALLOWED_SEARCH_ENGINES=exa,bing,csdn,juejin,startpage,sogou,weixin,dblp,cnki,deepseek

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
| `DIRECT_DOMAINS` | No | *mainland list* | Domains that bypass the proxy (direct). Default: `sogou,weixin,baidu,bilibili,hdslb,zhihu,csdn,juejin,xiaohongshu`. Set `none` to proxy everything. NOTE: YouTube always uses the proxy regardless of this setting (it needs egress outside mainland China) |
| `EXA_API_KEY` | No | — | Exa API key — get one [here](https://dashboard.exa.ai/api-keys) |
| `DEEPSEEK_API_KEY` | No | — | DeepSeek web-search key — official API key or OpenCode Go subscription key (auto-detected) — enables the `deepseek` engine |
| `DEEPSEEK_API_MODE` | No | *(auto)* | Force backend: `official` (api.deepseek.com) or `go` (opencode.ai) |
| `DEEPSEEK_MODEL` | No | `deepseek-v4-flash` | Model used by the deepseek engine |
| `FETCH_COOKIES` | No | — | Browser cookies for authenticated pages. Format: `key=value; key2=value2`. `d_c0` unlocks Zhihu; `SESSDATA` unlocks Bilibili subtitles (the subtitle list requires login) |
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

Supported engines (10 total, all tested): Exa (default), Bing, CSDN, Juejin, Startpage, Sogou, Weixin (WeChat articles), DBLP (CS bibliography), CNKI (知网), DeepSeek (LLM-backed, requires key).

Credential-gated engines (implemented, hidden until key set — appear automatically in `tools/list` when the env var is present):
- **`deepseek`** — LLM-backed web search via the Anthropic-compatible Messages API with the server-side `web_search_20250305` tool. Two backends auto-detected from key format:
  - **official** — DeepSeek official API key (`sk-`+32 hex) → `api.deepseek.com` (direct, no proxy needed)
  - **go** — OpenCode Go subscription key (`sk-`+64 chars) → `opencode.ai` via proxy. Runs via Python (embedded runtime on Windows, system `python3` on Linux/macOS — see build note). Returns an AI summary attached to the first result (`summary` field).

When the key is set via the env var, the engine is automatically listed in `tools/list`; without the key it stays hidden.

Note: `cnki` needs no cookie or key — it uses the public scholar.cnki.net REST API.

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
url       (string, required)    — URL to fetch, or an arXiv paper ID
maxLength (number, max 200000)  — Max content length
startChar (number)              — Start reading from this character offset (0-based)
endChar   (number)              — Read up to this offset (exclusive)
find      (string)              — Literal substring to search in the fetched text (find mode)
contextChars (number, default 200) — Context chars around each match in find mode
maxMatches (number, default 20, max 50) — Max context windows in find mode
matchCase (boolean, default false) — Case-sensitive matching (ASCII only)
```

Find mode (server-side substring search, saves tokens): when `find` is set, the fetch happens
normally, then the server scans the extracted text for the literal substring and returns only
merged context windows around matches — no full content is returned, and
`startChar`/`endChar`/`maxLength` are ignored. Each match reports absolute character offsets
(`startChar`/`endChar` of the window and of each hit inside) so you can follow up with a
targeted `startChar`/`endChar` read of the full text. Overlapping windows are merged to avoid
duplicate output. `totalMatches` always reports the full hit count even when `maxMatches`
truncates the window list. Empty needle is rejected; zero hits return a hint suggesting
different keywords.

Smart fetching strategy:
- **Zhihu** → signed API (requires `d_c0` in `FETCH_COOKIES`)
- **CSDN** → direct HTML extraction
- **Discourse** forums → JSON API
- **WeChat articles** (`mp.weixin.qq.com`) → MicroMessenger UA direct fetch
- **Sogou WeChat links** (`weixin.sogou.com/link?url=...`) → JS-URL resolution + fetch
- **GitHub issues/PRs** → page body + all comments appended via the public REST API (comments are client-side rendered on the HTML page, so they are fetched from `api.github.com` and appended as markdown)
- **Bilibili videos** (`bilibili.com/video/BVxxx`, optional `?p=N` for multi-part) → subtitle track returned as SRT (requires `SESSDATA` in `FETCH_COOKIES` — Bilibili only serves the subtitle list to logged-in users)
- **YouTube videos** (`youtube.com/watch?v=ID`, `/shorts/ID`, `/live/ID`, `/embed/ID`, `youtu.be/ID`) → caption track returned as SRT via the Innertube ANDROID player API (anonymous, no login; needs proxy egress outside mainland China). Track priority: manual English > manual any language > auto English > first available. Non-video pages (channel, playlist, search) fall through to the generic fetch path
- **arXiv papers** → accepts a paper ID (`2401.12345`, `arXiv:2401.12345`, or an `arxiv.org/abs/...` URL) and returns the paper's HTML content (official `arxiv.org/html/` conversion, falling back to ar5iv for older papers)
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
