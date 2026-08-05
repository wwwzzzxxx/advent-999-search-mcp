<#
.SYNOPSIS
  一键发布 release 到 GitHub 和 Gitee 双平台

.DESCRIPTION
  自动完成：
  1. 读取版本号（默认从 Cargo.toml）并构造 tag vX.Y.Z
  2. cargo build --release 构建二进制
  3. 创建/移动本地 tag 并推送到 GitHub(gitee) 远端
  4. 发布到 GitHub（gh CLI，已存在则更新并覆盖资产）
  5. 发布到 Gitee（API v5，已存在则更新并覆盖附件）

.PARAMETER Version
  版本号，默认从 Cargo.toml 读取（如 0.3.0 → tag v0.3.0）

.PARAMETER Notes
  Release 说明 Markdown。默认从 git log（上一 tag 至今）自动生成

.PARAMETER Title
  Release 标题，默认 = tag 名

.PARAMETER GiteeToken
  Gitee 私人令牌。优先级：本参数 > 环境变量 GITEE_TOKEN > $HOME\.agents\secrets\gitee_token.txt

.PARAMETER SkipBuild
  跳过 cargo build（用于热修复时复用已有二进制）

.PARAMETER SkipGitHub
  只发 Gitee

.PARAMETER SkipGitee
  只发 GitHub

.PARAMETER ForceTag
  tag 已存在时强制移动到当前 HEAD（热修复场景），并 force push

.PARAMETER Prerelease
  标记为预发布

.PARAMETER SaveToken
  把本次使用的令牌保存到 $HOME\.agents\secrets\gitee_token.txt 供以后复用

.PARAMETER DryRun
  只打印将要执行的步骤，不实际执行

.EXAMPLE
  ./scripts/publish.ps1                      # 发 v(当前Cargo版本) 到双平台
  ./scripts/publish.ps1 -Version 0.3.0       # 指定版本
  ./scripts/publish.ps1 -Notes "## 更新内容`n- ..." -Title "v0.3.0 大更新"
  ./scripts/publish.ps1 -ForceTag            # 热修复：tag 移到 HEAD 重新发布
#>
[CmdletBinding()]
param(
  [string]$Version = "",
  [string]$Notes = "",
  [string]$Title = "",
  [string]$GiteeToken = "",
  [switch]$SkipBuild,
  [switch]$SkipGitHub,
  [switch]$SkipGitee,
  [switch]$ForceTag,
  [switch]$Prerelease,
  [switch]$SaveToken,
  [switch]$DryRun
)

$ErrorActionPreference = "Stop"

$GH_REPO     = "wwwzzzxxx/advent-999-search-mcp"
$GITEE_REPO  = "pzwzx/advent-999-search-mcp"
$ProjectDir  = Split-Path -Parent $PSScriptRoot
$ExePath     = Join-Path $ProjectDir "target\release\advent-999-search-mcp.exe"

function Info($m) { Write-Host "[INFO] $m" -ForegroundColor Cyan }
function Ok($m)   { Write-Host "[OK]   $m" -ForegroundColor Green }
function Warn($m) { Write-Host "[WARN] $m" -ForegroundColor Yellow }
function Fail($m) { Write-Host "[FAIL] $m" -ForegroundColor Red; exit 1 }

