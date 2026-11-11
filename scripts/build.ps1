<#
.SYNOPSIS
    一键构建 MarkText Rust 移植版。

.DESCRIPTION
    依次完成：
      1. 构建 dlltool 代理（Windows GNU 工具链缺失 as.exe 的补丁）；
      2. 把代理安装到 %USERPROFILE%\.cargo\bin（rustc 以裸程序名调用它）；
      3. 构建整个 workspace。

    首次使用请先执行：
        powershell -ExecutionPolicy Bypass -File scripts\install-mingw-binutils.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build.ps1
    powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -Release
#>
[CmdletBinding()]
param(
    [switch]$Release,
    [int]$Jobs = 2
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# cargo 不一定在当前会话的 PATH 里。
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if ($env:PATH -notlike "*$cargoBin*") {
    $env:PATH = "$cargoBin;$env:PATH"
}

function Write-Step([string]$Message) {
    Write-Host "==> $Message" -ForegroundColor Cyan
}

$profile = if ($Release) { 'release' } else { 'debug' }

Write-Step '构建 dlltool 代理'
& cargo build -p dlltool-proxy --quiet
if ($LASTEXITCODE -ne 0) { throw 'dlltool-proxy 构建失败' }

Write-Step '安装代理到 PATH 目录'
$proxySource = Join-Path $root "target\$profile\dlltool-proxy.exe"
$proxyTarget = Join-Path $cargoBin 'dlltool-proxy.exe'
Copy-Item $proxySource $proxyTarget -Force
Write-Host "    $($proxyTarget)" -ForegroundColor Green

Write-Step "构建 workspace（$profile）"
& cargo build --workspace -j $Jobs
if ($LASTEXITCODE -ne 0) { throw 'workspace 构建失败' }

$exe = Join-Path $root "target\$profile\marktext-app.exe"
if (Test-Path $exe) {
    Write-Host ''
    Write-Host '构建完成：' -ForegroundColor Green
    Write-Host "  $exe"
}
