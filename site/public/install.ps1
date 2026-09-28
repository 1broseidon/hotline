# Install Hotline for Windows from its latest GitHub release.
#
#   irm https://hotline.dev/install.ps1 | iex
#
# With options, use a script block (or save this file and run .\install.ps1):
#   & ([scriptblock]::Create((irm https://hotline.dev/install.ps1))) -Yes -Version 0.26.0
# -Force reinstalls even the same version; -Yes skips confirmation.
# A per-user install: no administrator account.
# macOS and Linux: curl -fsSL https://hotline.dev/install | sh
# Docs: https://hotline.dev/docs/

[CmdletBinding()]
param([string]$Version, [switch]$Yes, [switch]$Force)

function Get-HotlineText($url) {
    # Windows PowerShell hands back bytes for downloads it cannot name as text.
    $content = (Invoke-WebRequest -UseBasicParsing $url).Content
    if ($content -is [byte[]]) { $content = [Text.Encoding]::UTF8.GetString($content) }
    $content
}

function Get-HotlineInstalledVersion {
    # NSIS registers per-user installs here, including a custom install location.
    $roots = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
               'HKCU:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*')
    foreach ($root in $roots) {
        $entry = Get-ItemProperty $root -ErrorAction SilentlyContinue |
            Where-Object { $_.DisplayName -eq 'Hotline' } | Select-Object -First 1
        if ($entry) {
            if ($entry.DisplayVersion) { return $entry.DisplayVersion -replace '^v', '' }
            return 'unknown'
        }
    }
    # An interrupted or portable per-user install may not have an uninstall key.
    foreach ($directory in @('Hotline', 'Programs\Hotline')) {
        $exe = Join-Path $env:LOCALAPPDATA "$directory\hotline-app.exe"
        if (Test-Path $exe) {
            $value = (Get-Item $exe).VersionInfo.ProductVersion
            if ($value) { return $value -replace '^v', '' }
            return 'unknown'
        }
    }
    return $null
}

function Test-HotlineTerminal {
    # iex consumes the downloaded string, not stdin: an interactive console still
    # prompts. Redirected input and unattended hosts must never wait for Read-Host.
    try { return [Environment]::UserInteractive -and -not [Console]::IsInputRedirected }
    catch { return $false }
}

function Install-Hotline {
    [CmdletBinding()]
    param([string]$Version, [switch]$Yes, [switch]$Force)
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $releases = 'https://github.com/1broseidon/hotline/releases'

    if (-not $Version) {
        $Version = (Get-HotlineText "$releases/latest/download/latest.json" | ConvertFrom-Json).version
    }
    if (-not $Version) { throw 'Could not find the latest Hotline release.' }
    $Version = $Version -replace '^v', ''
    if ($Version -notmatch '^\d+\.\d+\.\d+([.-][0-9A-Za-z.-]+)?$') { throw "Invalid version $Version." }
    $current = Get-HotlineInstalledVersion
    if ($current) {
        Write-Host "Hotline $current is installed; $Version is available."
        if ($current -eq $Version -and -not $Force) {
            Write-Host 'Already up to date. Use -Force to reinstall.'
            return
        }
    } else {
        Write-Host "Hotline $Version is available."
    }
    if ($current -and -not $Yes -and (Test-HotlineTerminal)) {
        $answer = Read-Host "Install Hotline $Version? (y/N)"
        if ($answer -notmatch '^(y|yes)$') {
            Write-Host 'Not installed; nothing changed.'
            return
        }
    }

    $name = "hotline_${Version}_windows_x86_64-setup.exe"
    $base = "$releases/download/desktop-v$Version"
    $line = (Get-HotlineText "$base/checksums.txt") -split "`n" |
        Where-Object { $_ -match "^[0-9a-fA-F]{64}\s+$([regex]::Escape($name))\s*$" }
    if (-not $line -or @($line).Count -ne 1) { throw "Release $Version has no unique checksum for $name." }
    $expected = ($line.Trim() -split '\s+')[0]

    # A private directory avoids colliding with another installer download.
    $work = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        $setup = Join-Path $work $name
        Write-Host "Downloading $name"
        Invoke-WebRequest -UseBasicParsing "$base/$name" -OutFile $setup
        if ((Get-FileHash -Algorithm SHA256 $setup).Hash -ne $expected) {
            throw "$name does not match its checksum."
        }
        Write-Host "Installing Hotline $Version"
        $installer = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
        if ($installer.ExitCode -ne 0) { throw "The installer exited with $($installer.ExitCode)." }
    } finally {
        Remove-Item -Recurse -Force $work
    }
    Write-Host ''
    Write-Host "Hotline $Version is installed. Open it from the Start menu."
}

Install-Hotline @PSBoundParameters
