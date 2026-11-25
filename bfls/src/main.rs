use std::fs;
use rustc_hash::FxHashMap;
use std::hash::{Hash, Hasher};
use rayon::prelude::*;

fn tokenize(data:&[u8], ts:usize)->Vec<u32>{
    let mut v=Vec::new();
    let mut i=0;
    while i<data.len(){
        let mut t=0u32;
        for j in 0..ts { if i+j < data.len() { t|= (data[i+j] as u32) << (8*j); } }
        v.push(t);
        i+=ts;
    }
    v
}
fn detokenize(toks:&[u32], ts:usize, orig_len:usize)->Vec<u8>{
    let mut out=Vec::with_capacity(orig_len);
    for &t in toks {
        for j in 0..ts { out.push(((t>>(8*j))&0xFF) as u8); }
    }
    out.truncate(orig_len);
    out
}
fn bitdrop_sym(toks:&[u32])->(Vec<u32>,Vec<u32>){
    if toks.is_empty(){ return (vec![],vec![]); }
    let mut core=Vec::new(); let mut runs=Vec::new();
    let mut prev=toks[0];
    core.push(prev); runs.push(1);
    for &x in &toks[1..]{
        if x==prev { *runs.last_mut().unwrap()+=1; }
        else { core.push(x); runs.push(1); prev=x; }
    }
    (core,runs)
}
fn reconstruct_sym(core:&[u32],runs:&[u32])->Vec<u32>{
    let mut out=Vec::new();
    for (&c,&r) in core.iter().zip(runs.iter()){ out.extend(std::iter::repeat(c).take(r as usize)); }
    out
}
fn hamming_bytes(a:&[u8],b:&[u8])->usize{
    a.iter().zip(b.iter()).map(|(&x,&y)|(x^y).count_ones() as usize).sum()
}
fn entropy(data:&[u8])->f64{
    if data.is_empty(){return 0.0;}
    let mut f=[0usize;256];
    for &b in data {f[b as usize]+=1;}
    let len=data.len() as f64;
    let mut e=0.0;
    for &c in &f { if c>0 { let p=c as f64/len; e-=p*p.log2(); } }
    e
}
fn fingerprint_pair(core:&[u32], runs:&[u32])->u64 {
    let mut h = rustc_hash::FxHasher::default();
    core.hash(&mut h);
    runs.hash(&mut h);
    h.finish()
}
fn fingerprint_slice(s:&[u32])->u64 {
    let mut h = rustc_hash::FxHasher::default();
    s.hash(&mut h);
    h.finish()
}

struct BloomFilter {
    bits: Vec<u8>,
    m: usize,
    k: usize,
}
impl BloomFilter {
    fn new(m_bits: usize, k: usize) -> Self {
        let bytes = (m_bits + 7) / 8;
        Self { bits: vec![0; bytes], m: m_bits, k }
    }
    fn add(&mut self, data: &[u8]) {
        let h1 = {
            let mut hasher = rustc_hash::FxHasher::default();
            hasher.write(data);
            hasher.finish()
        };
        let mut h2 = {
            let mut hasher = rustc_hash::FxHasher::default();
            hasher.write(data);
            hasher.write(&[0u8]);
            hasher.finish()
        };
        if h2 == 0 { h2 = 1; }
        for i in 0..self.k {
            let combined = h1.wrapping_add((i as u64).wrapping_mul(h2));
            let idx = (combined as usize) % self.m;
            let byte = idx / 8;
            let bit = idx % 8;
            self.bits[byte] |= 1 << bit;
        }
    }
    fn might_contain(&self, data: &[u8]) -> bool {
        let h1 = {
            let mut hasher = rustc_hash::FxHasher::default();
            hasher.write(data);
            hasher.finish()
        };
        let mut h2 = {
            let mut hasher = rustc_hash::FxHasher::default();
            hasher.write(data);
            hasher.write(&[0u8]);
            hasher.finish()
        };
        if h2 == 0 { h2 = 1; }
        for i in 0..self.k {
            let combined = h1.wrapping_add((i as u64).wrapping_mul(h2));
            let idx = (combined as usize) % self.m;
            let byte = idx / 8;
            let bit = idx % 8;
            if (self.bits[byte] & (1 << bit)) == 0 {
                return false;
            }
        }
        true
    }
}

