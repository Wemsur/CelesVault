param(
    [string]$Name = 'CelesVault-Probe-0.1.1-win-x64',
    [switch]$SkipBuild,
    [string]$ConfigPath
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$root = Split-Path (Split-Path $project -Parent) -Parent
$dist = Join-Path $root 'dist'
$destination = Join-Path $dist $Name
$zip = "$destination.zip"
if ((Test-Path -LiteralPath $destination) -or (Test-Path -LiteralPath $zip)) {
    throw "Output already exists; choose a new -Name. Nothing was overwritten: $destination"
}
if (-not $SkipBuild) {
    $previous = $env:RUSTFLAGS
    try {
        $env:RUSTFLAGS = '-C target-feature=+crt-static'
        & cargo build --offline --locked --manifest-path (Join-Path $project 'Cargo.toml') --target x86_64-pc-windows-msvc --release
        if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed' }
    } finally { $env:RUSTFLAGS = $previous }
}
$configuration = Join-Path $project 'probe-config.json'
if ($ConfigPath) {
    if (-not (Test-Path -LiteralPath $ConfigPath -PathType Leaf)) { throw "Config not found: $ConfigPath" }
    Get-Content -LiteralPath $ConfigPath -Raw -Encoding UTF8 | ConvertFrom-Json | Out-Null
    $configuration = $ConfigPath
}
$exe = Join-Path $project 'target/x86_64-pc-windows-msvc/release/celesvault-probe.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw "Binary not found: $exe" }
New-Item -ItemType Directory -Path $destination | Out-Null
Copy-Item -LiteralPath $exe -Destination $destination
foreach ($file in @('Start-Probe.cmd', 'README.md', 'VALIDATION.md')) {
    Copy-Item -LiteralPath (Join-Path $project $file) -Destination $destination
}
Copy-Item -LiteralPath $configuration -Destination (Join-Path $destination 'probe-config.json')
$hash = (Get-FileHash -LiteralPath (Join-Path $destination 'celesvault-probe.exe') -Algorithm SHA256).Hash
@("CelesVault WPS Probe", "SHA256 celesvault-probe.exe: $hash", 'Logs are private local metadata. No original documents are included.') | Set-Content -LiteralPath (Join-Path $destination 'BUILD.txt') -Encoding UTF8
Compress-Archive -LiteralPath $destination -DestinationPath $zip -CompressionLevel Optimal
Write-Output "Package: $zip"
Write-Output "EXE SHA256: $hash"
