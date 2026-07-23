[CmdletBinding()]
# Runs the validated frontend test/build pipeline, creates the Windows NSIS
# installer, and copies release artifacts into the repository artifacts folder.
# Made by Heavymask — https://heavymask.com
param(
    [switch]$SkipTests
)

$ErrorActionPreference = "Stop"
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$artifactDirectory = Join-Path $repositoryRoot "artifacts"
$releaseDirectory = Join-Path $repositoryRoot "src-tauri\target\release"
$installerDirectory = Join-Path $releaseDirectory "bundle\nsis"

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Command,

        [Parameter(Mandatory = $false)]
        [string[]]$Arguments = @()
    )

    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code $LASTEXITCODE`: $Command $($Arguments -join ' ')"
    }
}

Push-Location $repositoryRoot
try {
    if (-not $SkipTests) {
        Invoke-CheckedCommand "npm.cmd" @("test")
    }

    Invoke-CheckedCommand "npm.cmd" @("run", "tauri", "--", "build", "--bundles", "nsis")

    $standaloneExecutable = Join-Path $releaseDirectory "codex-tracker.exe"
    if (-not (Test-Path -LiteralPath $standaloneExecutable)) {
        throw "The release executable was not created: $standaloneExecutable"
    }

    $installer = Get-ChildItem -LiteralPath $installerDirectory -Filter "*-setup.exe" -File |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($null -eq $installer) {
        throw "The NSIS installer was not created in: $installerDirectory"
    }

    New-Item -ItemType Directory -Path $artifactDirectory -Force | Out-Null
    $standaloneArtifact = Join-Path $artifactDirectory "Codex Tracker.exe"
    Copy-Item -LiteralPath $standaloneExecutable -Destination $standaloneArtifact -Force
    Copy-Item -LiteralPath $installer.FullName -Destination (Join-Path $artifactDirectory $installer.Name) -Force

    Write-Host "Build complete."
    Write-Host "Standalone executable: $standaloneArtifact"
    Write-Host "Installer: $($installer.FullName)"
}
finally {
    Pop-Location
}
