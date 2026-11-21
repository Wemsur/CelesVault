# 只读查询。输入取自环境变量，不插入 PowerShell 源码。
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
try {
    $processId = [int]::Parse($env:CELESVAULT_QUERY_PID)
    if ($processId -le 0) { throw 'Invalid PID' }
    try {
        $process = Get-Process -Id $processId
    } catch {
        if ($_.FullyQualifiedErrorId -ne 'NoProcessFoundForGivenId,Microsoft.PowerShell.Commands.GetProcessCommand') { throw }
        '{"status":"gone"}'
        exit 0
    }
    # Keep Windows FILETIME exact; JSON numbers lose precision in JavaScript.
    $created = $process.StartTime.ToFileTimeUtc().ToString([Globalization.CultureInfo]::InvariantCulture)
    if ($created -cne $env:CELESVAULT_QUERY_CREATED) {
        '{"status":"reused"}'
        exit 0
    }
    $exe = $process.Path
    if ([string]::IsNullOrWhiteSpace($exe)) { throw 'Cannot read process image path' }
    @{ status = 'present'; exe = $exe } | ConvertTo-Json -Compress
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
