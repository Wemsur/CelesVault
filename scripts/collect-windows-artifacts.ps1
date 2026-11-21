param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Commit
)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$' -or $Commit -notmatch '^[0-9a-f]{40}$') { throw 'Invalid release metadata' }
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$out = Join-Path $root 'dist/windows-release'
if (Test-Path -LiteralPath $out) { throw 'Release output already exists; use a clean runner' }
New-Item -ItemType Directory -Path $out -Force | Out-Null
$release = Join-Path $root 'target/x86_64-pc-windows-msvc/release'
$exe = Join-Path $release 'CelesVault.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Missing portable EXE: $exe" }
$installers = @(Get-ChildItem -LiteralPath (Join-Path $release 'bundle/nsis') -Filter '*.exe' -File)
if ($installers.Count -ne 1) { throw "Expected exactly one NSIS installer; found $($installers.Count)" }
Copy-Item -LiteralPath $exe -Destination (Join-Path $out "CelesVault-$Version-windows-x64.exe")
Copy-Item -LiteralPath $installers[0].FullName -Destination (Join-Path $out "CelesVault-$Version-windows-x64-setup.exe")
# 对应源码来自该次构建的精确提交，只含 Git 跟踪内容；不打包 data、票据或本机日志。
& git archive --format=zip "--prefix=CelesVault-$Version/" "--output=$out/CelesVault-$Version-source.zip" $Commit
if ($LASTEXITCODE -ne 0) { throw 'git archive failed' }
Copy-Item -LiteralPath 'LICENSE', 'THIRD_PARTY_NOTICES.md' -Destination $out
$notice = @"
# CelesVault $Version · Windows x64

提交：$Commit
目标：x86_64-pc-windows-msvc

便携 EXE 和 NSIS 安装器均未代码签名，Windows 可能显示未知发布者提示。
应用需要管理员权限和已安装的 WebView2。不会安装 Windows 服务、自启动或更改文件关联。
资料 data 外置，首次请选择不受还原影响的持久目录。更新前正常退出并备份整个 data。
本包不是课堂验收证明；ETW、WPS、设备和学校环境需人工验证。
源码 ZIP 是同一提交的完整 Git 跟踪源码，包含 vendor、构建脚本、依赖锁与许可。
依赖按锁定清单联网获取，源码 ZIP 不是离线依赖镜像。

请核对 SHA256SUMS.txt。此工作流只创建 Release 草稿，不会自动公开发布。
"@
$notice | Set-Content -LiteralPath (Join-Path $out 'RELEASE-NOTES.md') -Encoding utf8
[ordered]@{ version = $Version; tag = "v$Version"; commit = $Commit; target = 'x86_64-pc-windows-msvc'; run_id = $env:GITHUB_RUN_ID; signed = $false } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $out 'build-info.json') -Encoding utf8
# 校验所有交付文件，不把校验清单自身纳入递归摘要。
$hashes = Get-ChildItem -LiteralPath $out -File | Sort-Object Name | ForEach-Object {
    $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $($_.Name)"
}
$hashes | Set-Content -LiteralPath (Join-Path $out 'SHA256SUMS.txt') -Encoding ascii