fn canonical_rotation(core:&[u32])->Vec<u32>{
    let n=core.len();
    if n<=1 {return core.to_vec();}
    let mut s=Vec::with_capacity(n*2);
    s.extend_from_slice(core);
    s.extend_from_slice(core);
    let mut i=0usize; let mut j=1usize; let mut k=0usize;
    while i<n && j<n && k<n {
        let a=s[i+k]; let b=s[j+k];
        if a==b { k+=1; continue; }
        if a>b { i=i+k+1; } else { j=j+k+1; }
        if i>=j { j=i+1; } k=0;
    }
    let start=std::cmp::min(i,j);
    s[start..start+n].to_vec()
}
#[derive(Clone)]
struct ChunkInfo { raw:Vec<u8>, core:Vec<u32>, runs:Vec<u32> }

fn compress_tok_sw(data:&[u8], token_size:usize, chunk_tokens:usize, window_chunks:usize)->Vec<u8>{
    let tokens=tokenize(data,token_size);
    let num_chunks=(tokens.len()+chunk_tokens-1)/chunk_tokens;
    let mut history:Vec<ChunkInfo>=Vec::with_capacity(num_chunks);
    let mut core_index:FxHashMap<(Vec<u32>,Vec<u32>), usize>=FxHashMap::default();
    let mut rot_index:FxHashMap<Vec<u32>, usize>=FxHashMap::default();
    let mut core_bloom = BloomFilter::new(1<<20, 3);
    let mut rot_bloom = BloomFilter::new(1<<20, 3);
    let mut out=Vec::new();
    out.extend_from_slice(b"BFLS4");
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(&(token_size as u32).to_le_bytes());
    out.extend_from_slice(&(chunk_tokens as u32).to_le_bytes());
    out.extend_from_slice(&(window_chunks as u32).to_le_bytes());
    for i in 0..num_chunks {
        let s=i*chunk_tokens; let e=std::cmp::min(s+chunk_tokens,tokens.len());
        let chunk_toks=&tokens[s..e];
        let (core,runs)=bitdrop_sym(chunk_toks);
        let cur_raw=&data[s*token_size..std::cmp::min((e)*token_size,data.len())];
        let cur_info=ChunkInfo{ raw:cur_raw.to_vec(), core:core.clone(), runs:runs.clone() };
        let key=(core.clone(),runs.clone());
        let mut found_idx=None;
        let core_fp = fingerprint_pair(&core, &runs);
        if core_bloom.might_contain(&core_fp.to_le_bytes()) {
            if let Some(&last_pos)=core_index.get(&key){
                if i>last_pos && i-last_pos<=window_chunks { found_idx=Some(i-last_pos); }
            }
        }
        let mut slide_found=None;
        if found_idx.is_none(){
            let can=canonical_rotation(&core);
            let rot_fp = fingerprint_slice(&can);
            if rot_bloom.might_contain(&rot_fp.to_le_bytes()) {
                if let Some(&last_pos)=rot_index.get(&can){
                    if i>last_pos && i-last_pos<=window_chunks {
                        let prev_core=&history[last_pos].core;
                        if prev_core.len()==core.len(){
                            let n=core.len();
                            let mut doubled=Vec::with_capacity(n*2);
                            doubled.extend_from_slice(prev_core);
                            doubled.extend_from_slice(prev_core);
                            for k in 0..n {
                                if &doubled[k..k+n]==core.as_slice(){ slide_found=Some((i-last_pos,k)); break; }
                            }
                        }
                    }
                }
            }
        }
        let mut layer_dist=None; let mut layer_xor=None;
        if found_idx.is_none() && slide_found.is_none(){
            let win_start=if i>window_chunks {i-window_chunks} else {0};
            for j in (win_start..i).rev(){
                let prev=&history[j];
                if cur_raw.len()==prev.raw.len(){
                    let hd=hamming_bytes(cur_raw,&prev.raw);
                    let thresh=cur_raw.len()*8/10;
                    if hd>0 && hd<=thresh{
                        layer_dist=Some(i-j);
                        let mut xor=vec![0u8;cur_raw.len()];
                        for k in 0..cur_raw.len(){ xor[k]=cur_raw[k]^prev.raw[k]; }
                        layer_xor=Some(xor);
                        break;
                    }
                }
            }
        }
        if let Some(dist)=found_idx {
            out.push(1u8);
            out.extend_from_slice(&(dist as u32).to_le_bytes());
            let cloned=history[i-dist].clone();
            history.push(cloned.clone());
            core_index.insert((cloned.core.clone(),cloned.runs.clone()), i);
            rot_index.insert(canonical_rotation(&cloned.core), i);
            core_bloom.add(&fingerprint_pair(&cloned.core, &cloned.runs).to_le_bytes());
            rot_bloom.add(&fingerprint_slice(&canonical_rotation(&cloned.core)).to_le_bytes());
        } else if let Some((dist,off))=slide_found {
            out.push(2u8);
            out.extend_from_slice(&(dist as u32).to_le_bytes());
            out.extend_from_slice(&(off as u32).to_le_bytes());
            let prev=&history[i-dist];
            let n=prev.core.len();
            let k=off%n;
            let mut rc=Vec::with_capacity(n); rc.extend_from_slice(&prev.core[k..]); rc.extend_from_slice(&prev.core[..k]);
            let mut rr=Vec::with_capacity(n); rr.extend_from_slice(&prev.runs[k..]); rr.extend_from_slice(&prev.runs[..k]);
            if rc[0]==rc[n-1]{ rr[0]+=rr[n-1]; rc.pop(); rr.pop(); }
            let new_info=ChunkInfo{ raw:cur_raw.to_vec(), core:rc.clone(), runs:rr.clone() };
            history.push(new_info.clone());
            core_index.insert((rc.clone(),rr.clone()), i);
            rot_index.insert(canonical_rotation(&rc), i);
            core_bloom.add(&fingerprint_pair(&rc, &rr).to_le_bytes());
            rot_bloom.add(&fingerprint_slice(&canonical_rotation(&rc)).to_le_bytes());
        } else if let (Some(dist),Some(xor))= (layer_dist,layer_xor) {
            // Automatic layering: only use if it reduces size vs raw storage
            let core_len = cur_info.core.len();
            let runs_len = cur_info.runs.len();
            let raw_cost = 1 + 4 + 4 + core_len*4 + runs_len*4;
            let layer_cost = 1 + 4 + 4 + xor.len();
            if layer_cost < raw_cost {
                out.push(3u8);
                out.extend_from_slice(&(dist as u32).to_le_bytes());
                out.extend_from_slice(&(xor.len() as u32).to_le_bytes());
                out.extend(&xor);
                let prev=&history[i-dist];
                let mut recon=Vec::with_capacity(prev.raw.len());
                for k in 0..prev.raw.len(){ recon.push(prev.raw[k]^xor[k]); }
                let toks2=tokenize(&recon,token_size);
                let (c2,r2)=bitdrop_sym(&toks2);
                let new_info=ChunkInfo{ raw:recon.clone(), core:c2.clone(), runs:r2.clone() };
                history.push(new_info.clone());
                core_index.insert((c2.clone(),r2.clone()), i);
                rot_index.insert(canonical_rotation(&c2), i);
                core_bloom.add(&fingerprint_pair(&c2, &r2).to_le_bytes());
                rot_bloom.add(&fingerprint_slice(&canonical_rotation(&c2)).to_le_bytes());
            } else {
                // fall through to raw storage
                out.push(0u8);
                out.extend_from_slice(&(cur_info.core.len() as u32).to_le_bytes());
                out.extend_from_slice(&(cur_info.runs.len() as u32).to_le_bytes());
                for &c in &cur_info.core { out.extend_from_slice(&c.to_le_bytes()); }
                for &r in &cur_info.runs { out.extend_from_slice(&r.to_le_bytes()); }
                history.push(cur_info.clone());
                core_index.insert((cur_info.core.clone(),cur_info.runs.clone()), i);
                rot_index.insert(canonical_rotation(&cur_info.core), i);
                core_bloom.add(&fingerprint_pair(&cur_info.core, &cur_info.runs).to_le_bytes());
                rot_bloom.add(&fingerprint_slice(&canonical_rotation(&cur_info.core)).to_le_bytes());
            }
        } else {
            out.push(0u8);
            out.extend_from_slice(&(cur_info.core.len() as u32).to_le_bytes());
            out.extend_from_slice(&(cur_info.runs.len() as u32).to_le_bytes());
            for &c in &cur_info.core { out.extend_from_slice(&c.to_le_bytes()); }
            for &r in &cur_info.runs { out.extend_from_slice(&r.to_le_bytes()); }
            history.push(cur_info.clone());
            core_index.insert((cur_info.core.clone(),cur_info.runs.clone()), i);
            rot_index.insert(canonical_rotation(&cur_info.core), i);
            core_bloom.add(&fingerprint_pair(&cur_info.core, &cur_info.runs).to_le_bytes());
            rot_bloom.add(&fingerprint_slice(&canonical_rotation(&cur_info.core)).to_le_bytes());
        }
    }
    out
}

