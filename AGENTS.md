# AGENTS.md — advent-999-search-mcp

## 项目概览

Rust 编写的 MCP 服务器，提供三类工具：
- **`web`** — 网络搜索（11 个引擎：exa, bing, csdn, juejin, startpage, sogou, weixin, dblp, cnki + 凭据门控的 deepseek, ieee）。v0.9.0 起：引擎**并发执行**（`tokio::task::JoinSet`，引擎以 `Arc<dyn SearchEngine>` 持有）、跨引擎 URL 去重、按请求过滤（freshness/topic/includeDomains/excludeDomains）。过滤能力按 `FreshnessTier` 三档如实上报：applied=exa/sogou/csdn/ieee（已实测生效）、best_effort=juejin/cnki/dblp/bing（bing 实测大陆出口无效果，仍照发参数）、unsupported=startpage/weixin
- **`local`** — 本地文件搜索（Everything / es.exe，未配置时工具不出现）
- **`get_page`** — 抓取网页正文（自动回退策略，支持需要登录 cookie 的站点；支持 find 服务端子串搜索模式，只返回匹配上下文窗口以省 token；支持 highlights 模式，按查询相关性返回最相关的 N 个片段）
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
| `src/anubis.rs` | **Anubis 工作量证明反爬求解器**（dblp / startpage 都用）。`detect()` 从 `<script id="anubis_challenge">` 取 `randomData`/`id`/`difficulty`；`solve()` 多线程暴力找 nonce 使 `sha256hex(randomData + nonce)` 有 `difficulty` 个**前导零十六进制位**；`pass()` GET `/.within.website/x/cmd/anubis/api/pass-challenge` 拿 auth cookie（**client 必须 `cookie_store(true)`**）。cookie 名随站点前缀变化（dblp=`dblp_org-auth-*`、startpage=`spchal-auth`），靠 jar 自动处理，勿写死名字 |
| `src/cookie_cache.rs` | **短时反爬 Cookie 的本地缓存**（搜狗 SNUID 等）。文件在 `%LOCALAPPDATA%\advent-mcp\cookie-cache.json`（**仓库外**），按 mtime 判断变更 → **重新播种后下次搜索即生效，无需重启**。`filter_cookie_header()` 只保留引擎需要的 cookie；`fingerprint()` 输出不可逆指纹（`SNUID(D04E…1AF4, len 32)`）供日志使用，**永不打印真实值**。配套 CLI：`tests/manual/seed_cookies.py`（`--show` / `--path` / `--clear` / `--stdin` / `--file`） |
| `src/models.rs` | `SearchResult`（含 `summary: Option<String>`、`engines: Vec<String>`，均按需跳过序列化）、`SearchError`、`SearchResponse`（含 `duplicatesRemoved` 与 `filters` 元数据）等。**注意**：给 `SearchResult` 加字段必须同步所有构造点（各 engine 内共 12 处，均带 `engines: Vec::new()`） |
| `src/filters.rs` | 按请求过滤：`Freshness`（day/week/month/year 或 `YYYY-MM-DD..YYYY-MM-DD`）、`Topic`、`SearchOptions`、`FreshnessTier`（applied/best_effort/unsupported），以及不依赖 chrono 的 civil date 换算（`days_from_civil`/`civil_from_days`）。时间窗口按 **UTC** 计算 |
| `src/dedupe.rs` | URL 规范化 + 精确去重。`canonical_url()` 先拆 Bing `/ck/` 跳转壳，再统一 scheme/host/尾斜杠/去 fragment，**只删已知追踪参数**（列表见 `TRACKING_PARAMS`），**绝不砍整个 query string**（否则 B 站 `?p=2` 会被误合并——这是照抄 Tavily `split('?')[0]` 的坑）。不做内容级去重 |
| `src/snippets.rs` | `get_page` 的 highlights 片段排序（`highlight()`）。纯本地无 LLM：ASCII 按词、CJK 按 bigram 匹配，窗口按**不同查询词覆盖数优先**打分，偏移为字符级半开区间 |
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

> 例外：短时**反爬** Cookie（搜狗 SNUID）按用户要求走**本地缓存文件**而非环境变量（见下文「两种 Cookie 的分工」），目的是刷新无需重启。文件同样在仓库外。

