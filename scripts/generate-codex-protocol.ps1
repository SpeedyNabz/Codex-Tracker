# Generates stable TypeScript app-server bindings from the installed Codex
# CLI and records the source version in the protocol lock file.
# Made by Heavymask — https://heavymask.com
param(
    [string]$OutputDirectory = "src/codex-protocol"
)

$ErrorActionPreference = "Stop"
$workspaceRoot = Split-Path -Parent $PSScriptRoot
$resolvedOutput = [System.IO.Path]::GetFullPath((Join-Path $workspaceRoot $OutputDirectory))
if (-not $resolvedOutput.StartsWith($workspaceRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Protocol output must remain inside the workspace."
}

$codex = Get-Command codex.exe -ErrorAction Stop
$version = (& $codex.Source --version).Trim()
if ($LASTEXITCODE -ne 0 -or -not $version) {
    throw "Could not determine the installed Codex version."
}

New-Item -ItemType Directory -Force -Path $resolvedOutput | Out-Null
& $codex.Source app-server generate-ts --out $resolvedOutput
if ($LASTEXITCODE -ne 0) {
    throw "Codex protocol generation failed."
}

$lock = [ordered]@{
    codexVersion = $version
    generatedSurface = "stable"
    outputDirectory = $OutputDirectory.Replace("\", "/")
} | ConvertTo-Json
$lockPath = Join-Path $workspaceRoot "codex-protocol.lock.json"
[System.IO.File]::WriteAllText($lockPath, "$lock`n", [System.Text.UTF8Encoding]::new($false))
Write-Output "Generated stable app-server TypeScript bindings for $version in $OutputDirectory"
