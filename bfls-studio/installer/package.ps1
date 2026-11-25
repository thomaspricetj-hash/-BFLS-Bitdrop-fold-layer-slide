# Build and package BFLS Studio for Windows distribution
Set-StrictMode -Version Latest

$ProjectRoot = Split-Path -Parent $MyInvocation.MyCommand.Definition
$ProjectRoot = Join-Path $ProjectRoot ".."
Set-Location $ProjectRoot

Write-Host "Building release..."
cargo build --release

$ExePath = Join-Path $ProjectRoot "target\release\bfls-studio.exe"
if (-not (Test-Path $ExePath)) {
    Write-Error "Build failed - exe not found"
    exit 1
}

# Copy exe to installer folder
$InstallerDir = Join-Path $ProjectRoot "installer"
Copy-Item -Force $ExePath (Join-Path $InstallerDir "bfls-studio.exe")

# Create distribution zip
$ZipPath = Join-Path $ProjectRoot "bfls-studio-windows.zip"
if (Test-Path $ZipPath) { Remove-Item $ZipPath }

Compress-Archive -Path $ExePath, (Join-Path $InstallerDir "install.ps1"), (Join-Path $InstallerDir "uninstall.ps1"), (Join-Path $InstallerDir "README.md") -DestinationPath $ZipPath

Write-Host "Package created: $ZipPath"
Write-Host "Size: $((Get-Item $ZipPath).Length / 1MB) MB"
