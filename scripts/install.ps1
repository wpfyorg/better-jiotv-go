param(
    [ValidateSet('full', 'slim')][string]$Variant = 'full',
    [string]$Version = 'latest',
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\JioTV'),
    [string]$Repo = 'wpfyorg/better-jiotv-go',
    [switch]$NoTls
)
$ErrorActionPreference = 'Stop'
if ($env:JIOTV_INSTALL_TLS -eq '0') { $NoTls = $true }

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$target = switch ($arch) {
    'X64' { 'x86_64-pc-windows-msvc' }
    'X86' { 'i686-pc-windows-msvc' }
    'Arm64' { 'aarch64-pc-windows-msvc' }
    default { throw "Unsupported Windows architecture: $arch" }
}
$asset = "jiotv-$Variant-$target.exe"
if ($Version -eq 'latest') { $base = "https://github.com/$Repo/releases/latest/download" }
else {
    if (-not $Version.StartsWith('v')) { $Version = "v$Version" }
    $base = "https://github.com/$Repo/releases/download/$Version"
}
$temp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temp | Out-Null
try {
    $binary = Join-Path $temp $asset
    $sums = Join-Path $temp 'SHA256SUMS'
    Invoke-WebRequest -Uri "$base/$asset" -OutFile $binary
    Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile $sums
    $entry = Get-Content $sums | Where-Object { $_ -match "^\s*([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($asset))\s*$" } | Select-Object -First 1
    if (-not $entry) { throw "SHA256SUMS has no entry for $asset" }
    $expected = [regex]::Match($entry, '^\s*([0-9a-fA-F]{64})').Groups[1].Value.ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $binary).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "Checksum mismatch for $asset" }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $destination = Join-Path $InstallDir 'jiotv.exe'
    Copy-Item -LiteralPath $binary -Destination $destination -Force
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @($userPath -split ';' | Where-Object { $_ })
    if (-not ($entries | Where-Object { $_.TrimEnd('\') -ieq $InstallDir.TrimEnd('\') })) {
        [Environment]::SetEnvironmentVariable('Path', (($entries + $InstallDir) -join ';'), 'User')
        $env:Path = "$InstallDir;$env:Path"
    }
    Write-Output "Installed jiotv ($Variant, $target) to $destination"
    if ($NoTls) {
        Write-Output 'Next: jiotv login otp; jiotv admin password; jiotv serve'
        Write-Output 'Browser UI: http://<host>:5001/ (browsers need HTTPS or localhost for DRM and encrypted HLS playback; add --tls to enable HTTPS)'
    }
    else {
        Write-Output 'Next: jiotv login otp; jiotv admin password; jiotv serve --tls'
        Write-Output 'Browser UI (HTTPS, self-signed certificate; accept the one-time warning): https://<host>:5443/'
        Write-Output 'IPTV apps (plain HTTP playlist): http://<host>:5001/'
    }
}
finally { Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue }
