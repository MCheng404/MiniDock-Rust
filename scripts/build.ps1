# MiniDock-rs 构建/测试（PowerShell 入口，注入 MSVC + Windows SDK 环境）。
#
# 与 scripts/env.sh 等价，供不习惯 Git Bash 的场景使用。
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File scripts\build.ps1            # debug 构建
#   powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -Mode release
#   powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -Mode test
#   powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -Mode run
#
# 说明：MSVC 与 SDK 路径**不写死版本号**，按目录扫描取最新，避免换机失效。

param(
    [ValidateSet('build', 'release', 'test', 'run', 'check', 'clippy')]
    [string]$Mode = 'build',
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Rest
)

$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

function Find-MsvcRoot {
    $bases = @(
        'D:\Program Files\Microsoft Visual Studio',
        'C:\Program Files\Microsoft Visual Studio',
        'C:\Program Files (x86)\Microsoft Visual Studio'
    )
    $candidates = @()
    foreach ($b in $bases) {
        if (-not (Test-Path $b)) { continue }
        $candidates += Get-ChildItem -Path $b -Directory -Recurse -Depth 3 -Filter 'MSVC' -ErrorAction SilentlyContinue |
            ForEach-Object { Get-ChildItem -Path $_.FullName -Directory -ErrorAction SilentlyContinue } |
            Where-Object { Test-Path (Join-Path $_.FullName 'bin\Hostx64\x64\cl.exe') }
    }
    if (-not $candidates) { return $null }
    # 按版本号排序取最新（14.52 > 14.44 > 14.29）
    return ($candidates | Sort-Object { [version](Split-Path $_.FullName -Leaf) } -Descending | Select-Object -First 1).FullName
}

function Find-SdkVersion {
    $inc = 'C:\Program Files (x86)\Windows Kits\10\Include'
    if (-not (Test-Path $inc)) { return $null }
    return (Get-ChildItem $inc -Directory | Sort-Object Name | Select-Object -Last 1).Name
}

if (Get-Command cl.exe -ErrorAction SilentlyContinue) {
    Write-Host '· 使用当前 shell 已有的 MSVC 环境'
} else {
    $msvc = Find-MsvcRoot
    $sdkVer = Find-SdkVersion
    if (-not $msvc) { Write-Host '✗ 找不到 MSVC 工具集（需要「使用 C++ 的桌面开发」工作负载）'; exit 1 }
    if (-not $sdkVer) { Write-Host '✗ 找不到 Windows SDK'; exit 1 }

    $sdk = 'C:\Program Files (x86)\Windows Kits\10'
    $env:INCLUDE = "$msvc\include;$sdk\Include\$sdkVer\ucrt;$sdk\Include\$sdkVer\um;$sdk\Include\$sdkVer\shared;$sdk\Include\$sdkVer\winrt"
    $env:LIB = "$msvc\lib\x64;$sdk\Lib\$sdkVer\ucrt\x64;$sdk\Lib\$sdkVer\um\x64"
    $env:PATH = "$msvc\bin\Hostx64\x64;$env:PATH"
    $env:CC = "$msvc\bin\Hostx64\x64\cl.exe"
    $env:CXX = "$msvc\bin\Hostx64\x64\cl.exe"
    $env:AR = "$msvc\bin\Hostx64\x64\lib.exe"
    Write-Host "· MSVC  $msvc"
    Write-Host "· SDK   $sdkVer"
}

Push-Location $RepoRoot
try {
    switch ($Mode) {
        'build'   { cargo build --target x86_64-pc-windows-msvc @Rest }
        'release' { cargo build --target x86_64-pc-windows-msvc --release @Rest }
        'run'     { cargo run   --target x86_64-pc-windows-msvc @Rest }
        'test'    { cargo test  --target x86_64-pc-windows-msvc --workspace --lib --tests @Rest }
        'check'   { cargo check --target x86_64-pc-windows-msvc --workspace --all-targets @Rest }
        'clippy'  { cargo clippy --target x86_64-pc-windows-msvc --workspace --all-targets -- -D warnings @Rest }
    }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}
Write-Host "✓ $Mode 完成"
