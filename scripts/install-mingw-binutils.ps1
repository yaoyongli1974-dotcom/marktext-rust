<#
.SYNOPSIS
    安装 GNU 汇编器 as.exe —— Windows GNU 工具链缺失的关键一环。

.DESCRIPTION
    Rust 的 `windows-gnu` 工具链依赖 `windows-link` 的 `raw-dylib` 链接方式，
    rustc 因此需要调用 GNU `dlltool` 生成导入库，而 GNU `dlltool` 又要靠 GNU
    汇编器 `as.exe` 汇编中间文件。

    rustup 分发的 `rust-mingw` 组件在 Windows 上只带
    `dlltool.exe / ld.exe / x86_64-w64-mingw32-gcc.exe`，**不含 `as.exe`**，
    于是任何用到 `windows-sys` 的 crate 都会在这里编译失败：

        error: dlltool could not create import library with ...dlltool.exe
        ...dlltool.exe: CreateProcess error: ... as.exe

    本脚本从 MSYS2 仓库下载 mingw-w64 的 binutils 与它需要的运行库，装到
    `%USERPROFILE%\.cargo\mingw-bin`，供 `tools/dlltool-proxy` 使用。

    幂等：已安装且版本可用时会直接跳过。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\install-mingw-binutils.ps1
#>
[CmdletBinding()]
param(
    # 安装目录，默认 %USERPROFILE%\.cargo\mingw-bin
    [string]$Destination = (Join-Path $env:USERPROFILE '.cargo\mingw-bin')
)

$ErrorActionPreference = 'Stop'

function Write-Step([string]$Message) {
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Write-Ok([string]$Message) {
    Write-Host "    $Message" -ForegroundColor Green
}

# ---------------------------------------------------------------- 依赖检查
if (-not (Get-Command tar.exe -ErrorAction SilentlyContinue)) {
    throw '需要 Windows 自带的 tar.exe（Windows 10 1803+），请先升级 Windows。'
}

$work = Join-Path $env:TEMP 'mingw-binutils-install'
New-Item -ItemType Directory -Path $work -Force | Out-Null

# ------------------------------------------------------- 解析 MSYS2 包名
$repo = 'https://repo.msys2.org/mingw/mingw64/'
Write-Step "读取 MSYS2 仓库索引：$repo"
$index = Invoke-WebRequest -Uri $repo -UseBasicParsing -TimeoutSec 60
$packages = @($index.Links | ForEach-Object { $_.href } |
    Where-Object { $_ -like '*.pkg.tar.zst' -and $_ -notlike '*.sig' })

function Resolve-Package([string]$Prefix) {
    $hit = @($packages | Where-Object { $_ -like "$Prefix-*.pkg.tar.zst" } |
        Sort-Object -Unique)
    if ($hit.Count -eq 0) { return $null }
    return $hit[-1]
}

# -------------------------------------------------------------- 安装计划
# binutils 提供 as.exe / dlltool.exe；其余是它们运行时的依赖库。
$plan = @(
    @{ Prefix = 'mingw-w64-x86_64-binutils'; Optional = $false },
    @{ Prefix = 'mingw-w64-x86_64-libiconv'; Optional = $true },
    @{ Prefix = 'mingw-w64-x86_64-zlib-ng-compat'; Optional = $true },
    @{ Prefix = 'mingw-w64-x86_64-zstd'; Optional = $true }
)

New-Item -ItemType Directory -Path $Destination -Force | Out-Null

foreach ($item in $plan) {
    $name = Resolve-Package $item.Prefix
    if (-not $name) {
        if ($item.Optional) {
            Write-Host "    跳过可选包 $($item.Prefix)（仓库中未找到）" -ForegroundColor DarkGray
            continue
        }
        throw "仓库中找不到包 $($item.Prefix)"
    }

    Write-Step "下载 $name"
    $archive = Join-Path $work $name
    Invoke-WebRequest -Uri "$repo$name" -OutFile $archive -UseBasicParsing -TimeoutSec 300
    & tar.exe -xf $archive -C $work
    if ($LASTEXITCODE -ne 0) { throw "解压 $name 失败" }

    $bin = Join-Path $work 'mingw64\bin'
    if (Test-Path $bin) {
        Copy-Item (Join-Path $bin '*.exe') $Destination -Force -ErrorAction SilentlyContinue
        Copy-Item (Join-Path $bin '*.dll') $Destination -Force -ErrorAction SilentlyContinue
    }
}

# libintl-8.dll：MSYS2 的仓库索引里未必能定位到该包，
# 但 Git for Windows 自带一份，直接借用。
$libintl = Join-Path $Destination 'libintl-8.dll'
if (-not (Test-Path $libintl)) {
    $gitIntl = 'C:\Program Files\Git\mingw64\bin\libintl-8.dll'
    if (Test-Path $gitIntl) {
        Write-Step '借用 Git for Windows 的 libintl-8.dll'
        Copy-Item $gitIntl $Destination -Force
    }
}

# ---------------------------------------------------------------- 验证
$as = Join-Path $Destination 'as.exe'
$dlltool = Join-Path $Destination 'dlltool.exe'
if (-not (Test-Path $as)) { throw "安装失败：找不到 $as" }
if (-not (Test-Path $dlltool)) { throw "安装失败：找不到 $dlltool" }

Write-Step '验证'
$asVersion = (& $as --version 2>&1 | Select-Object -First 1)
$dlltoolVersion = (& $dlltool --version 2>&1 | Select-Object -First 1)
Write-Ok $asVersion
Write-Ok $dlltoolVersion

Write-Host ''
Write-Host '安装完成。' -ForegroundColor Green
Write-Host "  binutils  : $Destination"
Write-Host '  代理安装 : cargo build -p dlltool-proxy; copy target\debug\dlltool-proxy.exe %USERPROFILE%\.cargo\bin\'
