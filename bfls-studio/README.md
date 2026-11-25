# BFLS Compression Studio

Desktop GUI for BFLS BitDrop Fold-Layer-Slide Compression.

## Phase 1 & 2 – Working drag-and-drop GUI with engine integration

* Drag & drop files onto window
* Compress → `.bfls`
* Decompress `.bfls` → original
* SHA-256 integrity verification
* Progress bar with ETA
* Advanced statistics panel with stage breakdown: BitDrop, Raw, Fold, Slide, Layer, zstd

The compression engine is now integrated in `src/engine.rs` and called directly from the GUI.

### Build

```bash
cd D:\compression\bfls-studio
cargo build --release
.\target\release\bfls-studio.exe
```

### Project layout

```
bfls-studio/
├─ Cargo.toml
├─ src/
│  ├─ lib.rs
│  ├─ engine.rs   # BFLS engine with StageStats
│  └─ main.rs     # egui/eframe GUI
└─ README.md
```

Phase 2 features: progress bars, ETA, statistics panel, stage accounting.

## Success criteria

* User can open studio, drag file, click Compress, get `.bfls`
* Decompress restores original exactly with SHA-256 check

## Phase 3 – Installer & File Association

See `installer/` for Windows installer scripts, file association, and double-click decompress support.

* `installer/install.ps1` – installs to Program Files, creates Start Menu shortcut, associates `.bfls`
* `installer/uninstall.ps1` – removes installation
* `installer/package.ps1` – builds release and creates distributable zip
* Double-click handler in `src/main.rs` auto-decompresses `.bfls` files passed on command line