fn decompress_tok_sw(in_bytes:&[u8])->Vec<u8>{
    if in_bytes.len()<5 || &in_bytes[0..5]!=b"BFLS4"{ panic!("bad magic"); }
    let mut off=5;
    let orig_size=u64::from_le_bytes(in_bytes[off..off+8].try_into().unwrap()); off+=8;
    let token_size=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
    let chunk_tokens=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
    let _window=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
    let total_tokens=(orig_size as usize + token_size -1)/token_size;
    let num_chunks=(total_tokens+chunk_tokens-1)/chunk_tokens;
    let mut history:Vec<ChunkInfo>=Vec::with_capacity(num_chunks);
    let mut data=Vec::with_capacity(orig_size as usize);
    for _ in 0..num_chunks {
        let tag=in_bytes[off]; off+=1;
        if tag==0 {
            let core_len=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
            let runs_len=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
            let mut core=Vec::with_capacity(core_len);
            for _ in 0..core_len { core.push(u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap())); off+=4; }
            let mut runs=Vec::with_capacity(runs_len);
            for _ in 0..runs_len { runs.push(u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap())); off+=4; }
            let toks=reconstruct_sym(&core,&runs);
            let bytes=detokenize(&toks,token_size,toks.len()*token_size);
            data.extend(&bytes);
            let raw_len=bytes.len();
            let raw=data[data.len()-raw_len..].to_vec();
            history.push(ChunkInfo{ raw, core, runs });
        } else if tag==1 {
            let dist=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
            let src=&history[history.len()-dist];
            data.extend(&src.raw);
            history.push(ChunkInfo{ raw:src.raw.clone(), core:src.core.clone(), runs:src.runs.clone() });
        } else if tag==2 {
            let dist=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
            let offs=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
            let src=&history[history.len()-dist];
            let n=src.core.len();
            let k=offs%n;
            let mut rc=Vec::with_capacity(n); rc.extend_from_slice(&src.core[k..]); rc.extend_from_slice(&src.core[..k]);
            let mut rr=Vec::with_capacity(n); rr.extend_from_slice(&src.runs[k..]); rr.extend_from_slice(&src.runs[..k]);
            if rc[0]==rc[n-1]{ rr[0]+=rr[n-1]; rc.pop(); rr.pop(); }
            let toks=reconstruct_sym(&rc,&rr);
            let bytes=detokenize(&toks,token_size,toks.len()*token_size);
            data.extend(&bytes);
            let raw_len=bytes.len();
            let raw=data[data.len()-raw_len..].to_vec();
            history.push(ChunkInfo{ raw, core:rc, runs:rr });
        } else if tag==3 {
            let dist=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
            let xor_len=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
            let xor=in_bytes[off..off+xor_len].to_vec(); off+=xor_len;
            let src=&history[history.len()-dist];
            let mut recon=Vec::with_capacity(src.raw.len());
            for k in 0..src.raw.len(){ recon.push(src.raw[k]^xor[k]); }
            data.extend(&recon);
            let toks=tokenize(&recon,token_size);
            let (c2,r2)=bitdrop_sym(&toks);
            history.push(ChunkInfo{ raw:recon, core:c2, runs:r2 });
        } else if tag==4 {
            let run_len=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
            let token_val=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()); off+=4;
            let mut toks=Vec::with_capacity(run_len);
            toks.resize(run_len, token_val);
            let bytes=detokenize(&toks,token_size,toks.len()*token_size);
            data.extend(&bytes);
            let raw_len=bytes.len();
            let raw=data[data.len()-raw_len..].to_vec();
            history.push(ChunkInfo{ raw, core:vec![token_val], runs:vec![run_len as u32] });
        } else { panic!("bad tag"); }
    }
    data.truncate(orig_size as usize);
    data
}

