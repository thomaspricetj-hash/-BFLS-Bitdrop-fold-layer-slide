use std::env;
use std::fs;
use sha2::{Sha256, Digest};

mod engine;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: bfls <compress|decompress> <input> <output>");
        std::process::exit(1);
    }
    let cmd = args[1].as_str();
    let input = &args[2];
    let output = &args[3];
    let data = fs::read(input).expect("read input");
    match cmd {
        "compress" => {
            let comp = engine::compress_one(&data);
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let checksum = hasher.finalize();
            let mut out = Vec::new();
            out.extend_from_slice(b"BFLS");
            out.push(1);
            out.push(0);
            out.extend_from_slice(&[0u8;2]);
            out.extend_from_slice(&(data.len() as u64).to_le_bytes());
            out.extend_from_slice(&checksum);
            out.extend_from_slice(&(comp.len() as u64).to_le_bytes());
            out.extend_from_slice(&comp);
            let out_len = out.len();
            fs::write(output, &out).expect("write output");
            println!("Compressed {} -> {} bytes", data.len(), out_len);
        }
        "decompress" => {
            let file = fs::read(input).expect("read input");
            if file.len() < 56 || &file[0..4] != b"BFLS" {
                let data = engine::decompress_chunk(&file);
                fs::write(output, &data).expect("write output");
                println!("Decompressed");
                return;
            }
            let payload_size = u64::from_le_bytes(file[48..56].try_into().unwrap()) as usize;
            let payload = &file[56..56+payload_size];
            let data = engine::decompress_chunk(payload);
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let checksum = hasher.finalize();
            let file_checksum = &file[16..48];
            if checksum.as_slice() != file_checksum {
                eprintln!("Checksum mismatch!");
            }
            fs::write(output, &data).expect("write output");
            println!("Decompressed {} -> {} bytes", file.len(), data.len());
        }
        _ => eprintln!("Unknown command"),
    }
}
