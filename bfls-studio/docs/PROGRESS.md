# BFLS Production Grade Improvements - Progress

## Current State
- Engine: `bfls-studio/src/engine.rs` with BFLS4 format
- Features working: BitDrop, Fold, Layer, Slide, rotation pre-transform, entropy gate, Bloom filter, stage stats, GUI
- Tests passing for round-trip

## Planned Improvements - Full Production Grade

### 1. Global Rule Dictionary with 2-byte IDs
Status: NOT STARTED

### 2. Variable-Length Integer Coding for Metadata
Status: PARTIAL - DISABLED DUE TO BUG
Helpers implemented, magic BFLS4V vs BFLS4 split
Bug: BFLS_S re-wrap loses magic flag causing decompress panic
Workaround: varint disabled for now, using fixed width
Next: fix BFLS_S header to preserve magic / avoid re-wrap

### 3. Huffman / Arithmetic Coding for Core Values
Status: NOT STARTED

### 4. Adaptive Token Size Per Chunk
Status: NOT STARTED

### 5. Better Layering
Status: CURRENT IMPLEMENTATION BASIC

### 6. Longer Entropy-Aware Window
Status: PARTIAL

### 7. Side-Info Free Pre-Transforms
Status: ROTATION IMPLEMENTED

### 8. Final Entropy Coder Selection
Status: CURRENTLY ZSTD ONLY

### 9. Skip Pure Random Early
Status: DONE

### 10. Multi-Level Folding
Status: NOT STARTED

## Notes
- Per-file A/B selection for varint implemented via heuristic, but BFLS_S re-wrap breaks magic detection
- Varint currently disabled to keep round-trip stable
- Next step: fix BFLS_S magic preservation or remove BFLS_S re-wrap
