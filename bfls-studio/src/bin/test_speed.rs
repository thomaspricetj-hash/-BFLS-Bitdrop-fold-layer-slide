use bfls_studio::engine::{compress_with_stats, decompress_chunk};
use std::time::Instant;

fn main() {
    let files = [
        "D:\\compression\\bfls\\small.txt",
        "D:\\compression\\bfls\\text.txt",
        "D:\\compression\\bfls\\json.txt",
        "D:\\compression\\bfls\\logs.txt",
        "D:\\compression\\bfls\\test10mb.bin",
    ];
    for path in files {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => { println!("{} read error {}", path, e); continue; }
        };
        let t0 = Instant::now();
        let (comp, stats) = compress_with_stats(&data);
        let enc_dur = t0.elapsed();
        println!("comp len {} first 12 bytes: {:?}", comp.len(), &comp[..comp.len().min(12)]);
        let t1 = Instant::now();
        let decomp = decompress_chunk(&comp);
        let dec_dur = t1.elapsed();
        let ok = decomp == data;
        println!("{} | orig {} | comp {} | ratio {:.2}% | enc {:?} | dec {:?} | {}", 
            path.split('\\').last().unwrap(),
            data.len(),
            comp.len(),
            comp.len() as f64 / data.len() as f64 * 100.0,
            enc_dur,
            dec_dur,
            if ok { "PASS" } else { "FAIL" }
        );
    }
}