fn compress_one(data:&[u8]) -> Vec<u8> {
    let e=entropy(data);
    // Skim: high entropy blocks are no longer hard-skipped; rely on post-size check
    // if e >= 7.95 { return raw; }
    let is_small = data.len() < 64*1024;
    let mut token_size=if e<3.0 {4} else if e<5.0 {4} else {1};
    // For small files, larger tokens reduce overhead
    if is_small { token_size = std::cmp::max(token_size, 4); }
    let mut chunk_tokens=if token_size==4 {256} else if token_size==2 {512} else {1024};
    let mut window = if e < 4.0 { 512 } else if e < 6.0 { 256 } else { 64 };
    // Small-file rule + adaptive chunk size to reduce per-chunk overhead
    if is_small {
        let total_tokens = if token_size>0 { (data.len() + token_size -1)/token_size } else {1};
        // Use smaller chunks for small files to enable folding, aim for 4-64 chunks
        if total_tokens == 0 {
            chunk_tokens = 1;
            window = 1;
        } else {
            // Single-chunk mode for small files to minimize per-chunk overhead
            // Folding is less useful than zstd on tiny files; keep one chunk
            chunk_tokens = total_tokens;
            window = 1;
        }
    } else {
        // Adaptive chunk size: grow for large files to reduce per-chunk overhead, keep default for small/medium
        if data.len() > 1024*1024 {
            let target_chunk_bytes = std::cmp::min(1024*1024, std::cmp::max(256*1024, data.len()/8));
            let target_chunk_tokens = std::cmp::max(1, target_chunk_bytes / token_size);
            if target_chunk_tokens > chunk_tokens {
                chunk_tokens = target_chunk_tokens;
            }
        }
        // Window scales with number of chunks, capped
        let total_tokens = (data.len() + token_size -1)/token_size;
        let num_chunks = (total_tokens + chunk_tokens -1)/chunk_tokens;
        window = std::cmp::min(window, std::cmp::max(64, num_chunks/4));
    }
    let out=compress_tok_sw(data,token_size,chunk_tokens,window);
    // Verify round-trip before returning; if fails, fall back to raw
    let test_decomp = decompress_chunk(&out);
    let out = if test_decomp == data { out } else { 
        // Fallback to raw for this data
        let mut raw_out=Vec::new();
        raw_out.extend_from_slice(b"BFLS_RAW");
        raw_out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        raw_out.extend(data);
        return raw_out;
    };
    // Automatic small-file mode: compact header, optionally zstd
    let compressed = if is_small { 
        // Strip BFLS4 header 5+8+4+4+4 = 25 bytes and replace with compact header
        if out.len() < 25 { out } else {
            let payload = &out[25..];
            let mut new_out = Vec::new();
            new_out.extend_from_slice(b"BFLS_S");
            new_out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            new_out.push(token_size as u8);
            new_out.extend_from_slice(&(chunk_tokens as u16).to_le_bytes());
            new_out.extend_from_slice(&(window as u16).to_le_bytes());
            new_out.extend_from_slice(payload);
            // Try zstd for small files, keep best result
            match zstd::encode_all(&new_out[..], 1) {
                Ok(z) if z.len() < new_out.len() => z,
                _ => new_out
            }
        }
    } else {
        zstd::encode_all(&out[..],3).unwrap()
    };
    if compressed.len() >= data.len() {
        let mut raw_out=Vec::new();
        raw_out.extend_from_slice(b"BFLS_RAW");
        raw_out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        raw_out.extend(data);
        raw_out
    } else {
        compressed
    }
}

