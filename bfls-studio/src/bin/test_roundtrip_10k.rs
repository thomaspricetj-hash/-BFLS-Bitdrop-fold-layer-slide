use bfls_studio::engine::{compress_with_stats, decompress_chunk};

fn lcg(seed: &mut u64) -> u8 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    (*seed >> 56) as u8
}

fn main() {
    let sizes = [0usize,1,2,3,4,63,64,65,255,256,257,1024,4096,65536,1048576];
    let mut passed = 0;
    let mut total = 0;
    for &sz in &sizes {
        for i in 0..1 {
            total += 1;
            let mut seed = 12345u64 + i as u64;
            let data: Vec<u8> = (0..sz).map(|_| lcg(&mut seed)).collect();
            let (comp, _) = compress_with_stats(&data);
            let decomp = decompress_chunk(&comp);
            if decomp == data {
                passed += 1;
            } else {
                println!("FAIL size {}", sz);
            }
        }
    }
    // random vectors
    for i in 0..1 { 
        total += 1;
        let mut seed = 12345u64;
        let sz = 86 as usize;
        let data: Vec<u8> = (0..sz).map(|_| lcg(&mut seed)).collect();
        println!("about to compress size {}, first byte {}", sz, data[0]);
        // quick entropy check
        let mut freq = [0usize;256];
        for &b in &data { freq[b as usize] += 1; }
        let len = data.len() as f64;
        let mut e = 0.0;
        for &c in &freq { if c>0 { let p=c as f64/len; e -= p*p.log2(); } }
        println!("entropy {}", e);
        let (comp, stats) = compress_with_stats(&data);
        println!("compress done");
        println!("comp len {}", comp.len());
        println!("comp first 20 bytes: {:?}", &comp[..comp.len().min(20)]);
        std::fs::write(r"D:\compression\temp_comp.bin", &comp).unwrap();
        let decomp = decompress_chunk(&comp);
        if decomp == data {
            passed += 1;
        } else {
            println!("FAIL random size {}", sz);
        }
    }
    println!("Round-trip: {}/{} passed", passed, total);
}
