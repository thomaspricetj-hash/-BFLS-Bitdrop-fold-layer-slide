# BFLS Compression Project

This workspace contains two implementations of BitDrop Fold-Layer-Slide compression:

* `bfls/` – Rust CLI prototype / research engine
* `bfls-studio/` – Desktop GUI application for non-technical users

---

## bfls – CLI Prototype

Research prototype with the full BitDrop + Fold + Layer + Slide pipeline, stage accounting, benchmarking, and round-trip testing.

### What it does
Plain Text → BitDrop → Canonical Core → Fold → Layer → Slide → Rule Dictionary → zstd → `.bfls`

* **BitDrop** – tokenizes input and removes runs, producing core patterns + run lengths
* **Fold** – detects identical core+run pairs within a sliding window and stores a reference
* **Layer** – Hamming-distance based differential storage against recent chunks
* **Slide** – canonical rotation + offset detection for shifted patterns
* **Rule Dictionary** – reusable rule IDs, Bloom filter pre-filter for speed
* **Entropy coding** – final payload is zstd compressed

File format:
* `BFLS4` – full sliding-window format with per-chunk tags 0=raw,1=ref,2=slide,3=layer,4=rle
* `BFLS_S` – compact small-file header
* `BFLS_RAW` – fallback when compression would expand

### Run it
```powershell
cd D:\compression\bfls
cargo build --release
.\target\release\bfls.exe compress input.txt output.bfls
.\target\release\bfls.exe decompress input.bfls output.txt
.\target\release\bfls.exe benchmark input.txt
.\target\release\bfls.exe inspect input.bfls
```

CLI also supports:
* `bfls c file.txt` / `bfls d file.bfls`
* Self-test and round-trip harness via `cargo test`
* Stage-by-stage accounting: Original → BitDrop → Fold/Layer/Slide → zstd → Final

Benchmark corpus is in `bfls/` with text, code, json, csv, logs, random, etc.

### How it works
1. **Tokenize** input bytes with adaptive token size based on entropy
2. **Chunk** tokens into windows. Window and chunk sizes auto-tune by entropy and file size
3. **BitDrop** each chunk to core+runs
4. **Reference** detection: same core+run → fold ref; canonical rotation → slide; Hamming diff → layer
5. **Encode** tags + payload, then zstd
6. **Decompress** reconstructs tokens via detokenize, verifies lossless via SHA-256

The prototype prioritizes correctness > stability > compatibility > performance.

---

## bfls-studio – Desktop GUI

Production-ready desktop app built with egui + eframe. Reuses the same Rust engine as `bfls/` but presents a drag-and-drop UI.

### Features
* Drag & drop files onto window
* Compress → `.bfls`
* Decompress `.bfls` → original with SHA-256 verification
* Progress bar with ETA
* Advanced statistics panel with stage breakdown:
  BitDrop size, Raw payload, Fold refs, Slide refs, Layer refs, Fold/Layer/Slide payload, zstd size, Final size
* Double-click handler: launching `bfls-studio.exe file.bfls` auto-decompresses
* Installer scripts for Windows file association

### Run it
```powershell
cd D:\compression\bfls-studio
cargo build --release
.\target\release\bfls-studio.exe
```

**GUI workflow**
1. Open BFLS Compression Studio
2. Drag files into the drop zone
3. Click `Compress` → `.bfls` files created next to sources
4. Later select `.bfls` files → Click `Decompress` → originals restored exactly

**Command-line double-click**
```powershell
.\target\release\bfls-studio.exe path\to\file.bfls
```
Will silently decompress to the same folder.

### Installer
```powershell
cd D:\compression\bfls-studio\installer
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
.\package.ps1   # builds release and creates bfls-studio-windows.zip
.\install.ps1   # installs to Program Files, creates Start Menu shortcut, associates .bfls
.\uninstall.ps1 # removes installation
```

File association created under HKCU:
```
.bfls → BFLS.File
BFLS.File\shell\open\command = "C:\Program Files\BFLS Compression Studio\bfls-studio.exe" "%1"
```

### How it works
The studio re-exports the engine from `src/engine.rs`:
* `compress_with_stats(&[u8]) -> (Vec<u8>, StageStats)`
* `decompress_chunk(&[u8]) -> Vec<u8>`
* `entropy(&[u8]) -> f64`

GUI calls the same functions as the CLI prototype, so results are identical. StageStats is populated by parsing the SW payload for raw/fold/slide/layer byte counts, providing the detailed breakdown shown in the Advanced panel.

---

## Project structure

```
D:\compression\
├─ bfls/            # CLI prototype, research engine, benchmarks
│  ├─ src/main.rs
│  ├─ tests/
│  └─ Cargo.toml
├─ bfls-studio/     # Desktop GUI
│  ├─ src/
│  │  ├─ lib.rs
│  │  ├─ engine.rs   # BFLS engine with StageStats
│  │  └─ main.rs     # egui/eframe UI
│  ├─ installer/
│  │  ├─ install.ps1
│  │  ├─ uninstall.ps1
│  │  ├─ package.ps1
│  │  └─ README.md
│  └─ Cargo.toml
└─ README.md        # this file
```

Both projects share the BitDrop Fold-Layer-Slide design, but `bfls-studio` is the user-facing product with installer, file association, and GUI.

## Testing

Round-trip verified on:
* small.txt 13 000 → 63 B
* text.txt 490 000 → 502 B
* json.txt 548 891 → 20 977 B
* logs.txt 1 080 000 → 362 B

All decompress to identical SHA-256.

---

For research details, see `bfls/README.md`. For distribution, use `bfls-studio/installer/package.ps1`.
