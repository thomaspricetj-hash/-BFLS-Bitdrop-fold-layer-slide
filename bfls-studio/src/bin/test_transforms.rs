use std::fs;
use bfls_studio::engine::{compress_with_stats, decompress_chunk};

fn main() {
    let files = [
        "text.txt",
        "json.txt",
        "logs.txt",
        "rand.bin",
        "var.bin",
        "rep.bin",
    ];
    let base = "D:/compression/bfls";
    for name in files {
        let path = format!("{}/{}", base, name);
        if let Ok(data) = fs::read(&path) {
            let (comp, stats) = compress_with_stats(&data);
            let decomp = decompress_chunk(&comp);
            let ok = decomp == data;
            println!("{} orig {} comp {} ratio {:.3}% ok {}", name, data.len(), comp.len(), comp.len() as f64 / data.len() as f64 * 100.0, ok);
        }
    }
}
