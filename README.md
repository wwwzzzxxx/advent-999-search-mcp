# advent-999-search-mcp 🔍

> [中文文档](README.zh-CN.md)

[![GitHub](https://img.shields.io/badge/GitHub-wwwzzzxxx%2Fadvent--999--search--mcp-181717?logo=github&style=flat)](https://github.com/wwwzzzxxx/advent-999-search-mcp)
[![Gitee](https://img.shields.io/badge/Gitee-pzwzx%2Fadvent--999--search--mcp-C71D23?logo=gitee&style=flat)](https://gitee.com/pzwzx/advent-999-search-mcp)
[![Release](https://img.shields.io/github/v/release/wwwzzzxxx/advent-999-search-mcp?label=Release)](https://github.com/wwwzzzxxx/advent-999-search-mcp/releases)

A Rust-based MCP server that gives your AI assistant **web search**, **local file search**, and **web page fetching** abilities.

- **`web`** — Search with 11 engines (Exa, Bing, CSDN, Juejin, Startpage, Sogou, Weixin, DBLP, CNKI + credential-gated DeepSeek, IEEE), with **per-request filters** (`freshness`, `topic`, `includeDomains`, `excludeDomains`), **cross-engine de-duplication**, and **several topics per call** (`queries`)
- **`local`** — Search your local files via Everything (voidtools)
- **`get_page`** — Fetch and extract readable content from any web page, with `find` (literal) and `highlights` (query-relevance) modes, and up to 10 URLs per call
- **`set_cookies`** — A local, **domain-scoped** cookie store: set / list / clear login and anti-bot cookies at runtime, no restart needed
- **`import_cookies`** — Opt-in (`ADVENT_ALLOW_COOKIE_IMPORT=1`): read cookies from a dedicated Edge profile over the DevTools protocol

### Why advent?

- 🪶 **Extremely low memory** — Written in Rust. Single binary: ~18 MB on Windows (includes embedded Python runtime), ~7 MB on Linux/macOS (uses system Python 3). Minimal runtime footprint
- ⚙️ **Minimal configuration** — Works out of the box with sensible defaults, no heavy dependencies
- 🌐 **Smart `get_page`** — Fetches and renders content from almost any site: handles JS-rendered pages, authenticated pages (Zhihu, Discourse), and complex HTML, all with automatic fallback strategies
- 🔎 **Honest about what it can't do** — Engines differ in whether they support a time filter, so the response reports exactly which engine applied it (`filters.freshnessByEngine`) instead of silently pretending the filter worked

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

# Optional — IEEE Xplore Metadata Search API key (enables the `ieee` engine).
# Get one at https://developer.ieee.org (needs IEEE activation during US
# business hours before first use). The key is read ONLY from this env var.
IEEE_API_KEY=your_key_here

# Optional — restrict which engines are available (comma-separated).
# IMPORTANT: if you set this, include every engine you want — including
# deepseek once its key is configured:
ALLOWED_SEARCH_ENGINES=exa,bing,csdn,juejin,startpage,sogou,weixin,dblp,cnki,deepseek,ieee

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
| `DIRECT_DOMAINS` | No | *mainland list* | Domains that bypass the proxy (direct). Default: `sogou,weixin,baidu,bilibili,hdslb,zhihu,csdn,juejin,xiaohongshu,cnki,dblp`. Set `none` to proxy everything. NOTE: YouTube always uses the proxy regardless of this setting (it needs egress outside mainland China) |
| `EXA_API_KEY` | No | — | Exa API key — get one [here](https://dashboard.exa.ai/api-keys) |
| `DEEPSEEK_API_KEY` | No | — | DeepSeek web-search key — official API key or OpenCode Go subscription key (auto-detected) — enables the `deepseek` engine |
| `DEEPSEEK_API_MODE` | No | *(auto)* | Force backend: `official` (api.deepseek.com) or `go` (opencode.ai) |
| `DEEPSEEK_MODEL` | No | `deepseek-v4-flash` | Model used by the deepseek engine |
| `IEEE_API_KEY` | No | — | IEEE Xplore Metadata Search API key ([developer.ieee.org](https://developer.ieee.org)) — enables the `ieee` engine |
| `FETCH_COOKIES` | No | — | Legacy way to supply long-lived **login** cookies as a flat `key=value; key2=value2` blob. It is now split by cookie owner: `d_c0`/`z_c0` → `zhihu.com`, `SESSDATA`/`buvid3`/`bili_jct`/… → `bilibili.com`, `SNUID`/`SUV` → `sogou.com`; **unrecognized names are dropped** (they would otherwise be sent to every fetched host). Prefer `set_cookies` / `import_cookies`, which store cookies per domain. **Never commit real cookies** |
| `COOKIE_CACHE_TTL` | No | `1200` | Age (seconds) after which a cached cookie is logged as stale. Advisory only — stale cookies are still used, the remote server is the real authority. `0` disables the warning |
| `ADVENT_ALLOW_COOKIE_IMPORT` | No | `0` | Set to `1` to register the `import_cookies` tool. Off by default: it is a credential-access capability and should be approved separately |
| `ADVENT_FETCH_LOG` | No | `0` | Set to `1` to append one JSON line per `get_page` fetch (URL, host, outcome, status, chars, duration, strategy, attached cookie names). Values are never logged |
| `ADVENT_FETCH_LOG_PATH` | No | `<cookie dir>/fetch-log.jsonl` | Where the fetch log is written (rotated once at 8 MB) |
| `EVERYTHING_ES_PATH` | No | — | Path to ES.exe (required to enable local search) |
| `DEFAULT_SEARCH_ENGINE` | No | `exa` | Default search engine |
| `ALLOWED_SEARCH_ENGINES` | No | *(all)* | Comma-separated list of allowed engines |
| `FETCH_TIMEOUT` | No | `30` | Fetch timeout in seconds |

---

## Tools

### `web` — Web Search

```
query          (string)             — Search query (single topic)
queries        (string[], 1-10)     — Several topics in one call (same options apply to all)
limit          (number, default 10) — Max results per query (1-50)
engines        (string[])           — Which engine(s) to use
searchMode     (string)             — "request" | "auto" | "playwright"
freshness      (string)             — day | week | month | year | YYYY-MM-DD..YYYY-MM-DD
topic          (string)             — "news" | "general"
includeDomains (string[])           — only these domains (and subdomains)
excludeDomains (string[])           — drop these domains (and subdomains)
dedupe         (boolean, default true) — fold duplicate URLs across engines
```

Engines run **concurrently**, and their results are merged into one list:
duplicate URLs are folded together (the survivor lists every engine that
returned it, in `engines`), and results agreed on by more engines rank first.
Per-engine relevance order is preserved within equal agreement.

#### `freshness` — and what each engine can actually do

Engines differ a lot here, and some silently ignore a filter. Rather than
pretend, the response reports the truth per engine in
`filters.freshnessByEngine`:

| Engine | Tier | Mechanism |
|---|---|---|
| `exa` | `applied` | `startPublishedDate` / `endPublishedDate` |
| `sogou` | `applied` | `tsn` (1/2/3/4 = day/week/month/year) |
| `csdn` | `applied` | `tm` (1/2/3/5 = day/week/month/year) |
| `ieee` | `applied` | `start_date` / `end_date` (`YYYYMMDD`) |
| `juejin` | `best_effort` | newest-first sort (`sort_type=1`), not a true range |
| `cnki` | `best_effort` | already sorted by publication time |
| `dblp` | `best_effort` | `year:YYYY` query term (year granularity only) |
| `bing` | `best_effort` | `filters=ex1:"ezN"` sent, but **no effect observed** from a mainland egress |
| `startpage` | `unsupported` | — |
| `weixin` | `unsupported` | the `tsn` parameter returns a rejection page |

`applied` means verified to change the result set. `best_effort` means a
parameter was sent or a newest-first sort was used, but it is not guaranteed to
restrict results. `unsupported` means the engine has no time filter at all.

> The window is computed in **UTC**. An explicit range
> (`2026-01-01..2026-02-01`) is passed through verbatim.

`topic` is currently honoured only by `exa` (`category: news`); the response
lists which engines applied it in `filters.topicAppliedBy`.

`includeDomains` / `excludeDomains` are passed to `exa` natively (better recall)
and applied as a post-filter to every other engine's results.

#### De-duplication

`dedupe` (default on) folds **exact duplicate URLs** together — the same page
returned by several engines, or repeated across an engine's pages. As a side
effect it strips tracking parameters, so a CSDN URL loses its ~300 characters
of `ops_request_misc`/`request_id`/`utm_*` cruft.

Only parameters known to be tracking are removed. A meaningful parameter such
as bilibili's `?p=2` (multi-part video) is preserved, so genuinely different
pages are never merged. The count of folded duplicates is reported as
`duplicatesRemoved`.

**Not** de-duplicated: the same article reposted on a different domain. That
requires fetching each page's body, which would turn the fast search path into
a slow one, and title-based heuristics merge genuinely different pages.

Supported engines (11 total, all tested): Exa (default), Bing, CSDN, Juejin, Startpage, Sogou, Weixin (WeChat articles), DBLP (CS bibliography), CNKI (知网), DeepSeek (LLM-backed, requires key), IEEE (Xplore metadata, requires key).

Credential-gated engines (implemented, hidden until key set — appear automatically in `tools/list` when the env var is present):
- **`deepseek`** — LLM-backed web search via the Anthropic-compatible Messages API with the server-side `web_search_20250305` tool. Two backends auto-detected from key format:
  - **official** — DeepSeek official API key (`sk-`+32 hex) → `api.deepseek.com` (direct, no proxy needed)
  - **go** — OpenCode Go subscription key (`sk-`+64 chars) → `opencode.ai` via proxy. Runs via Python (embedded runtime on Windows, system `python3` on Linux/macOS — see build note). Returns an AI summary attached to the first result (`summary` field).
- **`ieee`** — IEEE Xplore Metadata Search API (`ieeexploreapi.ieee.org`, `IEEE_API_KEY` required). Returns title, authors, venue, year, content type, citation count, DOI and abstract snippet per record.

When the key is set via the env var, the engine is automatically listed in `tools/list`; without the key it stays hidden.

Note: `cnki` needs no cookie or key — it uses the public scholar.cnki.net REST API. CNKI hosts sit behind Tencent EdgeOne, which answers HTTP 418 to proxy/datacenter IPs, so `cnki` must bypass the proxy (present in the default `DIRECT_DOMAINS`).

Note: `dblp` queries `dblp.org` and falls back to `dblp.uni-trier.de`. Both are protected by [Anubis](https://anubis.techaro.lol) proof-of-work anti-bot; the engine solves the challenge automatically (see below).

Note: `startpage` proxies Google results. Sogou-style blocks aside, Startpage answers automated clients with an Anubis challenge on `/sp/search`; the engine solves it automatically.

### Anubis proof-of-work (dblp, startpage)

Both sites sit behind [Anubis](https://anubis.techaro.lol), which replies to non-browser clients with a JavaScript challenge page instead of content. The server solves it the same way a browser would (`src/anubis.rs`):

1. read `randomData`, `id` and `difficulty` from `<script id="anubis_challenge">`
2. brute-force a `nonce` so that `sha256hex(randomData + nonce)` starts with `difficulty` zero **hex digits**
3. `GET /.within.website/x/cmd/anubis/api/pass-challenge` — this sets the auth cookie
4. replay the original request; the cookie is cached in the client's jar

The hash search runs multi-threaded; `difficulty` counts hex digits (not bits), so difficulty 4 takes ~15 ms and difficulty 6 ~1 s. The auth cookie lasts about a week, so the cost is paid once per process rather than per search.

Note: `sogou` (and `weixin`, which rides on weixin.sogou.com) rate-limits by egress IP. Once Sogou decides your IP is suspicious it serves a "请依次点击" click-captcha instead of results. **To recover**, solve it once in a browser and store the resulting cookies locally:

1. open `https://www.sogou.com/web?query=test`, solve the captcha
2. copy the cookies: in DevTools → Console run `copy(document.cookie)`
3. seed the cache — either paste into the `set_cookies` MCP tool, or run
   `python tests/manual/seed_cookies.py sogou --stdin`
4. search again — **no restart needed**

`SNUID` is the credential that proves the captcha was solved; it is *not* HttpOnly, so `document.cookie` is enough. It lasts roughly 20 minutes, after which Sogou challenges again. The cache warns when an entry looks stale.

### How cookies are stored

Everything lives in one file, keyed by **domain**:

```jsonc
// %LOCALAPPDATA%\advent-mcp\cookie-cache.json
{ "version": 2, "entries": {
    "sogou.com":    { "cookies": "SNUID=...; SUV=...", "saved_at": 1789000000, "source": "paste" },
    "bilibili.com": { "cookies": "SESSDATA=...",        "saved_at": 1789000000, "source": "browser" } } }
```

A request only ever receives the cookies of **its own host** (suffix match), so
a Bilibili session can never leak to an unrelated site. The file lives
**outside this repository** and must never be committed. Logs and
`set_cookies action=list` only show names/counts, never values.

Three ways in:

| Method | For | Refresh |
|---|---|---|
| `set_cookies` (tool) | pasting a `document.cookie` dump or a specific cookie | next request, no restart |
| `import_cookies` (tool, opt-in) | pulling cookies from a dedicated Edge profile automatically | next request, no restart |
| `FETCH_COOKIES` (env var) | legacy; flat blob split by cookie owner | edit config **and restart** |

`import_cookies` does **not** touch your everyday Edge profile. It launches
`msedge.exe` with its own `--user-data-dir` (`%LOCALAPPDATA%\advent-mcp\edge-profile`)
and asks the browser for the requested domains over the DevTools protocol —
the only supported route on current Windows builds, where cookie values are
protected by App-Bound Encryption (v20) and the live cookie DB is not readable
by other processes. Log in once in the window it opens; the profile keeps the
session for next time. Enable it with `ADVENT_ALLOW_COOKIE_IMPORT=1`.

Note: `cnki`, `dblp` and `sogou` all detect proxy/datacenter IPs, so they belong in `DIRECT_DOMAINS` (they are there by default). Sogou's block is IP-level, and note that in a rules-based VPN the proxy may still route CN domains direct — so "proxying" Sogou does not actually change your egress IP.

### Batching (`queries`)

`web` takes either `query` (one topic) or `queries` (1-10 topics). With
`queries` the server fans out (query × engine) concurrently with per-family
throttling (sogou/weixin share one permit and serialize; dblp/startpage are
capped; stateless engines run free up to a global cap) and returns one object
per query, in input order. An invalid item (bad `freshness`, empty query) is
reported as `{query, error}` and does not lose the other topics.

> The throttle is process-wide, so parallel `web` calls in one process are
> equally safe — batching mainly saves round-trips (and is faster on clients
> that execute tool calls one at a time).

Each item may also be an object (`{"query": ..., "limit": ...}`) to override
the top-level options for that topic.

### `set_cookies` — Domain-scoped cookie store

```
action (string) — "set" (default) | "list" | "clear"
domain (string) — owning domain, e.g. bilibili.com / zhihu.com / sogou.com
                  (short aliases sogou, weixin, bilibili, zhihu work too)
cookies (string) — Cookie header value, e.g. "SESSDATA=...; buvid3=..."
```

Cookies are filtered for `sogou.com` (only the names its trust check needs),
stored for any other domain as pasted (capped at 50 names / 8 KB), and sent
only to that domain and its subdomains. `action=list` shows domains, cookie
names and ages; `action=clear` with `domain:"*"` forgets everything.

### `import_cookies` — Opt-in browser import

```
domains     (string[], 1-5, required) — e.g. ["bilibili.com"]
loginUrl    (string)   — page to open (default https://<domain>/)
waitSeconds (number, default 90, max 300) — how long to wait for the login
keepOpen    (boolean, default false) — leave the window open afterwards
```

Registered only when the server starts with `ADVENT_ALLOW_COOKIE_IMPORT=1`.
Probes the private profile headlessly (no window); opens Edge only when a login
is actually missing, then stores the cookies per domain. Each domain reports
`usable` / `missingKey`, so a zhihu import that only got `d_c0` says it still
needs `z_c0`. Never reads your everyday browser profile.

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
url       (string)              — URL to fetch, or an arXiv paper ID
urls      (string[], 1-10)      — Several URLs/IDs in one call (fetched concurrently)
maxLength (number, max 200000)  — Max content length per URL (with several URLs the default
                                  is a ~60k-character total budget, split evenly)
startChar (number)              — Start reading from this character offset (0-based)
endChar   (number)              — Read up to this offset (exclusive)
find      (string)              — Literal substring to search in the fetched text (find mode)
contextChars (number, default 200) — Context chars around each match / highlight
maxMatches (number, default 20, max 50) — Max context windows in find mode
matchCase (boolean, default false) — Case-sensitive matching (ASCII only)
highlights (string)             — Query to rank passages against (highlights mode)
maxHighlights (number, default 5, max 20) — Max snippets in highlights mode
includeLinks (boolean, default false) — Keep markdown hyperlinks [text](url); by default links are stripped to their visible text to save tokens
```

**Highlights mode** (`highlights`) is the token-cheap way to read a long page:
instead of returning the whole document, the server ranks passages against your
query and returns only the best ones. It is local and deterministic (no LLM
call) and ranks by **distinct-term coverage first** — a passage containing
several different query terms beats one repeating a single term. ASCII words
and CJK bigrams are matched, so Chinese works without a segmenter.

```jsonc
// get_page { "url": "...", "highlights": "create task", "maxHighlights": 3 }
{
  "highlights": "create task",
  "totalLength": 47010,
  "totalSnippets": 3,
  "snippets": [
    { "startChar": 5441, "endChar": 7977, "score": 2044,
      "terms": ["create", "task"], "context": "..." }
  ]
}
```

Offsets are character-based and half-open, so a snippet can be re-read exactly
with `startChar`/`endChar`. `startChar`/`endChar`/`maxLength`/`find` are ignored
in highlights mode.

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
- **Zhihu** → signed API: question pages are expanded with their top 5 answers. Requires a **logged-in** session stored for `zhihu.com` — `d_c0` alone now gets `403 need_login`, so `z_c0` is needed too (use `import_cookies` to log in once, then import)
- **CSDN** → direct HTML extraction
- **Discourse** forums → JSON API
- **WeChat articles** (`mp.weixin.qq.com`) → MicroMessenger UA direct fetch
- **Sogou WeChat links** (`weixin.sogou.com/link?url=...`) → JS-URL resolution + fetch
- **GitHub issues/PRs** → page body + all comments appended via the public REST API (comments are client-side rendered on the HTML page, so they are fetched from `api.github.com` and appended as markdown)
- **Bilibili videos** (`bilibili.com/video/BVxxx`, optional `?p=N` for multi-part) → subtitle track returned as SRT (requires `SESSDATA` stored for `bilibili.com` — Bilibili only serves the subtitle list to logged-in users)
- **YouTube videos** (`youtube.com/watch?v=ID`, `/shorts/ID`, `/live/ID`, `/embed/ID`, `youtu.be/ID`) → caption track returned as SRT via the Innertube ANDROID player API (anonymous, no login; needs proxy egress outside mainland China). Track priority: manual English > manual any language > auto English > first available. Non-video pages (channel, playlist, search) fall through to the generic fetch path
- **arXiv papers** → accepts a paper ID (`2401.12345`, `arXiv:2401.12345`, or an `arxiv.org/abs/...` URL) and returns the paper's HTML content (official `arxiv.org/html/` conversion, falling back to ar5iv for older papers)
- **JS-heavy sites** → Jina Reader fallback
- **Normal sites** → direct HTTP with HTML-to-Markdown

### Fetch log

Set `ADVENT_FETCH_LOG=1` to append one JSON object per fetch to
`%LOCALAPPDATA%\advent-mcp\fetch-log.jsonl` (override with
`ADVENT_FETCH_LOG_PATH`). It records the URL, host, ok/failure, HTTP status,
extracted and returned character counts, duration, which strategy produced the
content (`direct`, `jina`, `zhihu-api`, ...) and the **names** of the stored
cookies that were attached. Cookie values and response bodies are never logged.

```jsonc
{"ts":"2026-10-20T11:37:05Z","url":"https://www.bing.com/search?q=rust","host":"www.bing.com",
 "site":"Bing","mode":"content","ok":true,"status":200,"totalChars":17264,"returnedChars":17264,
 "truncated":false,"ms":692,"via":"direct","cookies":19,"cookieNames":["MUID","SRCHD","..."],
 "batch":1,"index":1,"retried":false}
```

---

## Acknowledgements

- **[Aas-ee/open-webSearch](https://github.com/aas-ee/open-websearch)** — The original Node.js multi-engine search MCP that this project was inspired by.
- **[code-yeongyu/oh-my-openagent](https://github.com/code-yeongyu/oh-my-openagent)** — Referenced for the Exa MCP integration pattern.
- **[voidtools](https://www.voidtools.com/)** — Everything and ES.exe, the backbone of local file search.
- **[Exa](https://exa.ai/)** — AI-powered web search API.

## License

MIT