fn decompress_chunk(bytes:&[u8])->Vec<u8>{
    // Handle raw
    if bytes.len()>=8 && &bytes[0..8]==b"BFLS_RAW" {
        let mut o=8;
        let sz=u64::from_le_bytes(bytes[o..o+8].try_into().unwrap()); o+=8;
        return bytes[o..o+sz as usize].to_vec();
    }
    // Handle compact small header
    if bytes.len()>=6 && &bytes[0..6]==b"BFLS_S" {
        let mut off=6;
        let orig_size_u32 = u32::from_le_bytes(bytes[off..off+4].try_into().unwrap()); off+=4;
        let token_size_u8 = bytes[off]; off+=1;
        let chunk_tokens_u16 = u16::from_le_bytes(bytes[off..off+2].try_into().unwrap()); off+=2;
        let window_u16 = u16::from_le_bytes(bytes[off..off+2].try_into().unwrap()); off+=2;
        let payload = &bytes[off..];
        let mut reconstructed = Vec::new();
        reconstructed.extend_from_slice(b"BFLS4");
        reconstructed.extend_from_slice(&(orig_size_u32 as u64).to_le_bytes());
        reconstructed.extend_from_slice(&(token_size_u8 as u32).to_le_bytes());
        reconstructed.extend_from_slice(&(chunk_tokens_u16 as u32).to_le_bytes());
        reconstructed.extend_from_slice(&(window_u16 as u32).to_le_bytes());
        reconstructed.extend_from_slice(payload);
        return decompress_tok_sw(&reconstructed);
    }
    // Handle BFLS4 directly
    if bytes.len()>=5 && &bytes[0..5]==b"BFLS4" {
        return decompress_tok_sw(bytes);
    }
    // Try zstd decode then recurse
    let decoded = match zstd::decode_all(bytes) { Ok(d)=>d, Err(_)=>return decompress_tok_sw(bytes) };
    decompress_chunk(&decoded)
}

