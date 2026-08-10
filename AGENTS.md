# AGENTS.md — advent-999-search-mcp

## 项目概览

Rust 编写的 MCP 服务器，提供三类工具：
- **`web`** — 网络搜索（10 个引擎：exa, bing, csdn, juejin, startpage, sogou, weixin, dblp, cnki + 凭据门控的 deepseek）
- **`local`** — 本地文件搜索（Everything / es.exe，未配置时工具不出现）
- **`get_page`** — 抓取网页正文（自动回退策略，支持需要登录 cookie 的站点）

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
| `src/config.rs` | 全部配置均来自环境变量；`DEFAULT_DIRECT_DOMAINS` 默认国内直连域名表。完整环境变量清单（含 `DEFAULT_SEARCH_ENGINE`、`FETCH_TIMEOUT`、`FETCH_COOKIES` 等）见 [README.md 第 3 节](README.md) |
| `src/models.rs` | `SearchResult`（含 `summary: Option<String>`，序列化时跳过 None）、`SearchError` 等 |
| `src/engines/mod.rs` | `SearchEngine` trait、`create_engine_map()`、`credential_gated_engines()` |
| `src/engines/*.rs` | 各引擎实现 |
| `src/fetch.rs` | HTTP 抓取与正文提取：代理/直连分流、cookie、超时。**~2300 行大文件**，含 ElementRef→markdown 转换管线、zhihu 签名抓取（zhihu_sign）、反爬回退策略。改动前务必通读相关段落，勿盲目重写 |
| `src/local_search.rs` | Everything 本地搜索 |
| `src/python_embed.rs` | 内嵌 Python 运行时的解压与定位 |
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

### 5. 内嵌 Python
`python-embed.zip` 通过 `include_bytes!` 编译进 exe；首次运行解压到 `%LOCALAPPDATA%\advent-mcp-python`，`.version` marker 控制版本（改 zip 后必须 bump `PYTHON_EMBED_VERSION`）。go 后端用 `spawn_blocking` 启动 `python.exe`，脚本以 stdin 传 query、从 `os.environ` 读 key，输出 JSON。

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
- ❌ 编译测试时直接 `cargo build --release` 覆盖运行中的 exe（会失败并可能中断会话）——先用 debug 构建在 `target\debug\` 验证，通过后再替换

## 发布流程（publish.ps1 已封装）

1. 更新 `Cargo.toml` 版本号 + `src/main.rs` serverInfo
2. 构建 release，确认二进制字节数（当前 v0.3.0 = 17,812,480 bytes）
3. `./scripts/publish.ps1`：push 双 remote（GitHub `wwwzzzxxx` + Gitee `pzwzx`）→ 两平台创建/更新 Release → 上传附件
4. 验证两平台附件字节数一致（Gitee API 需要 `target_commitish` 字段）
