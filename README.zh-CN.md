# advent-999-search-mcp 🔍

> [English Docs](README.md)

[![GitHub](https://img.shields.io/badge/GitHub-wwwzzzxxx%2Fadvent--999--search--mcp-181717?logo=github&style=flat)](https://github.com/wwwzzzxxx/advent-999-search-mcp)
[![Gitee](https://img.shields.io/badge/Gitee-pzwzx%2Fadvent--999--search--mcp-C71D23?logo=gitee&style=flat)](https://gitee.com/pzwzx/advent-999-search-mcp)
[![Release](https://img.shields.io/github/v/release/wwwzzzxxx/advent-999-search-mcp?label=Release)](https://github.com/wwwzzzxxx/advent-999-search-mcp/releases)

基于 Rust 的 MCP 服务器，为你的 AI 助手提供**网络搜索**、**本地文件搜索**和**网页抓取**能力。

- **`web`** — 11 个搜索引擎（Exa、Bing、CSDN、掘金、Startpage、搜狗、微信、DBLP、知网 + 凭据门控的 DeepSeek、IEEE），支持**按请求过滤**（`freshness`、`topic`、`includeDomains`、`excludeDomains`）与**跨引擎去重**
- **`local`** — 通过 Everything (voidtools) 搜索本地文件
- **`get_page`** — 抓取任意网页并提取可读内容，支持 `find`（字面子串）与 `highlights`（按查询相关性）两种模式
- **`set_cookies`** — 为有验证码墙的引擎（搜狗）写入新会话 Cookie，无需重启

### 为什么选 advent？

- 🪶 **极低内存占用** — 使用 Rust 编写，单文件二进制：Windows 约 18 MB（内置 Python 运行时），Linux/macOS 约 7 MB（使用系统 Python 3），运行时内存占用极小
- ⚙️ **轻量化配置** — 开箱即用，无需繁琐依赖
- 🌐 **智能 `get_page`** — 能抓取并渲染几乎所有网站：支持 JS 渲染页面、需登录页面（知乎、Discourse）、复杂 HTML，自动降级策略保证成功率
- 🔎 **对做不到的事说实话** — 各引擎对时间过滤的支持差异很大，响应会逐个引擎说明是否真的过滤了（`filters.freshnessByEngine`），而不是假装过滤成功

---

> **💡 提示**：你可以直接让你的 AI 助手按以下步骤帮你完成安装。

## 安装指南

### 1. 获取可执行文件

**方式一：直接下载**（推荐）：从 [Releases 页面](https://github.com/wwwzzzxxx/advent-999-search-mcp/releases) 下载对应平台的二进制：
- `advent-999-search-mcp.exe` — Windows
- `advent-999-search-mcp-linux` — Linux（下载后执行 `chmod +x` 赋予可执行权限）

**方式二：从源码编译**：

```bash
git clone https://github.com/wwwzzzxxx/advent-999-search-mcp.git
cd advent-999-search-mcp

# 仅 Windows（第 1 步，必需）：生成内置 Python 运行时 zip
# （下载 Python 3.12 embeddable + 安装 requests，约 12 MB，产物已被 gitignore）
./scripts/prepare_python_embed.ps1

# 第 2 步：编译
cargo build --release
# 编译产物：target/release/advent-999-search-mcp.exe（约 18 MB）
```

**Linux/macOS 编译**：无需内置 Python zip（二进制不会嵌入）。安装编译依赖后正常编译：

```bash
sudo apt install pkg-config libssl-dev   # Debian/Ubuntu，其他发行版请自行调整
cargo build --release
# 编译产物：target/release/advent-999-search-mcp（约 7 MB）
```

> **为什么内置 Python？** `deepseek` 引擎的 OpenCode Go 后端调用
> `opencode.ai`，该站有 Cloudflare TLS 指纹检测——会拦截 reqwest/curl 的
> 指纹，但放行 Python urllib3/OpenSSL 的指纹。
> **Windows**：内置运行时（首次使用自动解压到 `%LOCALAPPDATA%\advent-mcp-python`）
> 让这一切无需额外安装 Python 即可工作。
> **Linux/macOS**：二进制改用**系统 `python3`** —— 只需确保系统装有 Python 3 及
> `requests` 库（`apt install python3-requests` 或 `pip install requests`）。
> 若缺失，go 后端会报错并提示安装方法。

### 2. （可选）安装 Everything 以启用本地搜索

> **仅 Windows** — Everything 没有 Linux/macOS 版本；在这些平台上（未设置 `EVERYTHING_ES_PATH` 时）`local` 工具会自动隐藏。

如果需要本地文件搜索功能，请安装 voidtools 的 Everything：

- 运行 `vendor/Everything-1.5.0.1418b.x64-Setup.exe`（仓库已打包）
- 或从 [voidtools.com](https://www.voidtools.com/) 下载

安装后，用 `vendor/Everything.ini` 覆盖你本机的 Everything.ini 文件以应用推荐配置，避免内容索引占用过多内存。

配置文件位置因安装方式而异。如需查找本机位置，打开 Everything，在搜索框输入 `about:config` 后回车即可直接打开配置文件。

然后设置环境变量 `EVERYTHING_ES_PATH` 指向 `es.exe`（可用仓库自带的 `vendor/es.exe`，或从 [ES 发布页面](https://github.com/voidtools/ES/releases) 下载最新版）。

> 不需要本地搜索可跳过此步 — 未设置该环境变量时，`local` 工具不会出现。

#### Everything 推荐配置

Everything 1.5 的内容索引若不加以限制，可能占用**数 GB 内存**。仓库打包的 `vendor/Everything.ini` 将内容索引限制为仅索引 20KB 以下的小型文本/代码文件，极大降低内存占用。核心设置：

- `content_indexing_include_only_files` — 仅索引代码和文本文件（不含二进制、图片、视频）
- `content_indexing_max_size=20`（KB）— 跳过大于 20KB 的文件
- `content_indexing_exclude_recall_on_data_access=1` — 跳过云端按需同步文件

可在 Everything 的 **工具 → 选项 → 高级** 中调整这些设置。

### 3. 配置环境变量

以下变量可通过系统环境变量设置，或在 MCP 配置的 `env` 中传递：

```bash
# 代理设置（推荐配置）
PROXY_URL=http://127.0.0.1:7890
USE_PROXY=true

# （可选）必须绕过代理直连的域名
# 默认包含大陆站点（sogou、weixin、baidu 等），这些站点会拦截代理/机房 IP 并弹验证码。
# 大陆用户无需设置；海外用户建议设为 none 让所有请求走代理，或自定义列表：
DIRECT_DOMAINS=sogou,weixin,baidu

# （可选）Exa API 密钥，每月免费 $10
# 申请地址：https://dashboard.exa.ai/api-keys
EXA_API_KEY=你的密钥

# （可选）DeepSeek 搜索密钥（启用 deepseek 引擎）
# 支持两种 key 之一：DeepSeek 官方 API key（sk-+32 位十六进制，35 字符，
# 申请：https://platform.deepseek.com）或 OpenCode Go 订阅 key（sk-+64 字符）。
# 后端自动按 key 格式检测；也可用 DEEPSEEK_API_MODE=official|go 强制指定。
# key 只从该环境变量读取（绝不读任何文件）。
DEEPSEEK_API_KEY=你的密钥
DEEPSEEK_API_MODE=official   # 可选：强制后端
DEEPSEEK_MODEL=deepseek-v4-flash  # 可选：覆盖模型

# （可选）IEEE Xplore 元数据搜索 API 密钥（启用 ieee 引擎）
# 申请：https://developer.ieee.org（需 IEEE 在美东工作时间激活后方可使用）
# key 只从该环境变量读取（绝不读任何文件）
IEEE_API_KEY=你的密钥

# （可选）限制可用引擎（逗号分隔）
# 重要：如果设置了这个变量，务必把想用的引擎都写进去——
# 包括配置了 key 后的 deepseek：
ALLOWED_SEARCH_ENGINES=exa,bing,csdn,juejin,startpage,sogou,weixin,dblp,cnki,deepseek,ieee

# （可选）浏览器 Cookie，用于抓取需登录的页面（如知乎）
# 格式与 HTTP 的 Cookie 请求头相同：
FETCH_COOKIES="d_c0=ABC...; z_c0=DEF...; SESSIONID=GHI..."

# （可选）本地文件搜索（见第 2 步）
EVERYTHING_ES_PATH=C:\path\to\es.exe
```

### 4. 注册为 MCP 服务器

#### VS Code

编辑 `%APPDATA%\Code\User\mcp.json`（全局）或 `.vscode\mcp.json`（当前项目）：

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
        "EXA_API_KEY": "你的密钥",
        "FETCH_COOKIES": "d_c0=ABC...; z_c0=DEF..."
      }
    }
  }
}
```

#### OpenCode

编辑 `opencode.json`：

```json
{
  "mcp": {
    "advent": {
      "command": ["C:\\path\\to\\advent-999-search-mcp.exe"],
      "enabled": true,
      "env": {
        "PROXY_URL": "http://127.0.0.1:7890",
        "USE_PROXY": "true",
        "EXA_API_KEY": "你的密钥",
        "FETCH_COOKIES": "d_c0=ABC...; z_c0=DEF..."
      },
      "type": "local"
    }
  }
}
```

---

## 环境变量参考

| 变量 | 必填 | 默认值 | 说明 |
|----------|:---:|---------|------|
| `PROXY_URL` | 否 | — | 代理地址（如 `http://127.0.0.1:7890`） |
| `USE_PROXY` | 否 | `true` | 是否启用代理 |
| `DIRECT_DOMAINS` | 否 | *大陆列表* | 绕过代理直连的域名。默认：`sogou,weixin,baidu,bilibili,hdslb,zhihu,csdn,juejin,xiaohongshu,cnki,dblp`。设为 `none` 则全部走代理。注意：YouTube 始终走代理（需出境），不受此设置影响 |
| `EXA_API_KEY` | 否 | — | Exa API 密钥，[点此申请](https://dashboard.exa.ai/api-keys) |
| `DEEPSEEK_API_KEY` | 否 | — | DeepSeek 搜索密钥——官方 API key 或 OpenCode Go 订阅 key（自动检测）——启用 `deepseek` 引擎 |
| `DEEPSEEK_API_MODE` | 否 | 自动 | 强制后端：`official`（api.deepseek.com）或 `go`（opencode.ai） |
| `DEEPSEEK_MODEL` | 否 | `deepseek-v4-flash` | deepseek 引擎使用的模型 |
| `IEEE_API_KEY` | 否 | — | IEEE Xplore 元数据搜索 API 密钥（[developer.ieee.org](https://developer.ieee.org)）——启用 `ieee` 引擎 |
| `FETCH_COOKIES` | 否 | — | 长期**登录** Cookie。格式：`key=value; key2=value2`。`d_c0` 解锁知乎；`SESSDATA` 解锁 B 站字幕（字幕列表需登录才能获取）。短时**反爬** Cookie（搜狗 `SNUID`）请用 cookie 缓存 / `set_cookies` 工具——见下文搜狗说明。**切勿提交真实 Cookie**——请用环境变量配置 |
| `EVERYTHING_ES_PATH` | 否 | — | ES.exe 路径（必须设置才能启用本地搜索） |
| `DEFAULT_SEARCH_ENGINE` | 否 | `exa` | 默认搜索引擎 |
| `ALLOWED_SEARCH_ENGINES` | 否 | *（全部）* | 允许的搜索引擎列表（逗号分隔） |
| `FETCH_TIMEOUT` | 否 | `30` | 抓取超时（秒） |

---

## 工具说明

### `web` — 网络搜索

```
query          (string, 必填)      — 搜索关键词
limit          (number, 默认 10)   — 结果总数上限（1-50）
engines        (string[])           — 使用的搜索引擎
searchMode     (string)             — "request" | "auto" | "playwright"
freshness      (string)             — day | week | month | year | YYYY-MM-DD..YYYY-MM-DD
topic          (string)             — "news" | "general"
includeDomains (string[])           — 只返回这些域名（含子域名）
excludeDomains (string[])           — 排除这些域名（含子域名）
dedupe         (boolean, 默认 true) — 跨引擎合并重复 URL
```

各引擎**并发执行**，结果合并为一张列表：重复 URL 会被合并（保留结果的
`engines` 字段列出所有命中它的引擎），被更多引擎一致命中的结果排更前；在
命中数相同时保持各引擎自身的相关性顺序。

#### `freshness` —— 各引擎的真实能力

这一项各引擎差异极大，且部分引擎会**静默忽略**参数。因此响应不会假装过滤
成功，而是在 `filters.freshnessByEngine` 里逐个引擎如实汇报：

| 引擎 | 档位 | 机制 |
|---|---|---|
| `exa` | `applied` | `startPublishedDate` / `endPublishedDate` |
| `sogou` | `applied` | `tsn`（1/2/3/4 = 一天/一周/一月/一年） |
| `csdn` | `applied` | `tm`（1/2/3/5 = 一天/一周/一月/一年） |
| `ieee` | `applied` | `start_date` / `end_date`（`YYYYMMDD`） |
| `juejin` | `best_effort` | 按最新排序（`sort_type=1`），并非真区间过滤 |
| `cnki` | `best_effort` | 本身已按出版时间倒序 |
| `dblp` | `best_effort` | 查询里加 `year:YYYY`（仅到年粒度） |
| `bing` | `best_effort` | 会发送 `filters=ex1:"ezN"`，但**大陆出口实测无效果** |
| `startpage` | `unsupported` | 无 |
| `weixin` | `unsupported` | `tsn` 参数会返回拒绝页 |

`applied` = 已实测确认会改变结果集；`best_effort` = 发了参数或用最新排序，
但不保证真的收窄了结果；`unsupported` = 该引擎根本没有时间过滤。

> 时间窗口按 **UTC** 计算；显式区间（`2026-01-01..2026-02-01`）原样传递。

`topic` 目前仅 `exa` 支持（`category: news`），实际生效的引擎列在
`filters.topicAppliedBy`。

`includeDomains` / `excludeDomains` 会原生传给 `exa`（召回更好），并对其他
所有引擎的结果做后置过滤。

#### 去重

`dedupe`（默认开启）合并**URL 完全重复**的条目——同一页面被多个引擎返回，
或同一引擎翻页重复。顺带会清掉追踪参数，例如 CSDN 的 URL 会丢掉约 300 字符
的 `ops_request_misc`/`request_id`/`utm_*` 噪声。

只移除**已知的追踪参数**。有语义的参数（如 B 站多 P 视频的 `?p=2`）会保留，
因此不会把不同的页面误合并。被合并的条数以 `duplicatesRemoved` 返回。

**不做**的去重：同一篇文章转载到不同域名。这需要抓取每个页面正文，会把快速
搜索路径变成慢速路径，而且基于标题的启发式会误合并真正不同的页面。

支持的搜索引擎（11 个，均已实测）：Exa（默认）、Bing、CSDN、掘金、Startpage、搜狗、微信（公众号文章）、DBLP（计算机文献）、知网、DeepSeek（LLM 搜索，需密钥）、IEEE（Xplore 元数据，需密钥）。

凭据门控引擎（已实现，设置密钥前隐藏——配置环境变量后自动出现在 `tools/list`）：
- **`deepseek`** — 基于 LLM 的搜索，通过 Anthropic 兼容 Messages API 调用服务端 `web_search_20250305` 工具。按 key 格式自动检测后端：
  - **official** — DeepSeek 官方 API key（`sk-`+32 位十六进制）→ `api.deepseek.com`（国内直连，无需代理）
  - **go** — OpenCode Go 订阅 key（`sk-`+64 字符）→ `opencode.ai`（走代理）。通过 Python 调用（Windows 用内置运行时，Linux/macOS 用系统 `python3`，见编译说明）。返回的 AI 总结会附加到第一条结果的 `summary` 字段。
- **`ieee`** — IEEE Xplore 元数据搜索 API（`ieeexploreapi.ieee.org`，需 `IEEE_API_KEY`）。每条记录返回标题、作者、出版物、年份、内容类型、被引数、DOI 和摘要片段。

通过环境变量设置密钥后，引擎会自动出现在 `tools/list` 中；未设置密钥时保持隐藏。

说明：`cnki`（知网）无需 Cookie 或密钥，直接使用公开的 scholar.cnki.net REST API。知网各域名在腾讯 EdgeOne 后，对代理/数据中心 IP 一律返回 HTTP 418，因此 `cnki` 必须绕过代理直连（已在默认 `DIRECT_DOMAINS` 中）。

说明：`dblp` 先请求 `dblp.org`，失败时回退到 `dblp.uni-trier.de`。两者均已启用 [Anubis](https://anubis.techaro.lol) 工作量证明反爬，引擎会自动求解（见下文）。

说明：`startpage` 是 Google 结果的匿名代理。除了常规反爬，它会对自动化客户端的 `/sp/search` 返回 Anubis 挑战页，引擎会自动求解。

### Anubis 工作量证明（dblp、startpage）

这两个站点都在 [Anubis](https://anubis.techaro.lol) 之后，它对非浏览器客户端返回 JavaScript 挑战页而不是内容。服务端按与浏览器相同的方式求解（`src/anubis.rs`）：

1. 从 `<script id="anubis_challenge">` 读取 `randomData`、`id`、`difficulty`
2. 暴力搜索 `nonce`，使 `sha256hex(randomData + nonce)` 以 `difficulty` 个**前导零十六进制位**开头
3. `GET /.within.website/x/cmd/anubis/api/pass-challenge` —— 该响应会设置认证 Cookie
4. 重放原请求；Cookie 缓存在客户端的 cookie jar 中

哈希搜索是多线程的；`difficulty` 的单位是**十六进制位数**而非比特，所以 difficulty 4 约 15 毫秒、difficulty 6 约 1 秒。认证 Cookie 有效期约一周，因此每个进程只需付出一次求解成本，而不是每次搜索都算。

说明：`sogou`（以及依赖 weixin.sogou.com 的 `weixin`）按出口 IP 限流。一旦搜狗判定你的 IP 可疑，就会返回「请依次点击」点选验证码而不是结果。**恢复方法**——在浏览器里通过一次验证，然后把 Cookie 存到本地：

1. 打开 `https://www.sogou.com/web?query=test`，完成验证码
2. 复制 Cookie：DevTools → Console 执行 `copy(document.cookie)`
3. 写入缓存 —— 粘贴给 `set_cookies` MCP 工具，或运行
   `python tests/manual/seed_cookies.py sogou --stdin`
4. 重新搜索 —— **无需重启**

`SNUID` 是证明「验证码已通过」的凭证，**不是 HttpOnly**，所以 `document.cookie` 就能取到。有效期约 20 分钟，之后搜狗会再次要求验证。缓存过期时会在日志里给出提醒。

### 两种 Cookie 的分工

| | `FETCH_COOKIES`（环境变量） | Cookie 缓存（文件） |
|---|---|---|
| 用途 | 长期**登录**凭证 —— 知乎 `d_c0`、B 站 `SESSDATA` | 短时**反爬**凭证 —— 搜狗 `SNUID`/`SUV` |
| 加载时机 | 启动时读取一次 | 文件变更后立即重读 |
| 刷新方式 | 改配置**并重启** | 重新写入即可，下次搜索生效 |
| 位置 | 你的 MCP 配置 | `%LOCALAPPDATA%\advent-mcp\cookie-cache.json`（Windows）、`~/.cache/advent-mcp/cookie-cache.json`（Linux/macOS） |

两者都在**本仓库之外**，切勿提交。日志只打印不可逆的指纹（如 `SNUID(D04E…1AF4, len 32)`），永远不会输出可用的真实值。

说明：`cnki`、`dblp`、`sogou` 都会识别代理/数据中心 IP，因此它们属于应当直连的域名（默认已包含在 `DIRECT_DOMAINS` 中）。搜狗的封锁是 IP 级的；另外注意在规则模式下 VPN 可能仍把国内域名走直连，所以「用代理」并不会真的改变搜狗的出口 IP。

### `local` — 本地文件搜索

```
query         (string, 必填)   — Everything 查询语法
maxResults    (number, 1-1000) — 最大结果数
sort          (string)         — 排序字段
path          (string)         — 限定搜索目录
contentSearch (boolean)        — 搜索文件内容
matchCase     (boolean)        — 区分大小写
matchRegex    (boolean)        — 正则模式
filesOnly     (boolean)        — 仅文件
foldersOnly   (boolean)        — 仅文件夹
```

### `get_page` — 网页抓取

```
url       (string, 必填)       — 要抓取的 URL，或 arXiv 论文号
maxLength (number, 最大 200000) — 最大内容长度
startChar (number)              — 从该字符偏移开始读（0-based）
endChar   (number)              — 读到该偏移为止（不含）
find      (string)              — 在抓取文本中搜索的字面子串（find 模式）
contextChars (number, 默认 200) — find/highlights 模式每处上下文保留的字符数
maxMatches (number, 默认 20, 最大 50) — find 模式最多返回的上下文窗口数
matchCase (boolean, 默认 false) — 是否区分大小写（仅 ASCII）
highlights (string)             — 用于排序段落的查询（highlights 模式）
maxHighlights (number, 默认 5, 最大 20) — highlights 模式最多返回的片段数
includeLinks (boolean, 默认 false) — 保留 Markdown 超链接 [text](url)；默认剥离为可见文本以省 token
```

**highlights 模式**（`highlights`）是读取长页面最省 token 的方式：不返回全文，
而是按查询对段落排序，只返回最相关的若干段。纯本地、确定性（不调用 LLM），
排序**以命中的不同查询词数量优先**——一段覆盖多个不同查询词，胜过反复出现同
一个词。ASCII 按词匹配、中文按 bigram 匹配，因此中文无需分词器。

```jsonc
// get_page { "url": "...", "highlights": "训练 推理", "maxHighlights": 3 }
{
  "highlights": "训练 推理",
  "totalLength": 47010,
  "totalSnippets": 3,
  "snippets": [
    { "startChar": 5441, "endChar": 7977, "score": 2044,
      "terms": ["推理", "训练"], "context": "..." }
  ]
}
```

偏移是字符级、半开区间，可直接用 `startChar`/`endChar` 精确复读该片段。
highlights 模式下 `startChar`/`endChar`/`maxLength`/`find` 均被忽略。

find 模式（服务端子串搜索，省 token）：设置 `find` 后先正常抓取，再在提取文本中做字面子串
匹配，只返回匹配点附近合并后的上下文窗口——不返回全文，且忽略
`startChar`/`endChar`/`maxLength`。每处匹配都带绝对字符偏移（窗口和窗口内每个命中点的
`startChar`/`endChar`），可据此再用 `startChar`/`endChar` 精确读取全文对应段落。重叠窗口会合并
去重。即使 `maxMatches` 截断了窗口列表，`totalMatches` 仍报告全部命中数。空串会被拒绝；
零命中时返回 hint 建议换关键词。

智能抓取策略：
- **知乎** → 签名 API（需在 `FETCH_COOKIES` 中提供 `d_c0`）
- **CSDN** → 直接 HTML 提取
- **Discourse** 论坛 → JSON API
- **微信公众号**（`mp.weixin.qq.com`）→ 微信 UA 直抓
- **搜狗微信跳转**（`weixin.sogou.com/link?url=...`）→ JS 拼接 URL 解析 + 抓取
- **GitHub issue/PR** → 页面正文 + 全部评论（评论在 HTML 页面中是客户端动态渲染的，通过 `api.github.com` REST API 获取并以 Markdown 追加到正文后）
- **B 站视频**（`bilibili.com/video/BVxxx`，多 P 视频可用 `?p=N` 选集）→ 返回字幕轨的 SRT 格式文本（需在 `FETCH_COOKIES` 中提供 `SESSDATA`——B 站只向登录用户返回字幕列表）
- **YouTube 视频**（`youtube.com/watch?v=ID`、`shorts`/`live`/`embed`、`youtu.be/ID`）→ 经 Innertube ANDROID player API 返回字幕轨 SRT（匿名即可，无需登录；需可出境的代理）。选轨优先级：手动英文 > 手动任意语言 > 自动英文 > 首个。非视频页（频道、播放列表、搜索页）回落通用抓取
- **arXiv 论文** → 接受论文号（`2401.12345`、`arXiv:2401.12345` 或 `arxiv.org/abs/...` 链接），返回论文 HTML 内容（优先官方 `arxiv.org/html/` 转换版，老论文自动降级到 ar5iv）
- **JS 密集型站点** → 自动降级到 Jina Reader
- **普通站点** → 直接 HTTP + HTML 转 Markdown

---

## 致谢

- **[Aas-ee/open-webSearch](https://github.com/aas-ee/open-websearch)** — 原始的 Node.js 多引擎搜索 MCP，本项目的灵感来源。
- **[code-yeongyu/oh-my-openagent](https://github.com/code-yeongyu/oh-my-openagent)** — Exa MCP 集成模式的参考来源。
- **[voidtools](https://www.voidtools.com/)** — Everything 和 ES.exe，本地文件搜索的基石。
- **[Exa](https://exa.ai/)** — AI 驱动的网络搜索 API。

## 许可证

MIT