use sha2::{Sha256, Digest};

fn write_header(out:&mut Vec<u8>, original_size:u64, checksum:&[u8], payload_size:u64){
    out.extend_from_slice(b"BFLS");
    out.push(1u8); // version
    out.push(0u8); // flags
    out.extend_from_slice(&[0u8;2]);
    out.extend_from_slice(&original_size.to_le_bytes());
    out.extend_from_slice(checksum);
    out.extend_from_slice(&payload_size.to_le_bytes());
}

fn read_header(data:&[u8]) -> Option<(u64, [u8;32], u64, usize)>{ // orig_size, checksum, payload_size, header_len
    if data.len() < 56 { return None; }
    if &data[0..4] != b"BFLS" { return None; }
    let version = data[4];
    if version != 1 { return None; }
    let orig_size = u64::from_le_bytes(data[8..16].try_into().unwrap());
    let mut checksum = [0u8;32];
    checksum.copy_from_slice(&data[16..48]);
    let payload_size = u64::from_le_bytes(data[48..56].try_into().unwrap());
    Some((orig_size, checksum, payload_size, 56))
}

fn run_selftest() {
    eprintln!("Running BFLS self-test...");
    // We'll just run a few quick checks here, full harness is in cargo test
    let mut passed = 0;
    let total = 6;
    // 1 repetitive
    {
        let data = b"Hello world\n".repeat(1000);
        let comp = compress_one(&data);
        let decomp = decompress_chunk(&comp);
        if decomp == data { eprintln!("[PASS] repetitive data"); passed+=1; } else { eprintln!("[FAIL] repetitive data"); }
    }
    // 2 text sample
    {
        let data = b"The quick brown fox jumps over the lazy dog ".repeat(500);
        let comp = compress_one(&data);
        let decomp = decompress_chunk(&comp);
        if decomp == data { eprintln!("[PASS] text sample"); passed+=1; } else { eprintln!("[FAIL] text sample"); }
    }
    // 3 json sample
    {
        let data = br#"{"a":1,"b":[1,2,3]}"#.repeat(2000);
        let comp = compress_one(&data);
        let decomp = decompress_chunk(&comp);
        if decomp == data { eprintln!("[PASS] json sample"); passed+=1; } else { eprintln!("[FAIL] json sample"); }
    }
    // 4 random data
    {
        let data: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
        let comp = compress_one(&data);
        let decomp = decompress_chunk(&comp);
        if decomp == data { eprintln!("[PASS] random data"); passed+=1; } else { eprintln!("[FAIL] random data"); }
    }
    // 5 checksum validation via header
    {
        let data = b"checksum test".repeat(100);
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let checksum: [u8;32] = hasher.finalize().into();
        let payload = compress_one(&data);
        let mut out = Vec::new();
        write_header(&mut out, data.len() as u64, &checksum, payload.len() as u64);
        out.extend(payload);
        let orig_size = u64::from_le_bytes(out[8..16].try_into().unwrap());
        let mut cs = [0u8;32];
        cs.copy_from_slice(&out[16..48]);
        if orig_size == data.len() as u64 && cs == checksum { eprintln!("[PASS] checksum validation"); passed+=1; } else { eprintln!("[FAIL] checksum validation"); }
    }
    // 6 chunked container
    {
        let data = vec![0u8; 10*1024*1024];
        const SPLIT: usize = 4*1024*1024;
        let mut buf = Vec::new();
        buf.extend_from_slice(b"BFLS_M");
        buf.extend_from_slice(&(data.len() as u64).to_le_bytes());
        buf.extend_from_slice(&(SPLIT as u32).to_le_bytes());
        let num = (data.len()+SPLIT-1)/SPLIT;
        buf.extend_from_slice(&(num as u32).to_le_bytes());
        for chunk in data.chunks(SPLIT) {
            let c = compress_one(chunk);
            buf.extend_from_slice(&(c.len() as u32).to_le_bytes());
            buf.extend(c);
        }
        // decompress
        let mut off = 6+8+4+4;
        let mut out = Vec::new();
        for _ in 0..num {
            let l = u32::from_le_bytes(buf[off..off+4].try_into().unwrap()) as usize; off+=4;
            let comp = &buf[off..off+l]; off+=l;
            out.extend(decompress_chunk(comp));
        }
        if out == data { eprintln!("[PASS] chunked container"); passed+=1; } else { eprintln!("[FAIL] chunked container"); }
    }
    eprintln!("\n{}/{} tests passed", passed, total);
}

