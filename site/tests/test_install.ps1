# Run with pwsh -NoProfile -File site/tests/test_install.ps1 (no Pester needed).
# All registry, network, and process-start calls are fakes; only temp files change.
$ErrorActionPreference = 'Stop'
$path = Join-Path $PSScriptRoot '../public/install.ps1'
$tokens = $null
$errors = $null
$null = [Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
$source = (Get-Content -Raw $path) -replace '(?m)^Install-Hotline @PSBoundParameters\s*$', ''
. ([scriptblock]::Create($source))
function Assert($condition, $message) { if (-not $condition) { throw $message } }

# Exercise real detection with isolated registry and file metadata fixtures.
$script:registry = $null
$script:fileExists = $false
$script:fileVersion = $null
$previousLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = [IO.Path]::GetTempPath()
function Get-ItemProperty { param($Path, $ErrorAction); return $script:registry }
function Test-Path { param($Path); return $script:fileExists }
function Get-Item {
    param($Path)
    [pscustomobject]@{ VersionInfo = [pscustomobject]@{ ProductVersion = $script:fileVersion } }
}
try {
    Assert ($null -eq (Get-HotlineInstalledVersion)) 'Fresh install was detected as installed'
    $script:registry = [pscustomobject]@{ DisplayName = 'Hotline'; DisplayVersion = '0.25.0' }
    Assert ((Get-HotlineInstalledVersion) -eq '0.25.0') 'Registry version not detected'
    $script:registry = [pscustomobject]@{ DisplayName = 'Hotline'; DisplayVersion = '' }
    Assert ((Get-HotlineInstalledVersion) -eq 'unknown') 'Missing registry version not unknown'
    $script:registry = $null
    $script:fileExists = $true
    $script:fileVersion = '0.25.0'
    Assert ((Get-HotlineInstalledVersion) -eq '0.25.0') 'Per-user executable not detected'
    $script:fileVersion = $null
    Assert ((Get-HotlineInstalledVersion) -eq 'unknown') 'Missing executable version not unknown'
} finally {
    $env:LOCALAPPDATA = $previousLocalAppData
    Remove-Item Function:Get-ItemProperty, Function:Test-Path, Function:Get-Item
}

$script:current = '0.25.0'
$script:terminal = $true
$script:answer = ''
$script:prompts = 0
$script:downloads = 0
$script:installs = 0
$script:badChecksum = $false
$script:hash = '239f59ed55e737c77147cf55ad0c1b030b6d7ee748a7426952f9b852d5a935e5' # SHA256 of payload
function Get-HotlineInstalledVersion { return $script:current }
function Test-HotlineTerminal { return $script:terminal }
function Read-Host { param($Prompt); $script:prompts++; return $script:answer }
function Get-HotlineText {
    param($url)
    if ($url.EndsWith('latest.json')) { return '{"version":"0.26.0"}' }
    $hash = $script:hash
    if ($script:badChecksum) { $hash = '0' * 64 }
    return "$hash  hotline_0.26.0_windows_x86_64-setup.exe"
}
function Invoke-WebRequest {
    param([switch]$UseBasicParsing, $Uri, $OutFile)
    $script:downloads++
    [IO.File]::WriteAllText($OutFile, 'payload', [Text.UTF8Encoding]::new($false))
}
function Start-Process {
    param($FilePath, $ArgumentList, [switch]$Wait, [switch]$PassThru)
    $script:installs++
    Assert ($ArgumentList -eq '/S') 'Installer must remain per-user silent setup'
    return [pscustomobject]@{ ExitCode = 0 }
}

Install-Hotline
Assert ($script:prompts -eq 1 -and $script:installs -eq 0 -and $script:downloads -eq 0) 'Default answer must decline before downloading'
$script:answer = 'y'
Install-Hotline -Version 0.26.0
Assert ($script:prompts -eq 2 -and $script:installs -eq 1) 'Yes answer must install'
Install-Hotline -Version 0.26.0 -Yes
Assert ($script:prompts -eq 2 -and $script:installs -eq 2) '-Yes must skip the prompt'
$script:terminal = $false
Install-Hotline -Version 0.26.0
Assert ($script:prompts -eq 2 -and $script:installs -eq 3) 'No terminal must proceed'
$script:current = '0.26.0'
Install-Hotline -Version 0.26.0 -Yes
Assert ($script:installs -eq 3) 'Same version must skip even with -Yes'
Install-Hotline -Version 0.26.0 -Force
Assert ($script:installs -eq 4) '-Force must reinstall the same version'
$script:terminal = $true
$script:answer = 'n'
Install-Hotline -Version 0.26.0 -Force
Assert ($script:prompts -eq 3 -and $script:installs -eq 4) '-Force must not imply -Yes'
$script:badChecksum = $true
$rejected = $false
try { Install-Hotline -Version 0.26.0 -Force -Yes } catch {
    $rejected = $_.Exception.Message -match 'does not match its checksum'
}
Assert ($rejected -and $script:installs -eq 4) 'Bad checksum must fail before executing setup'
Write-Host 'PowerShell installer parsing, detection, confirmation, force, and checksum tests passed.'
