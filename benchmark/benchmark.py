import subprocess
import os
import sys
import time
from pathlib import Path

BFLS = r"bfls\target\release\bfls.exe"
WORK = Path("bfls")

files = [
    "text.txt",
    "code.py",
    "csv.txt",
    "json.txt",
    "logs.txt",
    "rep.bin",
    "rand.bin",
    "big.bin",
    "var.bin",
    "small.txt",
]

def run_cmd(cmd):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True)

def file_size(p):
    return p.stat().st_size

results = []
for name in files:
    p = WORK / name
    if not p.exists():
        continue
    orig = file_size(p)
    # BFLS
    out_bfls = WORK / f"{name}.bfls.tmp"
    t0 = time.perf_counter()
    run_cmd(f'"{BFLS}" compress_tok_auto "{p}" "{out_bfls}"')
    bfls_time = time.perf_counter() - t0
    bfls_size = file_size(out_bfls) if out_bfls.exists() else orig
    # gzip
    gz = WORK / f"{name}.gz.tmp"
    run_cmd(f'gzip -c "{p}" > "{gz}"')
    gz_size = file_size(gz) if gz.exists() else orig
    # zstd
    zst = WORK / f"{name}.zst.tmp"
    run_cmd(f'zstd -q -c -19 "{p}" > "{zst}"')
    zst_size = file_size(zst) if zst.exists() else orig
    # brotli if available
    bro = WORK / f"{name}.br.tmp"
    bro_cmd = run_cmd(f'brotli -q -c "{p}" > "{bro}"')
    bro_size = file_size(bro) if bro.exists() and bro.stat().st_size>0 else None

    results.append({
        "file": name,
        "orig": orig,
        "bfls": bfls_size,
        "bfls_time": bfls_time,
        "gzip": gz_size,
        "zstd": zst_size,
        "brotli": bro_size,
    })
    # cleanup
    for f in [out_bfls, gz, zst, bro]:
        try:
            f.unlink()
        except:
            pass

print(f"{'File':20} {'Orig':>10} {'BFLS':>10} {'BFLS%':>8} {'BFLS ms':>9} {'Gzip':>10} {'Gzip%':>8} {'Zstd':>10} {'Zstd%':>8}")
for r in results:
    orig = r['orig']
    bfls = r['bfls']
    bfls_t = r['bfls_time']
    gz = r['gzip']
    zst = r['zstd']
    bfls_pct = bfls/orig*100
    gz_pct = gz/orig*100
    zst_pct = zst/orig*100
    print(f"{r['file']:20} {orig:10} {bfls:10} {bfls_pct:7.2f}% {bfls_t*1000:8.1f}ms {gz:10} {gz_pct:7.2f}% {zst:10} {zst_pct:7.2f}%")