# ---------- 1. 版本号 ----------
if (-not $Version) {
  $m = Select-String -Path (Join-Path $ProjectDir "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"'
  if (-not $m) { Fail "无法从 Cargo.toml 读取版本号" }
  $Version = $m.Matches[0].Groups[1].Value
}
$Tag = "v$Version"
if (-not $Title) { $Title = $Tag }
Info "发布版本: $Tag | 标题: $Title"

# ---------- 2. Release 说明 ----------
if (-not $Notes) {
  $tags = @(git -C $ProjectDir tag --sort=-version:refname 2>$null)
  $prev = if ($tags.Count -gt 1) { $tags[1] } else { $null }
  $log = if ($prev) { @(git -C $ProjectDir log --oneline "$prev..HEAD" 2>$null) }
         else       { @(git -C $ProjectDir log --oneline 2>$null) }
  $lines = @()
  foreach ($l in $log) {
    $msg = ($l -replace '^[0-9a-f]+\s+', '')
    if ($msg -match '^Merge ') { continue }
    $lines += "- $msg"
  }
  if ($lines.Count -eq 0) { $lines = "- 无新提交" }
  $Notes = "## What's new in $Tag`n`n### Commits`n" + ($lines -join "`n")
}

# ---------- 3. 构建 ----------
if (-not $SkipBuild) {
  Info "构建 release 二进制..."
  if ($DryRun) {
    Info "[DRY-RUN] cargo build --release"
  } else {
    Push-Location $ProjectDir
    try {
      cargo build --release 2>&1 | Select-Object -Last 3
      if ($LASTEXITCODE -ne 0) { Fail "cargo build 失败" }
    } finally { Pop-Location }
  }
}
if (-not (Test-Path $ExePath)) { Fail "找不到 $ExePath（可用 -SkipBuild 跳过构建）" }
Ok "二进制: $ExePath ($((Get-Item $ExePath).Length) bytes)"

# ---------- 4. Tag 处理 ----------
$tagExists = git -C $ProjectDir rev-parse -q --verify "refs/tags/$Tag" 2>$null
if ($tagExists) {
  if ($ForceTag) {
    Info "本地 tag $Tag 已存在，-ForceTag 移动到当前 HEAD"
    if ($DryRun) { Info "[DRY-RUN] git tag -f $Tag" }
    else { git -C $ProjectDir tag -f $Tag }
  } else {
    Warn "本地 tag $Tag 已存在，将复用；如需移动到最新代码用 -ForceTag"
  }
} else {
  Info "创建本地 tag $Tag"
  if ($DryRun) { Info "[DRY-RUN] git tag $Tag" }
  else { git -C $ProjectDir tag $Tag }
}

# 推送 tag 到两个远端
if (-not $DryRun) {
  foreach ($remote in @("origin", "gitee")) {
    $a = @("-C", $ProjectDir, "push", $remote, $Tag)
    if ($ForceTag) { $a += "--force" }
    & git @a 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { Warn "push $remote $Tag 失败（远端可能已存在；继续发布）" }
    else { Ok "tag $Tag 已推送到 $remote" }
  }
}

# ---------- 5. GitHub ----------
function Publish-GitHub {
  if ($SkipGitHub) { Warn "跳过 GitHub（-SkipGitHub）"; return }
  Info "发布到 GitHub ($GH_REPO)..."
  $notesFile = Join-Path $env:TEMP "release-notes-$Tag.md"
  $Notes | Set-Content -Path $notesFile -Encoding UTF8
  Push-Location $ProjectDir
  try {
    if ($DryRun) {
      Info "[DRY-RUN] gh release create/edit $Tag --repo $GH_REPO"
      return
    }
    $view = gh release view $Tag --repo $GH_REPO --json tagName 2>$null
    if ($view) {
      Info "Release $Tag 已存在，更新说明并覆盖资产"
      gh release edit $Tag --repo $GH_REPO --title $Title --notes-file $notesFile
      if ($LASTEXITCODE -ne 0) { Fail "gh release edit 失败" }
      gh release upload $Tag --repo $GH_REPO $ExePath --clobber
      if ($LASTEXITCODE -ne 0) { Fail "gh release upload 失败" }
    } else {
      $a = @("release", "create", $Tag, "--repo", $GH_REPO, "--title", $Title, "--notes-file", $notesFile)
      if ($Prerelease) { $a += "--prerelease" }
      $a += $ExePath
      gh @a
      if ($LASTEXITCODE -ne 0) { Fail "gh release create 失败" }
    }
    $url = gh release view $Tag --repo $GH_REPO --json url --jq .url
    Ok "GitHub: $url"
  } finally { Pop-Location; Remove-Item $notesFile -ErrorAction SilentlyContinue }
}

# ---------- 6. Gitee ----------
function Get-GiteeToken {
  if ($GiteeToken) { return $GiteeToken }
  if ($env:GITEE_TOKEN) { return $env:GITEE_TOKEN }
  $f = Join-Path $HOME ".agents\secrets\gitee_token.txt"
  if (Test-Path $f) { return (Get-Content $f -Raw).Trim() }
  Fail "未找到 Gitee 令牌：用 -GiteeToken 传入、设环境变量 GITEE_TOKEN、或先运行 -SaveToken 保存到 $f"
}

function Invoke-GiteeJson {
  param([string]$Method, [string]$Path, [hashtable]$Data)
  $url = "https://gitee.com/api/v5/repos/$GITEE_REPO$Path"
  $args = @("-s", "--max-time", "120", "-X", $Method)
  if ($Method -eq "GET") {
    $qs = ($Data.Keys | ForEach-Object { "$_=$([uri]::EscapeDataString($Data[$_]))" }) -join "&"
    if ($qs) { $url += "?$qs" }
  } else {
    foreach ($k in $Data.Keys) {
      $args += "--data-urlencode"; $args += "$k=$($Data[$k])"
    }
  }
  $out = & curl.exe @args $url
  if ($LASTEXITCODE -ne 0) { Fail "Gitee API 请求失败: $out" }
  if (-not $out -or $out -eq "null") { return $null }
  $obj = $out | ConvertFrom-Json
  if ($obj.message) { Fail "Gitee API 错误: $($obj.message) $($obj.messages -join ';')" }
  return $obj
}

function Publish-Gitee {
  if ($SkipGitee) { Warn "跳过 Gitee（-SkipGitee）"; return }
  Info "发布到 Gitee ($GITEE_REPO)..."
  $Token = Get-GiteeToken
  if ($DryRun) {
    Info "[DRY-RUN] POST/PATCH releases + 上传附件（token 已获取）"
    if ($SaveToken) { Info "[DRY-RUN] 保存令牌到 ~/.agents/secrets/gitee_token.txt" }
    return
  }
  if ($SaveToken) {
    $dir = Join-Path $HOME ".agents\secrets"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $Token | Set-Content -Path (Join-Path $dir "gitee_token.txt") -NoNewline
    Ok "令牌已保存到 $dir\gitee_token.txt"
  }
  # 查找已有 release
  $releases = @(Invoke-GiteeJson -Method GET -Path "/releases" -Data @{ access_token = $Token; per_page = 100 })
  $existing = $null
  foreach ($r in $releases) { if ($r.tag_name -eq $Tag) { $existing = $r; break } }
  if (-not $existing) {
    Info "创建 Gitee release $Tag ..."
    $rel = Invoke-GiteeJson -Method POST -Path "/releases" -Data @{
      access_token = $Token; tag_name = $Tag; target_commitish = "main"
      name = $Title; body = $Notes; prerelease = [bool]$Prerelease
    }
  } else {
    Info "Gitee release $Tag 已存在，更新说明"
    $rel = Invoke-GiteeJson -Method PATCH -Path "/releases/$($existing.id)" -Data @{
      access_token = $Token; name = $Title; body = $Notes
    }
  }
  $relId = $rel.id
  Ok "Gitee release: https://gitee.com/$GITEE_REPO/releases/tag/$Tag"

  # 上传附件（先删同名旧附件）
  $exeName = Split-Path $ExePath -Leaf
  $files = @(Invoke-GiteeJson -Method GET -Path "/releases/$relId/attach_files" -Data @{ access_token = $Token })
  foreach ($f in $files) {
    if ($f.name -eq $exeName) {
      Invoke-GiteeJson -Method DELETE -Path "/releases/$relId/attach_files/$($f.id)" -Data @{ access_token = $Token }
      Info "已删除旧附件 $exeName"
    }
  }
  Info "上传附件 $exeName ..."
  $up = & curl.exe -s --max-time 180 -X POST -F "access_token=$Token" -F "file=@$ExePath" `
        "https://gitee.com/api/v5/repos/$GITEE_REPO/releases/$relId/attach_files"
  if ($LASTEXITCODE -ne 0) { Fail "上传附件失败: $up" }
  $af = $up | ConvertFrom-Json
  if ($af.message) { Fail "上传附件错误: $($af.message)" }
  Ok "Gitee 附件: $($af.browser_download_url)"
}

# ---------- 执行 ----------
Publish-GitHub
Publish-Gitee

Info "=== 发布完成 ==="
Ok "GitHub: https://github.com/$GH_REPO/releases/tag/$Tag"
Ok "Gitee:  https://gitee.com/$GITEE_REPO/releases/tag/$Tag"
