# advent-999-search-mcp 🔍

基于 Rust 的 MCP 服务器，为你的 AI 助手提供**网络搜索**、**本地文件搜索**和**网页抓取**能力。

- **`web`** — 多引擎搜索（Exa、Bing、CSDN、掘金、Startpage、搜狗）
- **`local`** — 通过 Everything (voidtools) 搜索本地文件
- **`get_page`** — 抓取任意网页并提取可读内容

### 为什么选 advent？

- 🪶 **极低内存占用** — 使用 Rust 编写，二进制仅 ~5 MB，运行时内存占用极小
- ⚙️ **轻量化配置** — 开箱即用，无需繁琐依赖
- 🌐 **智能 `get_page`** — 能抓取并渲染几乎所有网站：支持 JS 渲染页面、需登录页面（知乎、Discourse）、复杂 HTML，自动降级策略保证成功率

---

> **💡 提示**：你可以直接让你的 AI 助手按以下步骤帮你完成安装。

## 安装指南

### 1. 获取可执行文件

**方式一：直接下载**（推荐）：从 [Releases 页面](https://github.com/wwwzzzxxx/advent-999-search-mcp/releases) 下载最新的 `.exe`。

**方式二：从源码编译**：

```bash
git clone https://github.com/wwwzzzxxx/advent-999-search-mcp.git
cd advent-999-search-mcp
cargo build --release
# 编译产物：target/release/advent-999-search-mcp.exe
```

### 2. （可选）安装 Everything 以启用本地搜索

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
| `DIRECT_DOMAINS` | 否 | *大陆列表* | 绕过代理直连的域名。默认：`sogou,weixin,baidu,bilibili,zhihu,csdn,juejin,xiaohongshu`。设为 `none` 则全部走代理 |
| `EXA_API_KEY` | 否 | — | Exa API 密钥，[点此申请](https://dashboard.exa.ai/api-keys) |
| `IEEE_API_KEY` | 否 | — | IEEE Xplore API 密钥（[developer.ieee.org](https://developer.ieee.org)），设置后启用 `ieee` 引擎 |
| `SEMANTIC_SCHOLAR_API_KEY` | 否 | — | Semantic Scholar API 密钥（[semanticscholar.org/product/api](https://www.semanticscholar.org/product/api)），设置后启用 `semantic_scholar` 引擎 |
| `FETCH_COOKIES` | 否 | — | 浏览器 Cookie。格式：`key=value; key2=value2` |
| `EVERYTHING_ES_PATH` | 否 | — | ES.exe 路径（必须设置才能启用本地搜索） |
| `DEFAULT_SEARCH_ENGINE` | 否 | `exa` | 默认搜索引擎 |
| `ALLOWED_SEARCH_ENGINES` | 否 | *（全部）* | 允许的搜索引擎列表（逗号分隔） |
| `FETCH_TIMEOUT` | 否 | `30` | 抓取超时（秒） |

---

## 工具说明

### `web` — 网络搜索

```
query      (string, 必填)      — 搜索关键词
limit      (number, 默认 10)   — 每引擎结果数（1-50）
engines    (string[])           — 使用的搜索引擎
searchMode (string)             — "request" | "auto" | "playwright"
```

支持的搜索引擎（9 个，均已实测）：Exa（默认）、Bing、CSDN、掘金、Startpage、搜狗、微信（公众号文章）、DBLP（计算机文献）、知网。

规划中的引擎（已实现但暂未暴露，等待 API 密钥）：
- **`ieee`** — IEEE Xplore 论文（需 [developer.ieee.org](https://developer.ieee.org) 申请的 `IEEE_API_KEY`，审核需数个工作日）
- **`semantic_scholar`** — 带引用数的论文搜索（需 [semanticscholar.org/product/api](https://www.semanticscholar.org/product/api) 的 `SEMANTIC_SCHOLAR_API_KEY`，注册后邮件发送）

通过环境变量设置密钥后，引擎会自动出现在 `tools/list` 中；未设置密钥时保持隐藏。

说明：`cnki`（知网）无需 Cookie 或密钥，直接使用公开的 scholar.cnki.net REST API。

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
```

智能抓取策略：
- **知乎** → 签名 API（需在 `FETCH_COOKIES` 中提供 `d_c0`）
- **CSDN** → 直接 HTML 提取
- **Discourse** 论坛 → JSON API
- **微信公众号**（`mp.weixin.qq.com`）→ 微信 UA 直抓
- **搜狗微信跳转**（`weixin.sogou.com/link?url=...`）→ JS 拼接 URL 解析 + 抓取
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