### 2. 代理分流
- `PROXY_URL` + `USE_PROXY=true`：默认走代理
- `DIRECT_DOMAINS`：命中子串的域名**绕过代理直连**（国内站点如 sogou/weixin/baidu/zhihu 会封代理 IP 弹验证码；`cnki` 在腾讯 EdgeOne 后，代理/数据中心 IP 一律 418；`dblp` 的 Anubis 反爬同样拦代理 IP）；默认值见 `config.rs` 的 `DEFAULT_DIRECT_DOMAINS`；设为 `none` 则全部走代理
- ⚠️ 用户若在 MCP 配置（mcp.json / opencode.json）里显式写了 `DIRECT_DOMAINS`，会**覆盖**默认值——新增直连域名时必须同步用户配置，否则改了 `config.rs` 也没效果
- 🔴 **必须调用 `.no_proxy()` 才能真正绕过**（2026-09-10 修复的重大 bug）：reqwest 的 `ClientBuilder` 默认 `auto_sys_proxy = true`，`build()` 时无条件 push 一个 system matcher，它会读 **Windows 注册表** `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`（`ProxyEnable`/`ProxyServer`）**以及** `HTTP_PROXY`/`HTTPS_PROXY` 环境变量。所以只"不加显式 proxy"是**无效绕过**——用户一旦开系统代理/全局代理，所谓直连的请求仍会走代理。`.proxy(p)` 会顺带把 `auto_sys_proxy` 置 false，`.no_proxy()` 同样（并清空 proxies）。所有"绕过"分支（`config.rs::build_reqwest_client_with_proxy`、`sogou.rs`、`weixin.rs`、`fetch.rs::build_fetch_client`）都必须显式 `builder.no_proxy()`。
- 验证手法：把 `HTTPS_PROXY` 指向死地址（如 `http://127.0.0.1:1`）跑 MCP——若"直连"域名仍失败，说明绕过没生效（参见仓库根目录的 `ab_test.py`）

### 手工验证脚本（`tests/manual/`，2026-09-10 加入）

仓库无自动化测试，这些脚本用 JSON-RPC 喂 stdin 直接跑二进制，全部**从仓库根目录**执行：

| 脚本 | 用途 |
|------|------|
| `ab_test.py [bin]` | 对照 `DIRECT_DOMAINS` × 环境变量代理（含**死地址**代理）验证绕过是否真的生效——修改代理逻辑后必跑 |
| `matrix_test.py <bin> <proxy\|noproxy> [engines…]` | 引擎 × 代理开关矩阵，模拟"全局/系统代理开/关"两种用户环境（密钥自动从 `mcp.json` 读，不硬编码） |
| `regression_test.py [bin]` | 10 个引擎全量回归，打印每个引擎的通过/失败与原因 |
| `retry_engine.py <engine> <query> [n]` | 单引擎重复 N 次，用于区分偶发抖动与稳定失败（csdn 常见偶发） |
| `net_diag.py` | 网络环境诊断：网卡/默认路由/环境变量代理/直连与代理出口 IP/CNKI 可达性 |
| `probe_academic.py` | dblp / startpage 直连 vs 代理探测，**自动 gunzip** 后判定 Anubis 还是 429 |
| `ua_probe.py` | 用多种 UA（chrome/googlebot/curl/...）探测 Anubis 是否放行 |
| `bin_compare.py [engines…]` | 对比 debug/release 两个二进制下各引擎的表现 |
| `sogou_once.py` | 搜狗**单次**探测（推荐用这个，别连测——会加重 IP 封禁） |
| `seed_cookies.py [site] <ck\|--stdin\|--file f>` | 🔴 **搜狗验证码后必用**：把浏览器 cookie 写入本地缓存（`--show`/`--path`/`--clear` 查看/路径/清空）；无需重启即生效 |
| `cache_cookie_test.py [bin]` | A/B 验证“仅靠缓存文件”（**故意剔除 FETCH_COOKIES**）搜狗/微信能否工作 |
| `sogou_cookie_test.py` | 裸请求验证 `FETCH_COOKIES` 里的 SNUID/SUV 能否让搜狗恢复 |
| `anubis_solve.py [url]` / `anubis_client_test.py` | Anubis PoW 求解器原型（已验证可破解 dblp + startpage） |
| `anubis_verify.py [bin] [engines…]` | 端到端验证 Anubis 求解（dblp/startpage）+ 各步耗时 |
| `dblp_timing.py` | 拆解 dblp Anubis 流程每步耗时（定位是网络慢还是代码慢） |
| `startpage_after_anubis.py` / `startpage_parse_probe.py` | 破解后 startpage 结果页结构分析（选择器已变） |
| `scan_binary_secrets.py [exe]` | 🔴 **出包后必跑**：在二进制里搜真实凭据，确认没被打包进去 |

