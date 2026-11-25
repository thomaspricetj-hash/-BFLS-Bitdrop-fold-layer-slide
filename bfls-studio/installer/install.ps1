# BFLS Compression Studio Installer for Windows
# Run as Administrator

param(
    [string]$InstallDir = "$env:ProgramFiles\BFLS Compression Studio"
)

Write-Host "Installing BFLS Compression Studio..."

# Create install directory
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# Copy binaries - assumes installer is run from folder containing bfls-studio.exe
$SourceExe = Split-Path -Parent $MyInvocation.MyCommand.Definition
$SourceExe = Join-Path $SourceExe "..\target\release\bfls-studio.exe"
if (-not (Test-Path $SourceExe)) {
    # Fallback to current directory
    $SourceExe = Join-Path $PSScriptRoot "bfls-studio.exe"
}

if (-not (Test-Path $SourceExe)) {
    Write-Error "bfls-studio.exe not found. Build release first."
    exit 1
}

Copy-Item -Force $SourceExe "$InstallDir\bfls-studio.exe"
Write-Host "Copied executable to $InstallDir"

# File association
Write-Host "Creating file association for .bfls..."

$ProgId = "BFLS.File"
$ExePath = "`"$InstallDir\bfls-studio.exe`""

# .bfls extension
New-Item -Path "HKCU:\Software\Classes\.bfls" -Force | Out-Null
Set-ItemProperty -Path "HKCU:\Software\Classes\.bfls" -Name "(Default)" -Value $ProgId

# ProgID
New-Item -Path "HKCU:\Software\Classes\$ProgId" -Force | Out-Null
Set-ItemProperty -Path "HKCU:\Software\Classes\$ProgId" -Name "(Default)" -Value "BFLS Compressed File"
New-Item -Path "HKCU:\Software\Classes\$ProgId\DefaultIcon" -Force | Out-Null
Set-ItemProperty -Path "HKCU:\Software\Classes\$ProgId\DefaultIcon" -Name "(Default)" -Value "$ExePath,0"

# Open command
New-Item -Path "HKCU:\Software\Classes\$ProgId\shell\open\command" -Force | Out-Null
Set-ItemProperty -Path "HKCU:\Software\Classes\$ProgId\shell\open\command" -Name "(Default)" -Value "$ExePath `"%1`""

# Add to PATH
$UserPath = [Environment]::GetEnvironmentVariable("PATH","User")
if ($InstallDir -notin $UserPath.Split(';')) {
    $NewPath = $UserPath + ";" + $InstallDir
    [Environment]::SetEnvironmentVariable("PATH","$NewPath","User")
    Write-Host "Added to User PATH"
}

# Create Start Menu shortcut
$StartMenu = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\BFLS Compression Studio"
New-Item -ItemType Directory -Force -Path $StartMenu | Out-Null
$WScriptShell = New-Object -ComObject WScript.Shell
$Shortcut = $WScriptShell.CreateShortcut("$StartMenu\BFLS Compression Studio.lnk")
$Shortcut.TargetPath = "$InstallDir\bfls-studio.exe"
$Shortcut.WorkingDirectory = $InstallDir
$Shortcut.Description = "BFLS Compression Studio"
$Shortcut.Save()
Write-Host "Created Start Menu shortcut"

Write-Host "Installation complete!"
Write-Host "You can now double-click .bfls files to decompress them."
