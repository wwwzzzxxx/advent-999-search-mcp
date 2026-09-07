# AGENTS.md — advent-999-search-mcp

## 项目概览

Rust 编写的 MCP 服务器，提供三类工具：
- **`web`** — 网络搜索（10 个引擎：exa, bing, csdn, juejin, startpage, sogou, weixin, dblp, cnki + 凭据门控的 deepseek）
- **`local`** — 本地文件搜索（Everything / es.exe，未配置时工具不出现）
- **`get_page`** — 抓取网页正文（自动回退策略，支持需要登录 cookie 的站点；支持 find 服务端子串搜索模式，只返回匹配上下文窗口以省 token）
- get_page 的 find 模式在 `src/main.rs::handle_fetch` 实现：先正常抓取再做字面子串匹配（默认 ASCII 大小写不敏感）→ 合并重叠窗口 → 只返回 matches 上下文（带绝对字符偏移），忽略 startChar/endChar/maxLength，totalMatches 始终为全量命中数
- get_page 的 YouTube 字幕在 `src/fetch.rs::fetch_youtube_subtitle` 实现：watch 页取 INNERTUBE_API_KEY → Innertube player API（ANDROID 20.10.38，WEB 会被 POT/BotGuard 拦）→ captionTracks 选轨（手动英文 > 手动任意 > 自动英文 > 首个）→ timedtext json3 转 SRT；强制走代理（不可进 DIRECT_DOMAINS），匿名即可；非视频页（channel/playlist）回落通用抓取

协议：stdio + JSON-RPC 2.0，逐行从 stdin 读入。技术栈：Rust 2021、tokio、reqwest 0.12（json/socks/cookies/gzip/brotli/deflate）、scraper、zip 2.x、aes、zhihu_sign（知乎 x-zse-96 请求签名，SM4）。

## 常用命令

```powershell
# 构建（必须先有 python-embed.zip，否则编译失败）
./scripts/prepare_python_embed.ps1   # 生成 python-embed.zip（gitignored，~12MB）
cargo build --release                # 产物 target/release/advent-999-search-mcp.exe (~17MB)

# 发布双平台（GitHub gh CLI + Gitee API v5）
./scripts/publish.ps1
```