⚠️ 除 `matrix_test.py` / `regression_test.py` / `bin_compare.py` 外，**不要在短时间内反复跑搜狗相关脚本**——实测会把出口 IP 从「已解封」打成「403 硬封」。

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
- dblp 引擎：服务器对 `Accept-Encoding: gzip, deflate, br` 返回 500，dblp.rs 已显式覆盖为 `gzip`（Windows 同样受益）；`dblp.org` 在部分大陆网络被连接重置，已按 `DBLP_HOSTS` 回退 `dblp.uni-trier.de`，并用 `PREFERRED_HOST` 记住上次成功的 host（避免每次重付 connect 超时）；2026-09 起两个域名均部署 **Anubis** PoW，已由 `src/anubis.rs` 自动求解
- startpage 引擎：2026-09 起被 Anubis 拦（首页正常、**只有 `/sp/search` POST 被挑战**），已接入 `src/anubis.rs`；解析器已适配新结构（`a.result-title` + `p.description` + `h2.wgl-title`，旧的 `.w-gl__result-wrapper` 已不存在）
- **Anubis 实测技巧**：挑战页是 **gzip 压缩**的，curl/python 直接看原始字节会漏判（看不到 "anubis"），必须 `--compressed` 或先解压；`difficulty` 单位是**十六进制位数**（JS `Math.pow(16, -difficulty)`）不是 bit
- sogou 的验证码是**入口 IP 级累积风控**：被封后叫用户做一次人机验证，再用 `set_cookies` 工具或 `tests/manual/seed_cookies.py` 写入 Cookie 缓存（**不要用环境变量**——用户 2026-09-10 明确要求短时 Cookie 走缓存文件，这样刷新无需重启）

### 3. 两种 Cookie 的分工（不要混用）
| | `FETCH_COOKIES`（env） | cookie 缓存（文件） |
|---|---|---|
| 用途 | 长期**登录**凭证（知乎 `d_c0`、B站 `SESSDATA`），`fetch.rs` 用 | 短时**反爬**凭证（搜狗 `SNUID`/`SUV`），`sogou.rs`/`weixin.rs` 用 |
| 加载 | 启动时读一次（改了必须重启） | 按 mtime 重读（**改了立即生效**） |
| 位置 | 用户 MCP 配置 | `%LOCALAPPDATA%\advent-mcp\cookie-cache.json` |
- 两者都在**仓库外**；日志只允许打印 `fingerprint()` 结果
- 对应 MCP 工具 `set_cookies`（`src/main.rs::handle_set_cookies`）

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

- ❌ **硬编码 / 文件读取 / 提交任何 API key、cookie、token**——🔴 **本仓库在 GitHub + Gitee 双公开**
  - 真实凭据**只允许从环境变量读**；用户在 `mcp.json` / `opencode.json` 的 `env` 里配（两处都在仓库外）
  - 代码/脚本/文档里只允许写**变量名**和占位符（`SNUID=...`、`your_key_here`）
  - 写测试脚本时先想公开性：**不要**把"本地能跑通"的真实 cookie 留在文件里（2026-09-10 曾把用户搜狗 cookie 写进 `tests/manual/sogou_with_cookies.py`，侥幸未提交）
  - 自查手法：`git status`、`git log -S "<敏感值>"`、正则扫 `SNUID=[A-F0-9]{20,}|sk-[a-f0-9]{20,}|SESSDATA=`
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
