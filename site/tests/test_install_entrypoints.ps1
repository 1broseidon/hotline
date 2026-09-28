# Offline tests of the complete advertised entry points, not just the functions.
$ErrorActionPreference = 'Stop'
$source = Get-Content (Join-Path $PSScriptRoot '../public/install.ps1') -Raw
$script:installedFixtureVersion = '0.26.0'
$script:requests = @()
$script:installs = 0
$bytes = [Text.Encoding]::UTF8.GetBytes('isolated setup fixture')
$hash = [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($bytes)).Replace('-', '').ToLowerInvariant()

function Invoke-WebRequest {
    param([Parameter(Position=0)]$Uri, [switch]$UseBasicParsing, $OutFile)
    $script:requests += $Uri
    if ($OutFile) {
        [IO.File]::WriteAllBytes($OutFile, $bytes)
        return
    }
    if ($Uri.EndsWith('/latest.json')) {
        return [PSCustomObject]@{ Content = '{"version":"0.26.0"}' }
    }
    if ($Uri.EndsWith('/checksums.txt')) {
        return [PSCustomObject]@{ Content = "$hash  hotline_0.26.0_windows_x86_64-setup.exe" }
    }
    throw "Unexpected download: $Uri"
}
function Get-ItemProperty {
    param($Path, $ErrorAction)
    [PSCustomObject]@{ DisplayName = 'Hotline'; DisplayVersion = $script:installedFixtureVersion }
}
function Start-Process {
    param($FilePath, $ArgumentList, [switch]$Wait, [switch]$PassThru)
    if (-not (Test-Path $FilePath)) { throw 'Setup download missing' }
    $script:installs++
    [PSCustomObject]@{ ExitCode = 0 }
}
function Read-Host { throw 'These invocations must not prompt' }

# The downloaded text piped through iex: current version returns before a prompt.
$source | Invoke-Expression
if ($script:installs -ne 0 -or $script:requests.Count -ne 1) { throw 'iex entry point did not skip the current release' }

# A saved/script-block invocation forwards the explicit version and -Yes.
$script:installedFixtureVersion = '0.25.0'
$script:requests = @()
& ([scriptblock]::Create($source)) -Version 0.26.0 -Yes
if ($script:installs -ne 1) { throw 'Script-block entry point did not install' }
if ($script:requests | Where-Object { $_.EndsWith('/latest.json') }) { throw '-Version was not forwarded' }

$script:installedFixtureVersion = '0.26.0'
& ([scriptblock]::Create($source)) -Version 0.26.0 -Force -Yes
if ($script:installs -ne 2) { throw '-Force was not forwarded' }
Write-Host 'PowerShell iex and script-block option forwarding passed.'
