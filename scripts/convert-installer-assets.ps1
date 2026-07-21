[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$installerDirectory = Join-Path (Split-Path -Parent $PSScriptRoot) "src-tauri\installer"

Add-Type -AssemblyName System.Drawing

foreach ($name in @("header", "sidebar")) {
    $sourcePath = Join-Path $installerDirectory "$name.png"
    $destinationPath = Join-Path $installerDirectory "$name.bmp"
    $source = [System.Drawing.Image]::FromFile($sourcePath)

    try {
        $source.Save($destinationPath, [System.Drawing.Imaging.ImageFormat]::Bmp)
    }
    finally {
        $source.Dispose()
    }
}
