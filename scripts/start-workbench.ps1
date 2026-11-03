param([int]$Port = 8765, [switch]$NoBrowser)
$ErrorActionPreference = 'Stop'
$WorkbenchRoot = Split-Path -Parent $PSScriptRoot
$WorkbenchBinary = Join-Path $WorkbenchRoot 'serial-workbench.exe'
if (-not (Test-Path -LiteralPath $WorkbenchBinary)) {
    $WorkbenchBinary = Join-Path $WorkbenchRoot 'target\release\serial-workbench.exe'
}
$WorkbenchData = Join-Path $WorkbenchRoot 'workbench-data'
$WorkbenchUrl = "http://127.0.0.1:$Port"
function Get-WorkbenchStatus {
    # Explicitly bypass proxies; compatible with Windows PowerShell 5.1 and PowerShell 7.
    $WorkbenchRequest = [System.Net.WebRequest]::Create("$WorkbenchUrl/api/status")
    $WorkbenchRequest.Proxy = $null
    $WorkbenchRequest.Timeout = 2000
    $WorkbenchResponse = $WorkbenchRequest.GetResponse()
    try {
        $WorkbenchReader = [System.IO.StreamReader]::new($WorkbenchResponse.GetResponseStream())
        try { return ($WorkbenchReader.ReadToEnd() | ConvertFrom-Json) }
        finally { $WorkbenchReader.Dispose() }
    } finally { $WorkbenchResponse.Dispose() }
}
$WorkbenchStatus = $null
try { $WorkbenchStatus = Get-WorkbenchStatus } catch { }
if ($WorkbenchStatus -and $WorkbenchStatus.name -ne 'Serial Workbench') {
    throw 'The selected port belongs to another application.'
}
if (-not $WorkbenchStatus) {
    if (-not (Test-Path -LiteralPath $WorkbenchBinary)) {
        if (-not (Test-Path -LiteralPath (Join-Path $WorkbenchRoot 'Cargo.toml'))) {
            throw 'serial-workbench.exe is missing. Extract the complete release archive first.'
        }
        Push-Location $WorkbenchRoot
        try {
            & cargo build --release --locked --bin serial-workbench
            if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
        } finally { Pop-Location }
    }
    New-Item -ItemType Directory -Force -Path $WorkbenchData | Out-Null
    Start-Process -FilePath $WorkbenchBinary -ArgumentList @('serve', '--port', "$Port", '--data-dir', "`"$WorkbenchData`"") -WorkingDirectory $WorkbenchRoot -WindowStyle Hidden -RedirectStandardOutput (Join-Path $WorkbenchData 'daemon.stdout.log') -RedirectStandardError (Join-Path $WorkbenchData 'daemon.stderr.log')
    $WorkbenchReady = $false
    for ($WorkbenchAttempt = 0; $WorkbenchAttempt -lt 40; $WorkbenchAttempt++) {
        try {
            $WorkbenchStatus = Get-WorkbenchStatus
            if ($WorkbenchStatus.name -eq 'Serial Workbench') { $WorkbenchReady = $true; break }
        } catch { }
        Start-Sleep -Milliseconds 100
    }
    if (-not $WorkbenchReady) { throw 'Workbench did not start; see workbench-data/daemon.stderr.log.' }
}
if (-not $NoBrowser) { Start-Process $WorkbenchUrl }