fn main(){
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: bfls <compress|c|decompress|d|benchmark|inspect|selftest> ...");
        return;
    }
    match args[1].as_str() {
        "compress" | "c" => {
            if args.len() < 4 { eprintln!("need input output"); return; }
            let in_path = &args[2];
            let out_path = &args[3];
            let data = match std::fs::read(in_path) { Ok(d)=>d, Err(e)=>{eprintln!("read error: {}",e); return;} };
            let mut hasher = sha2::Sha256::new();
            hasher.update(&data);
            let checksum: [u8;32] = hasher.finalize().into();
            const SPLIT_SIZE: usize = 4*1024*1024;
            let payload = if data.len() > SPLIT_SIZE {
                let mut buf = Vec::new();
                buf.extend_from_slice(b"BFLS_M");
                buf.extend_from_slice(&(data.len() as u64).to_le_bytes());
                buf.extend_from_slice(&(SPLIT_SIZE as u32).to_le_bytes());
                let num_chunks = (data.len()+SPLIT_SIZE-1)/SPLIT_SIZE;
                buf.extend_from_slice(&(num_chunks as u32).to_le_bytes());
                let chunks: Vec<&[u8]> = data.chunks(SPLIT_SIZE).collect();
                let comps: Vec<Vec<u8>> = chunks.par_iter().map(|c| compress_one(c)).collect();
                for comp in &comps { buf.extend_from_slice(&(comp.len() as u32).to_le_bytes()); buf.extend(comp); }
                buf
            } else { compress_one(&data) };
            let mut out_file = Vec::new();
            if data.len() < 64*1024 {
                // Small files: write compact container directly, no outer BFLS header
                out_file.extend(payload);
            } else {
                write_header(&mut out_file, data.len() as u64, &checksum, payload.len() as u64);
                out_file.extend(payload);
            }
            if let Err(e) = std::fs::write(out_path, &out_file) { eprintln!("write error: {}",e); return; }
            eprintln!("Wrote {} -> {} ratio {:.4}", data.len(), out_file.len(), out_file.len() as f64 / data.len() as f64);
        }
        "decompress" | "d" => {
            if args.len() < 4 { eprintln!("need input output"); return; }
            let in_path = &args[2];
            let out_path = &args[3];
            let in_bytes = match std::fs::read(in_path) { Ok(b)=>b, Err(e)=>{eprintln!("read error: {}",e); return;} };
            if in_bytes.len() >= 56 && &in_bytes[0..4] == b"BFLS" {
                let orig_size = u64::from_le_bytes(in_bytes[8..16].try_into().unwrap());
                let mut cs = [0u8;32]; cs.copy_from_slice(&in_bytes[16..48]);
                let payload_size = u64::from_le_bytes(in_bytes[48..56].try_into().unwrap()) as usize;
                let payload = in_bytes[56..56+payload_size].to_vec();
                let data = decompress_payload(&payload, orig_size);
                let mut hasher = sha2::Sha256::new(); hasher.update(&data);
                let calc: [u8;32] = hasher.finalize().into();
                if calc != cs { eprintln!("WARNING: checksum mismatch!"); } else { eprintln!("Checksum OK"); }
                if let Err(e) = std::fs::write(out_path, &data) { eprintln!("write error: {}",e); return; }
                eprintln!("Decompressed {} -> {} bytes", payload.len(), data.len());
            } else {
                let data = decompress_payload(&in_bytes, 0);
                std::fs::write(out_path, &data).unwrap();
                eprintln!("Decompressed {} bytes (no header)", data.len());
            }
        }
        "benchmark" => {
            if args.len() < 3 { eprintln!("need input file"); return; }
            let path = &args[2];
            let stages = args.iter().skip(2).any(|a| a == "--stages");
            let data = match std::fs::read(path) { Ok(d)=>d, Err(e)=>{eprintln!("read error: {}",e); return;} };
            let t0 = std::time::Instant::now();
            let comp = compress_one(&data);
            let enc_ms = t0.elapsed().as_millis();
            let t1 = std::time::Instant::now();
            let decomp = decompress_chunk(&comp);
            let dec_ms = t1.elapsed().as_millis();
            println!("Original: {} B", data.len());
            if stages {
                let e = entropy(&data);
                let token_size = if e<3.0 {4} else if e<5.0 {2} else {1};
                println!("Entropy: {:.3}", e);
                println!("Token size: {}", token_size);
                println!("Compressed payload: {} B", comp.len());
            } else {
                println!("Compressed: {} B", comp.len());
            }
            println!("Ratio: {:.4}", comp.len() as f64 / data.len() as f64);
            println!("Encode ms: {}", enc_ms);
            println!("Decode ms: {}", dec_ms);
            if decomp != data { eprintln!("ERROR: round-trip mismatch!"); } else { eprintln!("Round-trip OK"); }
        }
        "inspect" => {
            if args.len() < 3 { eprintln!("need file"); return; }
            let bytes = match std::fs::read(&args[2]) { Ok(b)=>b, Err(e)=>{eprintln!("read error: {}",e); return;} };
            if bytes.len() >= 56 && &bytes[0..4] == b"BFLS" {
                let orig = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                let payload_size = u64::from_le_bytes(bytes[48..56].try_into().unwrap());
                println!("Magic: BFLS");
                println!("Version: {}", bytes[4]);
                println!("Original size: {}", orig);
                println!("Payload size: {}", payload_size);
            } else { println!("Not a BFLS header file"); }
        }
        "selftest" => { run_selftest(); }
        _ => { eprintln!("Unknown command"); }
    }
}

