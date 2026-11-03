param([int]$Port = 8765)
$ErrorActionPreference = 'Stop'
$WorkbenchRoot = Split-Path -Parent $PSScriptRoot
$WorkbenchBinary = Join-Path $WorkbenchRoot 'target\release\serial-workbench.exe'
$WorkbenchData = Join-Path $WorkbenchRoot 'workbench-data'
$WorkbenchUrl = "http://127.0.0.1:$Port"
try {
    $Status = Invoke-RestMethod -Uri "$WorkbenchUrl/api/status" -TimeoutSec 2 -NoProxy
    if ($Status.name -ne 'Serial Workbench') { throw 'The selected port belongs to another application.' }
} catch {
    if (-not (Test-Path -LiteralPath $WorkbenchBinary)) {
        Push-Location $WorkbenchRoot
        try {
            & cargo build --release --bin serial-workbench
            if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
        } finally { Pop-Location }
    }
    New-Item -ItemType Directory -Force -Path $WorkbenchData | Out-Null
    Start-Process -FilePath $WorkbenchBinary -ArgumentList @('serve', '--port', "$Port", '--data-dir', "`"$WorkbenchData`"") -WorkingDirectory $WorkbenchRoot -WindowStyle Hidden -RedirectStandardOutput (Join-Path $WorkbenchData 'daemon.stdout.log') -RedirectStandardError (Join-Path $WorkbenchData 'daemon.stderr.log')
    $Ready = $false
    for ($Attempt = 0; $Attempt -lt 40; $Attempt++) {
        try {
            $Status = Invoke-RestMethod -Uri "$WorkbenchUrl/api/status" -TimeoutSec 1 -NoProxy
            if ($Status.name -eq 'Serial Workbench') { $Ready = $true; break }
        } catch { }
        Start-Sleep -Milliseconds 100
    }
    if (-not $Ready) { throw 'Workbench did not start; see workbench-data/daemon.stderr.log.' }
}
Start-Process $WorkbenchUrl