> ⚠️ 运行中的 MCP server 会锁定 exe（编译会报 `拒绝访问 os error 5`）。改代码后的验证流程见下文「调试 → 编译与替换 exe 的硬性流程」：**先 debug 构建到 `target\debug\` 实测功能，全部通过后再停掉客户端、`cargo build --release` 替换**。

## 源码结构

| 文件 | 职责 |
|------|------|
| `src/main.rs` | MCP 入口，JSON-RPC 消息循环，工具分发 |
| `src/config.rs` | 全部配置均来自环境变量；`DEFAULT_DIRECT_DOMAINS` 默认国内直连域名表。完整环境变量清单见 [README.md 第 3 节](README.md)。门控要点：`EXA_API_KEY` 开 exa、`DEEPSEEK_API_KEY` 开 deepseek、`EVERYTHING_ES_PATH` 开 local 工具（未设置则 local 工具不注册）、`DEEPSEEK_MODEL` 覆盖模型名 |
| `src/models.rs` | `SearchResult`（含 `summary: Option<String>`，序列化时跳过 None）、`SearchError` 等 |
| `src/engines/mod.rs` | `SearchEngine` trait、`create_engine_map()`、`credential_gated_engines()` |
| `src/engines/*.rs` | 各引擎实现 |
| `src/fetch.rs` | HTTP 抓取与正文提取：代理/直连分流、cookie、超时。**~2300 行大文件**，含 ElementRef→markdown 转换管线、zhihu 签名抓取（zhihu_sign）、反爬回退策略。改动前务必通读相关段落，勿盲目重写。定位入口：`fetch_url` 按域名分流——zhihu → `fetch_zhihu`（需 `FETCH_COOKIES` + `d_c0` 签名）、weixin → `fetch_weixin_article`、sogou 微信跳转 → `fetch_sogou_weixin_link`、YouTube 视频（watch/shorts/live/embed/youtu.be）→ `fetch_youtube_subtitle`（Innertube ANDROID client，自建强制走代理的 reqwest client，**不可**用 `build_fetch_client` 因为 youtube 绝不能进 DIRECT_DOMAINS；youtube-nocookie.com 无视频 id，直接回落通用抓取）、bilibili 视频 → `fetch_bilibili_subtitle`（`bilibili_api_get` 浏览器头过 WAF，`finger/spi` 补 buvid3；SESSDATA 缺失时字幕列表为空属正常）、GitHub issue/PR（`github.com/{owner}/{repo}/issues|pull/{n}`）→ 页面抓取后追加 `fetch_github_comments`（公开 REST API 免鉴权 60 req/h，评论是客户端渲染的，best-effort 失败不影响正文）、其他 → `fetch_direct` 失败/低质量再 `fetch_via_jina` |
| `src/local_search.rs` | Everything 本地搜索（es.exe）——Windows 专属；Linux/macOS 上无 es.exe，`EVERYTHING_ES_PATH` 未设置时 local 工具自动不注册 |
| `src/python_embed.rs` | 内嵌 Python 运行时（**Windows**：解压 `python-embed.zip` 到 `%LOCALAPPDATA%`；**Linux/macOS**：直接返回系统 `python3`，需装 requests，不嵌入 zip） |
| `scripts/prepare_python_embed.ps1` | 下载 Python 3.12 embeddable + 安装 requests，重打包为 `python-embed.zip` |
| `scripts/publish.ps1` | 构建 → 打 tag → 推送 → 双平台 Release + 附件上传 |

## 核心机制（务必先理解再动手）

### 1. 凭据只读环境变量 — 硬性规则
所有 API key 只允许从**用户环境变量**读取（如 `DEEPSEEK_API_KEY`、`EXA_API_KEY`）。
**禁止**：硬编码 key、从任何文件读取（含 `~/.local/share/opencode/auth.json`）、把 key 提交进 git。
凭据型引擎无 key 时自动禁用（不注册、不可调用）。

### 2. 代理分流
- `PROXY_URL` + `USE_PROXY=true`：默认走代理
- `DIRECT_DOMAINS`：命中子串的域名**绕过代理直连**（国内站点如 sogou/weixin/baidu/zhihu 会封代理 IP 弹验证码）；默认值见 `config.rs` 的 `DEFAULT_DIRECT_DOMAINS`；设为 `none` 则全部走代理

### 3. 引擎两步启用（用户侧易漏）
引擎要可用需同时满足：① 加入 `ALLOWED_SEARCH_ENGINES` 白名单；② 凭据型引擎已配置 key。改引擎可用性时两个都要检查。

### 4. deepseek 引擎双后端
- **official**：DeepSeek 官方 API（`api.deepseek.com/anthropic/v1/messages`，国内直连，已加入默认直连域）
- **go**：OpenCode Go 订阅（`opencode.ai/zen/go/v1/messages`），**必须走内嵌 Python** 调用——opencode.ai 在 Cloudflare 后面，检测 TLS 指纹，reqwest/curl 被 500 拦截，Python urllib3 可通过
- 后端按 key 格式自动识别：`sk-`+32hex（35 字符）= official；`sk-`+64 字符（67 字符）= go；`DEEPSEEK_API_MODE=official|go` 可强制

### 5. 内嵌 Python（跨平台）
`python-embed.zip` 通过 `include_bytes!` 编译进 **Windows** exe；首次运行解压到 `%LOCALAPPDATA%\advent-mcp-python`，`.version` marker 控制版本（改 zip 后必须 bump `PYTHON_EMBED_VERSION`）。go 后端用 `spawn_blocking` 启动 `python.exe`，脚本以 stdin 传 query、从 `os.environ` 读 key，输出 JSON。
**Linux/macOS**：`python_exe()` 返回系统 `python3`（每次调用前探测 `import requests`，失败给出安装提示），脚本相同；Windows 专有符号全部 `#[cfg(target_os = "windows")]` 隔离，Linux 二进制不嵌入 zip（体积 ~7MB vs Windows ~18MB）。

### 6. Linux 构建与运行要点（WSL 实测 2026-08-16）
- 依赖：`apt install pkg-config libssl-dev`；target 目录建议放 WSL 内（如 `CARGO_TARGET_DIR=/home/user/advent-target`），避免 /mnt/c 9P 文件系统拖慢编译
- `local` 工具在 Linux 自动隐藏（无 Everything）；Jina fallback（r.jina.ai）在无代理时被 DNS 污染不可达，**必须配 `PROXY_URL` 才能用 get_page 的 Jina 兜底**（普通直连抓取不受影响）
- dblp 引擎：服务器对 `Accept-Encoding: gzip, deflate, br` 返回 500，dblp.rs 已显式覆盖为 `gzip`（Windows 同样受益）

## 添加新引擎的工作流

1. `src/engines/new.rs`：实现 `SearchEngine` trait（`name()` + `search()`）
2. `src/engines/mod.rs`：`pub mod` 注册；需凭据则加入 `credential_gated_engines()`，否则加入 `create_engine_map()` 基础列表
3. `src/config.rs`：新增环境变量字段（`Option<String>`，无 key 禁用）
4. `src/models.rs`：`SearchResult` 新增字段时给所有构造点补上（当前 12 处，均带 `summary: None`）
5. 更新 README.md / README.zh-CN.md 的引擎列表与环境变量说明
6. 提醒用户在自己的 MCP 配置（mcp.json / opencode.json）中把新引擎名加入 `ALLOWED_SEARCH_ENGINES`

## 调试

- 启动日志走 stderr（引擎列表、代理/直连域、deepseek 模式识别都会打印）
- 手动测试：`echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"web","arguments":{"query":"test"}}}' | target\release\advent-999-search-mcp.exe`
- 注意区分引擎无结果（正常）与引擎报错（`SearchError`），两种都算"搜索完成"
- **无自动化测试**（无 tests/ 目录、无 `#[test]`）：所有验证都靠手工 JSON-RPC 喂 stdin + stderr 日志，改代码后必须按下方流程实测

### 编译与替换 exe 的硬性流程（变更代码后必须遵守）

> ⚠️ **运行中的 MCP server 会锁定 exe**（VS Code / opencode / 其他 agent 都在用），
> 直接 `cargo build --release` 会因 `拒绝访问 (os error 5)` 失败，且正在使用该 MCP 的 agent 会话会被中断。

1. **编译到别的文件夹**：先 `cargo check` 或 `cargo build`（debug，输出 `target\debug\`，不碰 release exe）验证编译；
   debug 二进制可直接用上面手动测试命令指向 `target\debug\advent-999-search-mcp.exe` 实测功能（JSON-RPC 喂 stdin，看 stderr 日志）
2. **全部功能验证通过后**，才允许杀进程替换：
   - 先确认使用方（VS Code 重载窗口 / 重启 opencode）
   - 再 `cargo build --release` 生成新 exe
3. 替换时告知用户"旧进程即将被替换"，避免中断正在进行的 agent 会话

## 禁止事项

- ❌ 硬编码 / 文件读取 / 提交任何 API key、cookie、token
- ❌ 删除或 gitignore 变更 `python-embed.zip`（构建必需，已 gitignore）
- ❌ 随意改 `DEFAULT_DIRECT_DOMAINS` 默认值（影响所有国内用户的反验证码策略）
- ❌ 未经测试验证直接替换正在运行的 MCP 二进制（exe 进程经常有 agent 程序在使用；会中断用户上下文，成本高）
- ❌ 提交 `vendor/` 变更（`Everything.ini` / `es.exe` / Setup.exe 含本机配置，目前未 gitignore，建议加入 .gitignore 或移出仓库）
- ❌ 编译测试时直接 `cargo build --release` 覆盖运行中的 exe（会失败并可能中断会话）——先用 debug 构建在 `target\debug\` 验证，通过后再替换

## 发布流程（publish.ps1 已封装）

1. 更新 `Cargo.toml` 版本号 + `src/main.rs` serverInfo
2. 构建 release，确认二进制字节数（版本以 `Cargo.toml` 为准，当前 v0.4.1；字节数以实际构建为准）
3. `./scripts/publish.ps1`：push 双 remote（GitHub `wwwzzzxxx` + Gitee `pzwzx`）→ 两平台创建/更新 Release → 上传附件
4. 验证两平台附件字节数一致（Gitee API 需要 `target_commitish` 字段）
