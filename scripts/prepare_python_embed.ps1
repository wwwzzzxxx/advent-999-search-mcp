<#
.SYNOPSIS
  生成嵌入 Python 运行时的 python-embed.zip（advent-999-search-mcp 构建前置步骤）

.DESCRIPTION
  deepseek 引擎的 OpenCode Go 后端需要 Python requests 才能通过
  opencode.ai 的 Cloudflare TLS 指纹检测。本脚本：
  1. 下载 Python 3.12 embeddable 包（约 11MB）
  2. 解压，安装 requests 及依赖
  3. 重新打包成 python-embed.zip（放项目根目录，被 include_bytes! 嵌入 exe）

  产物 python-embed.zip 已 gitignore，不提交。

.PARAMETER PythonVersion
  embeddable Python 版本，默认 3.12.10（3.12.12 无 embeddable 包）

.PARAMETER Force
  强制重新下载（默认 zip 存在则跳过）

.EXAMPLE
  ./scripts/prepare_python_embed.ps1
#>
[CmdletBinding()]
param(
  [string]$PythonVersion = "3.12.10",
  [switch]$Force
)

$ErrorActionPreference = "Stop"
$ProjectDir = Split-Path -Parent $PSScriptRoot
$OutZip = Join-Path $ProjectDir "python-embed.zip"
$WorkDir = Join-Path $env:TEMP "advent-python-embed"

function Info($m) { Write-Host "[INFO] $m" -ForegroundColor Cyan }
function Ok($m)   { Write-Host "[OK]   $m" -ForegroundColor Green }

if (Test-Path $OutZip) {
  if (-not $Force) {
    Info "python-embed.zip 已存在: $((Get-Item $OutZip).Length / 1MB) MB，跳过（用 -Force 重建）"
    exit 0
  }
  Remove-Item $OutZip -Force
}

# 清理工作目录
if (Test-Path $WorkDir) { Remove-Item $WorkDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null

$zipPath = Join-Path $WorkDir "python-embed.zip"
$extractDir = Join-Path $WorkDir "py"

# 1. 下载 embeddable 包
$url = "https://www.python.org/ftp/python/$PythonVersion/python-$PythonVersion-embed-amd64.zip"
Info "下载 $url ..."
curl.exe -s -L -o $zipPath --max-time 300 $url
if ($LASTEXITCODE -ne 0 -or (Get-Item $zipPath).Length -lt 1000000) {
  throw "下载失败（大小 $((Get-Item $zipPath).Length) bytes），检查版本号或网络"
}
Ok "下载完成: $((Get-Item $zipPath).Length / 1MB) MB"

# 2. 解压
Info "解压 ..."
Expand-Archive -Path $zipPath -DestinationPath $extractDir -Force

# 3. 启用 site-packages（_pth 文件加 import site）
$pth = Get-ChildItem $extractDir -Filter "python*._pth" | Select-Object -First 1
if (-not $pth) { throw "未找到 ._pth 文件" }
Set-Content $pth.FullName "python312.zip`n.`nimport site" -Encoding ASCII
Info "已启用 site-packages: $($pth.Name)"

# 4. 安装 requests（用系统 python 的 pip 装到目标目录）
Info "安装 requests ..."
$sysPy = Get-Command python -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source
if (-not $sysPy) { $sysPy = "python" }
# 优先 conda 环境（本机固定路径），否则 PATH
foreach ($cand in @("Z:\conda\envs\E1\python.exe", "C:\ProgramData\Miniconda3\python.exe")) {
  if (Test-Path $cand) { $sysPy = $cand; break }
}
& $sysPy -m pip install requests --target $extractDir --no-compile --quiet
if ($LASTEXITCODE -ne 0) { throw "pip 安装 requests 失败" }
Ok "requests 已安装"

# 5. 重新打包（zip 用 deflate，Python 的 zipimport 兼容）
Info "重新打包 ..."
Push-Location $extractDir
try {
  Compress-Archive -Path (Get-ChildItem -Force | Select-Object -ExpandProperty FullName) -DestinationPath $OutZip -CompressionLevel Optimal -Force
} finally { Pop-Location }

$size = (Get-Item $OutZip).Length / 1MB
Ok "生成完成: $OutZip ($('{0:N1}' -f $size) MB)"

# 6. 清理
Remove-Item $WorkDir -Recurse -Force -ErrorAction SilentlyContinue
Info "清理完成"
