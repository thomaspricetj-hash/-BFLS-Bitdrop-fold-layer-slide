# BFLS Compression Studio Uninstaller for Windows
# Run as Administrator

param(
    [string]$InstallDir = "$env:ProgramFiles\BFLS Compression Studio"
)

Write-Host "Uninstalling BFLS Compression Studio..."

# Remove file association
$ProgId = "BFLS.File"
Remove-Item -Path "HKCU:\Software\Classes\.bfls" -ErrorAction SilentlyContinue
Remove-Item -Path "HKCU:\Software\Classes\$ProgId" -Recurse -ErrorAction SilentlyContinue

# Remove Start Menu shortcut
$StartMenu = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\BFLS Compression Studio"
Remove-Item -Path $StartMenu -Recurse -ErrorAction SilentlyContinue

# Remove install directory
if (Test-Path $InstallDir) {
    Remove-Item -Path $InstallDir -Recurse -Force
    Write-Host "Removed $InstallDir"
}

Write-Host "Uninstall complete."
