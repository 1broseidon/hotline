# Install Hotline for Windows from its latest GitHub release.
#
#   irm https://hotline.dev/install.ps1 | iex
#
# A per-user install: no administrator account. Run it again to upgrade.
# macOS and Linux: curl -fsSL https://hotline.dev/install | sh
# Docs: https://hotline.dev/docs/

& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $releases = 'https://github.com/1broseidon/hotline/releases'

    # Windows PowerShell hands back bytes for a download it cannot name as text.
    function Get-Text($url) {
        $content = (Invoke-WebRequest -UseBasicParsing $url).Content
        if ($content -is [byte[]]) { $content = [Text.Encoding]::UTF8.GetString($content) }
        $content
    }

    $version = (Get-Text "$releases/latest/download/latest.json" | ConvertFrom-Json).version
    if (-not $version) { throw 'Could not find the latest Hotline release.' }
    $name = "hotline_${version}_windows_x86_64-setup.exe"
    $base = "$releases/download/desktop-v$version"

    $line = (Get-Text "$base/checksums.txt") -split "`n" | Where-Object { $_ -match "^\w+\s+$([regex]::Escape($name))\s*$" }
    if (-not $line) { throw "Release $version has no $name." }
    $expected = ($line -split '\s+')[0]

    $setup = Join-Path ([IO.Path]::GetTempPath()) $name
    Write-Host "Downloading $name"
    Invoke-WebRequest -UseBasicParsing "$base/$name" -OutFile $setup
    if ((Get-FileHash -Algorithm SHA256 $setup).Hash -ne $expected.ToUpperInvariant()) {
        Remove-Item $setup
        throw "$name does not match its checksum."
    }

    Write-Host "Installing Hotline $version"
    $installer = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
    Remove-Item $setup
    if ($installer.ExitCode -ne 0) { throw "The installer exited with $($installer.ExitCode)." }
    Write-Host ''
    Write-Host "Hotline $version is installed. Open it from the Start menu."
}
