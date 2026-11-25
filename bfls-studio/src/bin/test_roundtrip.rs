use bfls_studio::engine;
use sha2::{Sha256, Digest};

fn main() {
    let test_path = "D:\\compression\\bfls\\small.txt";
    let data = std::fs::read(test_path).expect("read test file");
    println!("Original size: {}", data.len());

    let (compressed, stats) = engine::compress_with_stats(&data);
    println!("Compressed size: {}", compressed.len());
    println!("Stats: {:?}", stats);

    let decompressed = engine::decompress_chunk(&compressed);
    println!("Decompressed size: {}", decompressed.len());

    let mut h1 = Sha256::new();
    h1.update(&data);
    let hash1 = hex::encode(h1.finalize());

    let mut h2 = Sha256::new();
    h2.update(&decompressed);
    let hash2 = hex::encode(h2.finalize());

    println!("Original SHA256: {}", hash1);
    println!("Decompressed SHA256: {}", hash2);

    if data == decompressed && hash1 == hash2 {
        println!("PASS: round-trip successful");
    } else {
        println!("FAIL: mismatch");
    }

    // Also test the double-click handler path by writing a .bfls file and decompressing via engine
    let out_path = "D:\\compression\\bfls-studio\\test_small.bfls";
    std::fs::write(out_path, &compressed).expect("write bfls");
    let read_back = std::fs::read(out_path).expect("read bfls");
    let dec2 = engine::decompress_chunk(&read_back);
    assert_eq!(data, dec2);
    println!("File write/read round-trip PASS");
}
