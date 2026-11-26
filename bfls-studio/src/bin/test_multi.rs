use bfls_studio::engine;
use sha2::{Sha256, Digest};

fn main() {
    let files = [
        "D:\\compression\\bfls\\small.txt",
        "D:\\compression\\bfls\\text.txt",
        "D:\\compression\\bfls\\json.txt",
        "D:\\compression\\bfls\\logs.txt",
    ];
    for path in files {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => { println!("{}: read error {}", path, e); continue; }
        };
        let (compressed, stats) = engine::compress_with_stats(&data);
        println!("Compressed first 40 bytes: {:?}", &compressed[..compressed.len().min(40)]);
        let decompressed = engine::decompress_chunk(&compressed);
        let mut h1 = Sha256::new();
        h1.update(&data);
        let hash1 = hex::encode(h1.finalize());
        let mut h2 = Sha256::new();
        h2.update(&decompressed);
        let hash2 = hex::encode(h2.finalize());
        let ok = data == decompressed && hash1 == hash2;
        println!("{} | orig {} -> comp {} | ratio {:.2}% | {}", 
            path.split('\\').last().unwrap(),
            data.len(),
            compressed.len(),
            compressed.len() as f64 / data.len() as f64 * 100.0,
            if ok { "PASS" } else { "FAIL" }
        );
        if !ok {
            println!("  SHA mismatch");
        }
    }
}
