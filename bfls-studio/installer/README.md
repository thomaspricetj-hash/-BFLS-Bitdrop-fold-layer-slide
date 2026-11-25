# BFLS Compression Studio - Windows Installer

## Prerequisites
* Windows 10/11
* PowerShell execution policy allowing scripts
* Built release binary at `target\release\bfls-studio.exe`

## Install
1. Build release:
   ```powershell
   cd D:\compression\bfls-studio
   cargo build --release
   ```
2. Copy `target\release\bfls-studio.exe` to `installer\` or run install script from project root.
3. Run as Administrator:
   ```powershell
   Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
   .\installer\install.ps1
   ```

The installer will:
* Install to `C:\Program Files\BFLS Compression Studio`
* Create Start Menu shortcut
* Associate `.bfls` files with the studio
* Double-clicking a `.bfls` file will auto-decompress it to the same folder

## Uninstall
```powershell
.\installer\uninstall.ps1
```

## Manual file association
If you prefer manual association:
```
HKEY_CURRENT_USER\Software\Classes\.bfls\(Default) = BFLS.File
HKEY_CURRENT_USER\Software\Classes\BFLS.File\shell\open\command\(Default) = "C:\Program Files\BFLS Compression Studio\bfls-studio.exe" "%1"
```

## Packaging
Create a distributable zip:
```powershell
Compress-Archive -Path target\release\bfls-studio.exe,installer\* -DestinationPath bfls-studio-windows.zip
```

This is a Phase 3 deliverable: installer package, file association, and double-click decompress.