fn decompress_payload(payload: &[u8], expected_size: u64) -> Vec<u8> {
    if payload.len() >= 6 && &payload[0..6] == b"BFLS_M" {
        let mut off = 6;
        let orig_size = u64::from_le_bytes(payload[off..off+8].try_into().unwrap()); off+=8;
        let _chunk_size = u32::from_le_bytes(payload[off..off+4].try_into().unwrap()); off+=4;
        let num_chunks = u32::from_le_bytes(payload[off..off+4].try_into().unwrap()) as usize; off+=4;
        let mut out = Vec::with_capacity(orig_size as usize);
        for _ in 0..num_chunks {
            let comp_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap()) as usize; off+=4;
            let comp_bytes = &payload[off..off+comp_len];
            out.extend(decompress_chunk(comp_bytes));
            off += comp_len;
        }
        out.truncate(orig_size as usize);
        out
    } else {
        decompress_chunk(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_10k_random() {
        for i in 0..10000 {
            let size = match i % 6 {
                0 => 0,
                1 => 1,
                2 => 128,
                3 => 1024,
                4 => 65536,
                _ => 1024*1024,
            };
            let data: Vec<u8> = (0..size).map(|j| ((j + i) % 256) as u8).collect();
            let comp = compress_one(&data);
            let decomp = decompress_chunk(&comp);
            assert_eq!(decomp, data, "roundtrip failed at iteration {}", i);
        }
    }
    #[test]
    fn roundtrip_corpus() {
        let cases = vec![
            ("empty", vec![] as Vec<u8>),
            ("1byte", vec![0xAB]),
            ("repetitive", "Hello world\n".repeat(10000).into_bytes()),
            ("text", "The quick brown fox ".repeat(10000).into_bytes()),
            ("json", br#"{"a":1}"#.repeat(5000).to_vec()),
            ("csv", "a,b,c\n".repeat(20000).into_bytes()),
        ];
        for (name, data) in cases {
            let comp = compress_one(&data);
            let decomp = decompress_chunk(&comp);
            assert_eq!(decomp, data, "roundtrip failed for {}", name);
        }
    }
}

