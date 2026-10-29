$ErrorActionPreference = 'Stop'

$installDir = Join-Path $env:RUNNER_TEMP 'jiotv-installer-smoke'
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$processPath = $env:Path
$previousOnboard = $env:JIOTV_ONBOARD
$previousVariant = $env:JIOTV_VARIANT

try {
    $env:JIOTV_ONBOARD = 'never'
    $env:JIOTV_VARIANT = 'slim'
    $output = & (Join-Path $PSScriptRoot '..\install.ps1') -Version '1.4.0' -InstallDir $installDir -NoTls 6>&1 | Out-String

    $installed = Join-Path $installDir 'jiotv.exe'
    if (-not (Test-Path -LiteralPath $installed -PathType Leaf)) {
        throw 'The Windows installer did not place jiotv.exe in the requested directory.'
    }
    if ($output -notmatch 'Installed jiotv \(slim,') {
        throw 'JIOTV_VARIANT=slim did not select the slim Windows asset.'
    }

    $help = & $installed --help 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0 -or $help -notmatch 'USAGE:') {
        throw 'The installed Windows executable did not pass its help smoke test.'
    }
}
finally {
    [Environment]::SetEnvironmentVariable('Path', $userPath, 'User')
    $env:Path = $processPath
    if ($null -eq $previousOnboard) { Remove-Item Env:JIOTV_ONBOARD -ErrorAction SilentlyContinue }
    else { $env:JIOTV_ONBOARD = $previousOnboard }
    if ($null -eq $previousVariant) { Remove-Item Env:JIOTV_VARIANT -ErrorAction SilentlyContinue }
    else { $env:JIOTV_VARIANT = $previousVariant }
    Remove-Item -LiteralPath $installDir -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host 'Windows installer smoke test OK'
