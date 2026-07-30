# advent-999-search-mcp 🔍

基于 Rust 的 MCP（Model Context Protocol）服务器，为 AI 助手提供**网络搜索**、**本地文件搜索**和**网页抓取**能力。

## 功能特性

- **`web`** — 多引擎网络搜索（Exa、Bing、CSDN、掘金、Startpage、搜狗）
- **`local`** — 通过 Everything (voidtools) ES.exe 进行本地文件搜索
- **`get_page`** — 网页内容抓取，支持 HTML 到 Markdown 转换

## 快速开始

### 前置依赖

- [Rust](https://rustup.rs/)（edition 2021）
- 本地搜索需安装 [Everything](https://www.voidtools.com/)

### 安装

```bash
git clone https://github.com/wwwzzzxxx/advent-999-search-mcp.git
cd advent-999-search-mcp
cargo build --release
```

编译后的二进制文件位于 `target/release/advent-999-search-mcp.exe`。

### Everything 配置（本地搜索）

本地文件搜索需要 Everything 的命令行工具 `es.exe`。启用方法：

1. 安装 [Everything](https://www.voidtools.com/)
2. 从 [ES 发布页面](https://github.com/voidtools/ES/releases) 下载 `es.exe`
3. 设置环境变量 `EVERYTHING_ES_PATH`，指向 `es.exe` 的路径

若未设置此环境变量，`local` 工具将不会显示。

## MCP 配置

### VS Code（`mcp.json`）

添加到你的 `.vscode/mcp.json` 或用户级别的 MCP 配置：

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

### OpenCode（`opencode.json`）

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

## 环境变量

| 变量 | 默认值 | 说明 |
|----------|---------|------|
| `DEFAULT_SEARCH_ENGINE` | `exa` | 默认搜索引擎 |
| `ALLOWED_SEARCH_ENGINES` | *（全部）* | 允许的搜索引擎列表（逗号分隔） |
| `PROXY_URL` | — | 代理地址（如 `http://127.0.0.1:7890`） |
| `USE_PROXY` | `true` | 是否启用代理 |
| `EXA_API_KEY` | — | Exa API 密钥，用于更高频率限制（[申请](https://dashboard.exa.ai/api-keys)） |
| `FETCH_COOKIES` | — | 浏览器 Cookie，用于抓取需要登录的页面（如知乎） |
| `FETCH_TIMEOUT` | `30` | 抓取超时时间（秒） |
| `EVERYTHING_ES_PATH` | — | ES.exe 的自定义路径 |

## 搜索引擎

| 引擎 | 类型 | 需要 API Key | 备注 |
|--------|------|:-----------:|------|
| Exa | MCP API | 可选（每月免费 $10） | 默认引擎 |
| Bing | 爬取 | 否 | cn.bing.com |
| CSDN | API | 否 | so.csdn.net |
| 掘金 | API | 否 | juejin.cn |
| Startpage | 爬取 | 否 | 注重隐私 |
| 搜狗 | 爬取 | 否 | sogou.com |

## 工具说明

### `web` — 网络搜索
```
query: string（必填）     — 搜索关键词
limit: number（默认 10）  — 最大结果数（1-50）
engines: string[]         — 使用的搜索引擎
searchMode: string        — "request" | "auto" | "playwright"
```

### `local` — 本地文件搜索
```
query: string（必填）      — Everything 查询语法
maxResults: number（1-1000）— 最大结果数
sort: string               — 排序字段
path: string               — 限定搜索目录
contentSearch: boolean     — 搜索文件内容
... 更多选项
```

### `get_page` — 网页抓取
```
url: string（必填）         — 要抓取的 URL
maxLength: number（最大 200k）— 最大内容长度
```

支持：知乎（签名 API）、CSDN、Discourse 论坛、对 JS 密集型站点自动降级到 Jina。

## 许可证

MIT
