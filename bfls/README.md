# BFLS - BitDrop Fold-Layer-Slide Compression

Lossless compression prototype in Rust implementing the BitDrop Fold-Layer-Slide research idea.

## Core Idea

Plain Text
DATA
↓
BitDrop
↓
Canonical Core
↓
Fold
↓
Layer
↓
Slide
↓
Store Rules

Compressed file contains:
- HEADER
- CORE PATTERNS
- RULE DICTIONARY
- RECONSTRUCTION MAP

## Implementation v2

- **BitDrop (bit-level)**: converts bytes → bit stream, removes consecutive duplicate bits, stores core + run lengths. Lossless reconstruction.
- **Fold reduction**: detects reverse fold `core = A + reverse(A)`. Stores only `A` + rule_id `Reverse`, expands on decompress.
- **Rule Dictionary**: extensible, first rule is Reverse.
- **Core collapse experiment**: measures distinct canonical cores.

## Build

```bash
cargo build --release
```

## Usage

Experiment:
```bash
target/release/bfls.exe experiment 10000 64
```

Benchmark:
```bash
target/release/bfls.exe benchmark <file>
```

Compress / Decompress:
```bash
target/release/bfls.exe compress <in> <out.bfls>
target/release/bfls.exe decompress <in.bfls> <out>
```

Sliding window mode:
```bash
target/release/bfls.exe compress_sw <in> <out.bfls> <chunk_size> <window_chunks>
target/release/bfls.exe decompress_sw <in.bfls> <out>
```

## Most Important Experiment

Bit-level BitDrop collapse:

```
Core collapse experiment (bit-level BitDrop)
  inputs: 10000
  length bytes: 64
  bits per input: 512
  distinct cores: 153
  collapse ratio: 0.015300
  average run count per input: 256.35
```

10 000 random inputs → 153 distinct cores. Collapse ratio ~1.5%.

Fold reduction example:
Input `0xAA 0x55` → bitstream `10101010 01010101`
BitDrop removes boundary duplicate → core 15 bits, runs 15.
When reverse fold is detected, core is halved and stored with rule_id=Reverse, cutting storage.

## Format

`BFLS2` magic
- original_size_bytes u64
- core_len u32
- runs_len u32
- rule_id u8  (255=none, 0=Reverse)
- core bytes
- runs u32[]

## Sliding Window Results

For repetitive data `big.bin` 102400 bytes:
```
sw compressed 102400 -> 13171 bytes, ratio 0.129, time 3.5ms
```

Sliding window reuses BitDrop cores within a window, giving ~7.7× compression vs raw BitDrop on repetitive data. For random data compression expands as expected.

## Next Steps

- Layering via Hamming diff and Slide via cyclic offset detection
- Full rule dictionary serialization
- Benchmark vs gzip/brotli/zstd/lzma
- Streaming API and bit-packing optimization
