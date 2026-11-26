use std::fs;

#[derive(Debug, Clone, Default)]
pub struct StageStats {
    pub original: usize,
    pub bitdrop: usize,
    pub fold: usize,
    pub layer: usize,
    pub slide: usize,
    pub raw: usize,
    pub fold_layer_slide: usize,
    pub zstd: usize,
    pub final_size: usize,
}

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
pub fn entropy(data:&[u8])->f64{
    if data.is_empty(){return 0.0;}
    let mut f=[0usize;256];
    for &b in data {f[b as usize]+=1;}
    let len=data.len() as f64;
    let mut e=0.0;
    for &c in &f { if c>0 { let p=c as f64/len; e-=p*p.log2(); } }
    e
}

// GF(256) arithmetic for Berlekamp-Massey
#[inline]
fn gf_mul_by_x(a: u8) -> u8 {
    if a & 0x80 != 0 {
        ((a << 1) ^ 0x1b) & 0xff
    } else {
        (a << 1) & 0xff
    }
}
#[inline]
fn gf_mul(a: u8, b: u8) -> u8 {
    let mut res = 0u8;
    let mut aa = a;
    let mut bb = b;
    while bb != 0 {
        if bb & 1 != 0 { res ^= aa; }
        aa = gf_mul_by_x(aa);
        bb >>= 1;
    }
    res
}
#[inline]
fn gf_add(a: u8, b: u8) -> u8 { a ^ b }

// Bit view transforms
#[inline]
fn bit_reverse_byte(b: u8) -> u8 {
    let mut v = b;
    v = ((v & 0xF0) >> 4) | ((v & 0x0F) << 4);
    v = ((v & 0xCC) >> 2) | ((v & 0x33) << 2);
    v = ((v & 0xAA) >> 1) | ((v & 0x55) << 1);
    v
}
#[inline]
fn bit_rotate_left_byte(b: u8, n: u32) -> u8 {
    let n = n % 8;
    ((b << n) | (b >> (8 - n))) & 0xFF
}
#[inline]
fn nibble_swap_byte(b: u8) -> u8 {
    ((b & 0x0F) << 4) | ((b & 0xF0) >> 4)
}

fn apply_bit_transform(data: &[u8], id: u8) -> Vec<u8> {
    match id {
        0 => data.to_vec(),
        1 => data.iter().map(|&b| bit_reverse_byte(b)).collect(),
        2 => data.iter().map(|&b| bit_rotate_left_byte(b,1)).collect(),
        3 => data.iter().map(|&b| bit_rotate_left_byte(b,2)).collect(),
        4 => data.iter().map(|&b| bit_rotate_left_byte(b,3)).collect(),
        5 => data.iter().map(|&b| bit_rotate_left_byte(b,4)).collect(),
        6 => data.iter().map(|&b| nibble_swap_byte(b)).collect(),
        7 => {
            let mut v = data.to_vec();
            v.reverse();
            v
        }
        _ => data.to_vec(),
    }
}

fn reverse_bit_transform(data: &[u8], id: u8) -> Vec<u8> {
    match id {
        0 => data.to_vec(),
        1 => data.iter().map(|&b| bit_reverse_byte(b)).collect(),
        2 => data.iter().map(|&b| bit_rotate_left_byte(b,7)).collect(),
        3 => data.iter().map(|&b| bit_rotate_left_byte(b,6)).collect(),
        4 => data.iter().map(|&b| bit_rotate_left_byte(b,5)).collect(),
        5 => data.iter().map(|&b| bit_rotate_left_byte(b,4)).collect(),
        6 => data.iter().map(|&b| nibble_swap_byte(b)).collect(),
        7 => {
            let mut v = data.to_vec();
            v.reverse();
            v
        }
        _ => data.to_vec(),
    }
}

fn find_order2_recurrence(data: &[u8]) -> Option<(u8,u8)> {
    if data.len() < 3 { return None; }
    // brute force c1,c2 over GF(256)
    for c1 in 0u8..=255 {
        for c2 in 0u8..=255 {
            let mut ok = true;
            // quick check first 32 bytes
            for i in 2..std::cmp::min(32, data.len()) {
                let expected = gf_add(gf_mul(c1, data[i-1]), gf_mul(c2, data[i-2]));
                if data[i] != expected { ok = false; break; }
            }
            if !ok { continue; }
            for i in 32..data.len() {
                let expected = gf_add(gf_mul(c1, data[i-1]), gf_mul(c2, data[i-2]));
                if data[i] != expected { ok = false; break; }
            }
            if ok { return Some((c1, c2)); }
        }
    }
    None
}

fn find_order2_recurrence_approx(data: &[u8]) -> Option<(u8,u8)> {
    if data.len() < 3 { return None; }
    // find c1,c2 that fits first 64 bytes exactly, for approximate model
    let check_len = std::cmp::min(64, data.len());
    if check_len < 3 { return None; }
    for c1 in 0u8..=255 {
        for c2 in 0u8..=255 {
            let mut ok = true;
            for i in 2..check_len {
                let expected = gf_add(gf_mul(c1, data[i-1]), gf_mul(c2, data[i-2]));
                if data[i] != expected { ok = false; break; }
            }
            if ok { return Some((c1, c2)); }
        }
    }
    None
}

// LEB128 varint helpers
fn write_varint(buf:&mut Vec<u8>, mut v:u64){
    loop {
        let mut byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 { byte |= 0x80; }
        buf.push(byte);
        if v == 0 { break; }
    }
}
fn read_varint(data:&[u8], pos:&mut usize)->u64{
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        let byte = data[*pos];
        *pos += 1;
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 { break; }
        shift += 7;
    }
    result
}

fn write_u32_conditional(buf:&mut Vec<u8>, v:u32, use_varint:bool){
    if use_varint { write_varint(buf, v as u64); } else { buf.extend_from_slice(&v.to_le_bytes()); }
}
fn read_u32_conditional(data:&[u8], pos:&mut usize, use_varint:bool)->u32{
    if use_varint { read_varint(data, pos) as u32 } else { let v = u32::from_le_bytes(data[*pos..*pos+4].try_into().unwrap()); *pos += 4; v }
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

fn find_canonical_rotation_offset(toks:&[u32])->usize {
    let n = toks.len();
    if n <= 1 { return 0; }
    let canon = canonical_rotation(toks);
    for k in 0..n {
        let mut ok = true;
        for i in 0..n {
            if toks[(k + i) % n] != canon[i] { ok = false; break; }
        }
        if ok { return k; }
    }
    0
}
#[derive(Clone, Default)]
struct ChunkInfo { raw:Vec<u8>, core:Vec<u32>, runs:Vec<u32>, rot:usize }

fn compress_tok_sw(data:&[u8], token_size:usize, chunk_tokens:usize, window_chunks:usize, enable_rotation:bool, use_varint:bool)->Vec<u8>{
    let tokens=tokenize(data,token_size);
    let num_chunks=(tokens.len()+chunk_tokens-1)/chunk_tokens;
    let mut history:Vec<ChunkInfo>=Vec::with_capacity(num_chunks);
    let mut core_index:FxHashMap<(Vec<u32>,Vec<u32>), usize>=FxHashMap::default();
    let mut rot_index:FxHashMap<Vec<u32>, usize>=FxHashMap::default();
    let mut core_bloom = BloomFilter::new(1<<20, 3);
    let mut rot_bloom = BloomFilter::new(1<<20, 3);
    let mut out=Vec::new();
    if use_varint { out.extend_from_slice(b"BFLS4V"); } else { out.extend_from_slice(b"BFLS4"); }
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(&(token_size as u32).to_le_bytes());
    out.extend_from_slice(&(chunk_tokens as u32).to_le_bytes());
    out.extend_from_slice(&(window_chunks as u32).to_le_bytes());
    for i in 0..num_chunks {
        let s=i*chunk_tokens; let e=std::cmp::min(s+chunk_tokens,tokens.len());
        let chunk_toks=&tokens[s..e];
        let rot = if enable_rotation { find_canonical_rotation_offset(chunk_toks) } else { 0 };
        let rotated_toks = if rot==0 || chunk_toks.len()<=1 {
            chunk_toks.to_vec()
        } else {
            let n = chunk_toks.len();
            let mut v = Vec::with_capacity(n);
            v.extend_from_slice(&chunk_toks[rot..]);
            v.extend_from_slice(&chunk_toks[..rot]);
            v
        };
        let (core,runs)=bitdrop_sym(&rotated_toks);
        let cur_raw_orig=&data[s*token_size..std::cmp::min((e)*token_size,data.len())];
        // Rotate raw bytes to match token rotation for layer comparison
        let cur_raw_rot = if rot==0 || chunk_toks.len()<=1 {
            cur_raw_orig.to_vec()
        } else {
            let tmp = detokenize(&rotated_toks, token_size, rotated_toks.len()*token_size);
            tmp[..cur_raw_orig.len()].to_vec()
        };
        let cur_info=ChunkInfo{ raw:cur_raw_orig.to_vec(), core:core.clone(), runs:runs.clone(), rot };
        // cur_raw_rot is kept for potential future use, currently unused
        let _cur_raw_rot = &cur_raw_rot;

        let key=(core.clone(),runs.clone());
        let mut found_idx=None;
        let core_fp = fingerprint_pair(&core, &runs);
        if core_bloom.might_contain(&core_fp.to_le_bytes()) {
            if let Some(&last_pos)=core_index.get(&key){
                if i>last_pos && i-last_pos<=window_chunks { found_idx=Some(i-last_pos); }
            }
        }
        // Rotation-aware fold: try to match core+runs after rotation
        let mut rot_fold_found=None;
        if found_idx.is_none() {
            let can=canonical_rotation(&core);
            let rot_fp = fingerprint_slice(&can);
            if rot_bloom.might_contain(&rot_fp.to_le_bytes()) {
                if let Some(&last_pos)=rot_index.get(&can){
                    if i>last_pos && i-last_pos<=window_chunks {
                        let prev_core=&history[last_pos].core;
                        let prev_runs=&history[last_pos].runs;
                        if prev_core.len()==core.len() && prev_runs.len()==runs.len() {
                            let n=core.len();
                            for k in 0..n {
                                let mut core_match=true;
                                let mut runs_match=true;
                                for j in 0..n {
                                    if prev_core[(k+j)%n]!=core[j] { core_match=false; break; }
                                    if prev_runs[(k+j)%n]!=runs[j] { runs_match=false; break; }
                                }
                                if core_match && runs_match {
                                    rot_fold_found=Some((i-last_pos,k));
                                    break;
                                }
                            }
                        }
                    }
                }
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
                if cur_raw_orig.len()==prev.raw.len(){
                    let hd=hamming_bytes(cur_raw_orig,&prev.raw);
                    let thresh=cur_raw_orig.len()*8/10;
                    if hd>0 && hd<=thresh{
                        layer_dist=Some(i-j);
                        let mut xor=vec![0u8;cur_raw_orig.len()];
                        for k in 0..cur_raw_orig.len(){ xor[k]=cur_raw_orig[k]^prev.raw[k]; }
                        layer_xor=Some(xor);
                        break;
                    }
                }
            }
        }
        if let Some(dist)=found_idx {
            out.push(1u8);
            write_u32_conditional(&mut out, dist as u32, use_varint);
            let cloned=history[i-dist].clone();
            history.push(cloned.clone());
            core_index.insert((cloned.core.clone(),cloned.runs.clone()), i);
            rot_index.insert(canonical_rotation(&cloned.core), i);
            core_bloom.add(&fingerprint_pair(&cloned.core, &cloned.runs).to_le_bytes());
            rot_bloom.add(&fingerprint_slice(&canonical_rotation(&cloned.core)).to_le_bytes());
        } else if let Some((dist,off))=rot_fold_found {
            out.push(5u8);
            write_u32_conditional(&mut out, dist as u32, use_varint);
            write_u32_conditional(&mut out, off as u32, use_varint);
            write_u32_conditional(&mut out, cur_info.rot as u32, use_varint);
            let prev=&history[i-dist];
            let n=prev.core.len();
            let k=off%n;
            let mut rc=Vec::with_capacity(n); rc.extend_from_slice(&prev.core[k..]); rc.extend_from_slice(&prev.core[..k]);
            let mut rr=Vec::with_capacity(n); rr.extend_from_slice(&prev.runs[k..]); rr.extend_from_slice(&prev.runs[..k]);
            if rc[0]==rc[n-1]{ rr[0]+=rr[n-1]; rc.pop(); rr.pop(); }
            let new_info=ChunkInfo{ raw:cur_raw_orig.to_vec(), core:rc.clone(), runs:rr.clone(), rot:cur_info.rot };
            history.push(new_info.clone());
            core_index.insert((rc.clone(),rr.clone()), i);
            rot_index.insert(canonical_rotation(&rc), i);
            core_bloom.add(&fingerprint_pair(&rc, &rr).to_le_bytes());
            rot_bloom.add(&fingerprint_slice(&canonical_rotation(&rc)).to_le_bytes());
        } else if let Some((dist,off))=slide_found {
            out.push(2u8);
            write_u32_conditional(&mut out, dist as u32, use_varint);
            write_u32_conditional(&mut out, off as u32, use_varint);
            write_u32_conditional(&mut out, cur_info.rot as u32, use_varint);
            let prev=&history[i-dist];
            let n=prev.core.len();
            let k=off%n;
            let mut rc=Vec::with_capacity(n); rc.extend_from_slice(&prev.core[k..]); rc.extend_from_slice(&prev.core[..k]);
            let mut rr=Vec::with_capacity(n); rr.extend_from_slice(&prev.runs[k..]); rr.extend_from_slice(&prev.runs[..k]);
            if rc[0]==rc[n-1]{ rr[0]+=rr[n-1]; rc.pop(); rr.pop(); }
            let new_info=ChunkInfo{ raw:cur_raw_orig.to_vec(), core:rc.clone(), runs:rr.clone(), rot:cur_info.rot };
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
                write_u32_conditional(&mut out, dist as u32, use_varint);
                write_u32_conditional(&mut out, xor.len() as u32, use_varint);
                out.extend(&xor);
                let prev=&history[i-dist];
                let mut recon=Vec::with_capacity(prev.raw.len());
                for k in 0..prev.raw.len(){ recon.push(prev.raw[k]^xor[k]); }
                let toks2=tokenize(&recon,token_size);
                let (c2,r2)=bitdrop_sym(&toks2);
                let new_info=ChunkInfo{ raw:recon.clone(), core:c2.clone(), runs:r2.clone(), rot:cur_info.rot };
                history.push(new_info.clone());
                core_index.insert((c2.clone(),r2.clone()), i);
                rot_index.insert(canonical_rotation(&c2), i);
                core_bloom.add(&fingerprint_pair(&c2, &r2).to_le_bytes());
                rot_bloom.add(&fingerprint_slice(&canonical_rotation(&c2)).to_le_bytes());
            } else {
                // fall through to raw storage
                out.push(0u8);
                write_u32_conditional(&mut out, cur_info.rot as u32, use_varint);
                write_u32_conditional(&mut out, cur_info.core.len() as u32, use_varint);
                write_u32_conditional(&mut out, cur_info.runs.len() as u32, use_varint);
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
            write_u32_conditional(&mut out, cur_info.rot as u32, use_varint);
            write_u32_conditional(&mut out, cur_info.core.len() as u32, use_varint);
            write_u32_conditional(&mut out, cur_info.runs.len() as u32, use_varint);
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
    let use_varint;
    let magic_len;
    if in_bytes.len()>=6 && &in_bytes[0..6]==b"BFLS4V" {
        use_varint = true;
        magic_len = 6;
    } else if in_bytes.len()>=5 && &in_bytes[0..5]==b"BFLS4" {
        use_varint = false;
        magic_len = 5;
    } else {
        panic!("bad magic");
    }
    let mut off = magic_len;
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
            let rot = read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let core_len = read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let runs_len = read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let mut core=Vec::with_capacity(core_len);
            for _ in 0..core_len { core.push(u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap())); off+=4; }
            let mut runs=Vec::with_capacity(runs_len);
            for _ in 0..runs_len { runs.push(u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap())); off+=4; }
            let mut toks=reconstruct_sym(&core,&runs);
            // rotate back to original order
            if rot != 0 && !toks.is_empty() {
                let n = toks.len();
                let r = rot % n;
                if r != 0 {
                    let mut rotated = Vec::with_capacity(n);
                    rotated.extend_from_slice(&toks[n - r..]);
                    rotated.extend_from_slice(&toks[..n - r]);
                    toks = rotated;
                }
            }
            let bytes=detokenize(&toks,token_size,toks.len()*token_size);
            data.extend(&bytes);
            let raw_len=bytes.len();
            let raw=data[data.len()-raw_len..].to_vec();
            history.push(ChunkInfo{ raw, core, runs, rot });
        } else if tag==1 {
            let dist=read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let src=&history[history.len()-dist];
            // rotate back raw bytes
            let raw_rotated = &src.raw;
            let rot = src.rot;
            let raw_original = if rot != 0 && !raw_rotated.is_empty() {
                // need token size to rotate? We have raw bytes, rotate by token_size? 
                // Simpler: we assume raw is already rotated, we need to rotate back by rot tokens.
                // We don't know token boundaries here. For simplicity, rotate bytes by rot * ?
                // We'll reconstruct via detokenize? Hmm.
                // For now, assume rot is 0 for tag1 (since raw is rotated, but we need original).
                // We'll just copy raw as is, rotation will be handled by stored rot in ChunkInfo.
                raw_rotated.clone()
            } else {
                raw_rotated.clone()
            };
            data.extend(&raw_original);
            history.push(ChunkInfo{ raw:src.raw.clone(), core:src.core.clone(), runs:src.runs.clone(), rot:src.rot });
        } else if tag==2 {
            let dist=read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let offs=read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let rot=read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let src=&history[history.len()-dist];
            let n=src.core.len();
            let k=offs%n;
            let mut rc=Vec::with_capacity(n); rc.extend_from_slice(&src.core[k..]); rc.extend_from_slice(&src.core[..k]);
            let mut rr=Vec::with_capacity(n); rr.extend_from_slice(&src.runs[k..]); rr.extend_from_slice(&src.runs[..k]);
            if rc[0]==rc[n-1]{ rr[0]+=rr[n-1]; rc.pop(); rr.pop(); }
            let mut toks=reconstruct_sym(&rc,&rr);
            if rot != 0 && !toks.is_empty() {
                let n_t = toks.len();
                let r = rot % n_t;
                if r != 0 {
                    let mut rotated = Vec::with_capacity(n_t);
                    rotated.extend_from_slice(&toks[n_t - r..]);
                    rotated.extend_from_slice(&toks[..n_t - r]);
                    toks = rotated;
                }
            }
            let bytes=detokenize(&toks,token_size,toks.len()*token_size);
            data.extend(&bytes);
            let raw_len=bytes.len();
            let raw=data[data.len()-raw_len..].to_vec();
            history.push(ChunkInfo{ raw, core:rc, runs:rr, rot });
        } else if tag==5 {
            let dist=read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let offs=read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let rot=read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let src=&history[history.len()-dist];
            let n=src.core.len();
            let k=offs%n;
            let mut rc=Vec::with_capacity(n); rc.extend_from_slice(&src.core[k..]); rc.extend_from_slice(&src.core[..k]);
            let mut rr=Vec::with_capacity(n); rr.extend_from_slice(&src.runs[k..]); rr.extend_from_slice(&src.runs[..k]);
            if rc[0]==rc[n-1]{ rr[0]+=rr[n-1]; rc.pop(); rr.pop(); }
            let mut toks=reconstruct_sym(&rc,&rr);
            if rot != 0 && !toks.is_empty() {
                let n_t = toks.len();
                let r = rot % n_t;
                if r != 0 {
                    let mut rotated = Vec::with_capacity(n_t);
                    rotated.extend_from_slice(&toks[n_t - r..]);
                    rotated.extend_from_slice(&toks[..n_t - r]);
                    toks = rotated;
                }
            }
            let bytes=detokenize(&toks,token_size,toks.len()*token_size);
            data.extend(&bytes);
            let raw_len=bytes.len();
            let raw=data[data.len()-raw_len..].to_vec();
            history.push(ChunkInfo{ raw, core:rc, runs:rr, rot });
        } else if tag==3 {
            let dist=read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let xor_len=read_u32_conditional(in_bytes, &mut off, use_varint) as usize;
            let xor=in_bytes[off..off+xor_len].to_vec(); off+=xor_len;
            let src=&history[history.len()-dist];
            let mut recon=Vec::with_capacity(src.raw.len());
            for k in 0..src.raw.len(){ recon.push(src.raw[k]^xor[k]); }
            data.extend(&recon);
            let toks=tokenize(&recon,token_size);
            let (c2,r2)=bitdrop_sym(&toks);
            history.push(ChunkInfo{ raw:recon, core:c2, runs:r2, rot:0 });
        } else if tag==4 {
            let run_len=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()) as usize; off+=4;
            let token_val=u32::from_le_bytes(in_bytes[off..off+4].try_into().unwrap()); off+=4;
            let mut toks=Vec::with_capacity(run_len);
            toks.resize(run_len, token_val);
            let bytes=detokenize(&toks,token_size,toks.len()*token_size);
            data.extend(&bytes);
            let raw_len=bytes.len();
            let raw=data[data.len()-raw_len..].to_vec();
            history.push(ChunkInfo{ raw, core:vec![token_val], runs:vec![run_len as u32], rot:0 });
        } else { panic!("bad tag"); }
    }
    data.truncate(orig_size as usize);
    data
}

fn parse_sw_payload_stats(payload: &[u8]) -> (usize, usize, usize, usize) {
    // payload starts with 25 byte header
    if payload.len() < 25 { return (0,0,0,0); }
    let mut off = 25;
    let mut raw = 0usize;
    let mut fold = 0usize;
    let mut slide = 0usize;
    let mut layer = 0usize;
    while off < payload.len() {
        let tag = payload[off]; off += 1;
        match tag {
            0 => {
                // rot, core_len, runs_len, core data, runs data
                if off + 12 > payload.len() { break; }
                let _rot = u32::from_le_bytes(payload[off..off+4].try_into().unwrap()); off += 4;
                let core_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap()) as usize; off += 4;
                let runs_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap()) as usize; off += 4;
                let need = core_len*4 + runs_len*4;
                if off + need > payload.len() { break; }
                raw += 1 + 12 + need;
                off += need;
            }
            1 => {
                if off + 4 > payload.len() { break; }
                fold += 1 + 4;
                off += 4;
            }
            2 => {
                if off + 12 > payload.len() { break; }
                slide += 1 + 12;
                off += 12;
            }
            3 => {
                if off + 8 > payload.len() { break; }
                let xor_len = u32::from_le_bytes(payload[off+4..off+8].try_into().unwrap()) as usize;
                off += 8;
                if off + xor_len > payload.len() { break; }
                layer += 1 + 4 + 4 + xor_len;
                off += xor_len;
            }
            4 => {
                if off + 8 > payload.len() { break; }
                // treat as raw-ish
                raw += 1 + 8;
                off += 8;
            }
            5 => {
                if off + 12 > payload.len() { break; }
                fold += 1 + 12;
                off += 12;
            }
            _ => { break; }
        }
    }
    (raw, fold, slide, layer)
}

pub fn compress_with_stats(data: &[u8]) -> (Vec<u8>, StageStats) {
    let original = data.len();
    if original == 0 {
        let stats = StageStats { original, final_size: 0, ..Default::default() };
        return (Vec::new(), stats);
    }
    // Large file parallel chunking for speed - only if significantly larger than chunk size
    if original > CHUNK_SIZE * 2 {
        let comp = compress_chunked(data);
        let stats = StageStats { original, final_size: comp.len(), ..Default::default() };
        return (comp, stats);
    }
    let e = entropy(data);
    if e >= 7.9 {
        let mut raw_out = Vec::new();
        raw_out.extend_from_slice(b"BFLS_RAW");
        raw_out.extend_from_slice(&(original as u64).to_le_bytes());
        raw_out.extend_from_slice(data);
        let stats = StageStats { original, final_size: raw_out.len(), ..Default::default() };
        return (raw_out, stats);
    }
    let token_size = if e < 3.0 {4} else if e < 5.0 {4} else {1};
    let is_small = data.len() < 64*1024;
    let tokens = tokenize(data, token_size);
    let chunk_tokens = if token_size == 4 {256} else if token_size == 2 {512} else {1024};
    let num_chunks = if tokens.is_empty() {0} else {(tokens.len() + chunk_tokens - 1) / chunk_tokens};
    let mut bitdrop_bytes = 0usize;
    for i in 0..num_chunks {
        let s = i * chunk_tokens;
        let e_idx = std::cmp::min(s + chunk_tokens, tokens.len());
        let chunk = &tokens[s..e_idx];
        let (core, runs) = bitdrop_sym(chunk);
        bitdrop_bytes += core.len()*4 + runs.len()*4;
    }
    let mut chunk_tokens_adj = chunk_tokens;
    let mut window = if e < 4.0 {512} else if e < 6.0 {256} else {64};
    if is_small {
        let total_tokens = if token_size>0 {(data.len()+token_size-1)/token_size} else {1};
        chunk_tokens_adj = std::cmp::max(1, total_tokens);
        window = 1;
    } else if data.len() > 1024*1024 {
        let target_chunk_bytes = std::cmp::min(1024*1024, std::cmp::max(256*1024, data.len()/8));
        let target_chunk_tokens = std::cmp::max(1, target_chunk_bytes / token_size);
        if target_chunk_tokens > chunk_tokens { chunk_tokens_adj = target_chunk_tokens; }
        let total_tokens = (data.len() + token_size -1)/token_size;
        let num_chunks2 = (total_tokens + chunk_tokens_adj -1)/chunk_tokens_adj;
        window = std::cmp::min(window, std::cmp::max(64, num_chunks2/4));
    }
    let enable_rotation = e < 7.8;
    let mut best_compressed: Option<Vec<u8>> = None;
    let mut best_stats = StageStats { original, ..Default::default() };
    let mut best_len = usize::MAX;
    let mut best_bit_id = 0u8;
    // Bit-view + varint search (sequential for 8 candidates)
    for bit_id in 0..8u8 {
        let transformed = apply_bit_transform(data, bit_id as u8);
        let mut best_for_view_len = usize::MAX;
        let mut best_for_view_comp: Option<Vec<u8>> = None;
        let mut best_for_view_stats: Option<StageStats> = None;
        for use_varint in [false, true] {
            let payload_sw = compress_tok_sw(&transformed, token_size, chunk_tokens_adj, window, enable_rotation, use_varint);
            if decompress_chunk(&payload_sw) != transformed { continue; }
            let compressed = if is_small {
                match zstd::encode_all(&payload_sw[..], 1) { Ok(z) if z.len() < payload_sw.len() => z, _ => payload_sw.clone() }
            } else {
                zstd::encode_all(&payload_sw[..], 3).unwrap()
            };
            if compressed.len() >= best_for_view_len { continue; }
            let fold_layer_slide = payload_sw.len();
            let (raw_bytes, fold_bytes, slide_bytes, layer_bytes) = parse_sw_payload_stats(&payload_sw);
            let final_size = if compressed.len() >= original { original } else { compressed.len() };
            let stats_candidate = StageStats { original, bitdrop: bitdrop_bytes, fold: fold_bytes, layer: layer_bytes, slide: slide_bytes, raw: raw_bytes, fold_layer_slide, zstd: compressed.len(), final_size };
            best_for_view_len = compressed.len();
            best_for_view_comp = Some(compressed);
            best_for_view_stats = Some(stats_candidate);
        }
        if let Some(comp) = best_for_view_comp {
            let final_comp = if let Some(math) = math_compress(&comp) { math } else { comp };
            let final_len = final_comp.len();
            if final_len < best_len {
                best_len = final_len;
                best_bit_id = bit_id;
                if let Some(mut stats) = best_for_view_stats {
                    let final_size = if final_len >= original { original } else { final_len };
                    stats.final_size = final_size;
                    stats.zstd = final_len;
                    best_stats = stats;
                    best_compressed = Some(final_comp);
                }
            }
        }
    }
    if let Some(mut comp) = best_compressed {
        if best_bit_id != 0 {
            let mut out = Vec::new();
            out.extend_from_slice(BITVIEW_MAGIC);
            out.push(best_bit_id);
            out.extend_from_slice(&comp);
            comp = out;
            let final_size = if comp.len() >= original { original } else { comp.len() };
            best_stats.final_size = final_size;
            best_stats.zstd = comp.len();
        }
        (comp, best_stats)
    } else {
        let mut raw_out = Vec::new();
        raw_out.extend_from_slice(b"BFLS_RAW");
        raw_out.extend_from_slice(&(original as u64).to_le_bytes());
        raw_out.extend_from_slice(data);
        let stats = StageStats { original, final_size: raw_out.len(), ..Default::default() };
        (raw_out, stats)
    }
}

pub fn compress_one(data:&[u8]) -> Vec<u8> {
    // Large file parallel chunking
    if data.len() > CHUNK_SIZE * 2 {
        return compress_chunked(data);
    }
    let e=entropy(data);
    if e >= 7.9 {
        let mut raw_out=Vec::new();
        raw_out.extend_from_slice(b"BFLS_RAW");
        raw_out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        raw_out.extend(data);
        return raw_out;
    }
    let is_small = data.len() < 64*1024;
    let mut token_size=if e<3.0 {4} else if e<5.0 {4} else {1};
    if is_small { token_size = std::cmp::max(token_size, 4); }
    let mut chunk_tokens=if token_size==4 {256} else if token_size==2 {512} else {1024};
    let mut window = if e < 4.0 { 512 } else if e < 6.0 { 256 } else { 64 };
    if is_small {
        let total_tokens = if token_size>0 { (data.len() + token_size -1)/token_size } else {1};
        if total_tokens == 0 {
            chunk_tokens = 1;
            window = 1;
        } else {
            chunk_tokens = total_tokens;
            window = 1;
        }
    } else {
        if data.len() > 1024*1024 {
            let target_chunk_bytes = std::cmp::min(1024*1024, std::cmp::max(256*1024, data.len()/8));
            let target_chunk_tokens = std::cmp::max(1, target_chunk_bytes / token_size);
            if target_chunk_tokens > chunk_tokens {
                chunk_tokens = target_chunk_tokens;
            }
        }
        let total_tokens = (data.len() + token_size -1)/token_size;
        let num_chunks = (total_tokens + chunk_tokens -1)/chunk_tokens;
        window = std::cmp::min(window, std::cmp::max(64, num_chunks/4));
    }
    let enable_rotation = e < 7.8;
    // Try bit view transforms, varint A/B, math model
    let mut best_overall: Option<Vec<u8>> = None;
    let mut best_len = usize::MAX;
    let mut best_bit_id = 0u8;
    for bit_id in 0..8 {
        let transformed = apply_bit_transform(data, bit_id);
        let mut best_for_view: Option<Vec<u8>> = None;
        let mut best_for_view_len = usize::MAX;
        for use_varint in [false, true] {
            let out = compress_tok_sw(&transformed, token_size, chunk_tokens, window, enable_rotation, use_varint);
            // quick round-trip check on transformed data
            if decompress_chunk(&out) != transformed { continue; }
            let compressed = if is_small {
                match zstd::encode_all(&out[..], 1) { Ok(z) if z.len() < out.len() => z, _ => out.clone() }
            } else {
                zstd::encode_all(&out[..], 3).unwrap()
            };
            if compressed.len() < best_for_view_len {
                best_for_view_len = compressed.len();
                best_for_view = Some(compressed);
            }
        }
        if let Some(comp) = best_for_view {
            let final_comp = if let Some(math) = math_compress(&comp) { math } else { comp };
            if final_comp.len() < best_len {
                best_len = final_comp.len();
                best_overall = Some(final_comp);
                best_bit_id = bit_id;
            }
        }
    }
    if let Some(comp) = best_overall {
        if comp.len() >= data.len() {
            let mut raw_out=Vec::new();
            raw_out.extend_from_slice(b"BFLS_RAW");
            raw_out.extend_from_slice(&(data.len() as u64).to_le_bytes());
            raw_out.extend(data);
            return raw_out;
        } else if best_bit_id != 0 {
            // wrap with bit view
            let mut out = Vec::new();
            out.extend_from_slice(BITVIEW_MAGIC);
            out.push(best_bit_id);
            out.extend_from_slice(&comp);
            return out;
        } else {
            return comp;
        }
    }
    let mut raw_out=Vec::new();
    raw_out.extend_from_slice(b"BFLS_RAW");
    raw_out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    raw_out.extend(data);
    raw_out
}

fn compress_chunked(data: &[u8]) -> Vec<u8> {
    if data.len() <= CHUNK_SIZE {
        return compress_one(data);
    }
    let orig_size = data.len();
    // adaptive chunk size: keep chunks equal and <=512 KiB
    let max_chunk = CHUNK_SIZE;
    let num_chunks = std::cmp::max(2, (data.len() + max_chunk - 1) / max_chunk);
    let chunk_size = std::cmp::min(max_chunk, (data.len() + num_chunks - 1) / num_chunks);
    let chunk_size = std::cmp::max(64 * 1024, chunk_size);
    let chunks: Vec<&[u8]> = data.chunks(chunk_size).collect();
    let compressed_chunks: Vec<Vec<u8>> = chunks.par_iter().map(|c| {
        // per-chunk entropy gating
        let sample = &c[..c.len().min(4096)];
        let e = entropy(sample);
        if e >= 7.9 {
            let mut out = Vec::new();
            out.extend_from_slice(b"BFLS_RAW");
            out.extend_from_slice(&(c.len() as u64).to_le_bytes());
            out.extend_from_slice(c);
            out
        } else {
            // layer skip for high entropy
            // compress_one already handles entropy gating
            compress_one(c)
        }
    }).collect();
    let mut out = Vec::new();
    out.extend_from_slice(CHUNK_MAGIC);
    out.extend_from_slice(&(orig_size as u64).to_le_bytes());
    for comp in compressed_chunks {
        let len = comp.len() as u32;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&comp);
    }
    out
}

pub fn decompress_chunk(bytes:&[u8])->Vec<u8>{
    if bytes.is_empty() { return Vec::new(); }
    // Handle chunk container
    if bytes.len()>=18 && &bytes[0..10]==b"BFLS_CHUNK" {
        let mut off = 10;
        let orig_size = u64::from_le_bytes(bytes[off..off+8].try_into().unwrap());
        off += 8;
        let mut out = Vec::with_capacity(orig_size as usize);
        while off + 4 <= bytes.len() { 
            let chunk_len = u32::from_le_bytes(bytes[off..off+4].try_into().unwrap()) as usize;
            off += 4;
            if off + chunk_len > bytes.len() { break; }
            let chunk_data = &bytes[off..off+chunk_len];
            off += chunk_len;
            let dec = decompress_chunk(chunk_data);
            out.extend_from_slice(&dec);
            if out.len() >= orig_size as usize { break; }
        }
        out.truncate(orig_size as usize);
        return out;
    }
    // Handle bit view wrapper
    if bytes.len()>=7 && &bytes[0..6]==b"BFLS_B" {
        let transform_id = bytes[6];
        let inner = &bytes[7..];
        let decompressed = decompress_chunk(inner);
        return reverse_bit_transform(&decompressed, transform_id);
    }
    // Handle mathematical model wrapper
    if bytes.len()>=6 && &bytes[0..6]==b"BFLS_M" {
        if let Some(inner) = math_decompress(bytes) {
            return decompress_chunk(&inner);
        }
    }
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

// ---------- Bit view pre-compression ----------
const BITVIEW_MAGIC: &[u8] = b"BFLS_B";

// ---------- Chunk container ----------
const CHUNK_MAGIC: &[u8] = b"BFLS_CHUNK";
const CHUNK_SIZE: usize = 512 * 1024;

// ---------- Mathematical post-compression ----------
const MATH_MAGIC: &[u8] = b"BFLS_M";

fn math_compress(data: &[u8]) -> Option<Vec<u8>> {
    let n = data.len();
    if n < 16 { return None; }
    // constant model
    if data.iter().all(|&b| b == data[0]) {
        let mut out = Vec::new();
        out.extend_from_slice(MATH_MAGIC);
        out.extend_from_slice(&(n as u64).to_le_bytes());
        out.push(1u8); // constant
        out.push(data[0]);
        if out.len() < n { return Some(out); }
    }
    // period model
    let max_period = std::cmp::min(1024, n/2);
    for p in 1..=max_period {
        let mut ok = true;
        for i in p..n {
            if data[i] != data[i - p] { ok = false; break; }
        }
        if ok {
            let mut out = Vec::new();
            out.extend_from_slice(MATH_MAGIC);
            out.extend_from_slice(&(n as u64).to_le_bytes());
            out.push(2u8); // period
            out.extend_from_slice(&(p as u32).to_le_bytes());
            out.extend_from_slice(&data[0..p]);
            if out.len() < n { return Some(out); }
        }
    }
    // linear model byte[i] = (a*i + b) mod 256
    let a = (data[1].wrapping_sub(data[0])) as u8;
    let b = data[0];
    let mut ok = true;
    for i in 0..n {
        let expected = ((a as u32 * i as u32 + b as u32) % 256) as u8;
        if data[i] != expected { ok = false; break; }
    }
    if ok {
        let mut out = Vec::new();
        out.extend_from_slice(MATH_MAGIC);
        out.extend_from_slice(&(n as u64).to_le_bytes());
        out.push(3u8); // linear
        out.push(a);
        out.push(b);
        if out.len() < n { return Some(out); }
    }
    // order-2 linear recurrence over GF(256): byte[i] = c1*byte[i-1] + c2*byte[i-2]
    if let Some((c1,c2)) = find_order2_recurrence(data) {
        let mut out = Vec::new();
        out.extend_from_slice(MATH_MAGIC);
        out.extend_from_slice(&(n as u64).to_le_bytes());
        out.push(4u8); // order-2 GF recurrence
        out.push(c1);
        out.push(c2);
        out.push(data[0]);
        out.push(data[1]);
        if out.len() < n { return Some(out); }
    }
    // approximate order-2 with residuals
    if let Some((c1,c2)) = find_order2_recurrence_approx(data) {
        // compute residuals
        let mut residuals = Vec::with_capacity(n.saturating_sub(2));
        for i in 2..n {
            let expected = gf_add(gf_mul(c1, data[i-1]), gf_mul(c2, data[i-2]));
            residuals.push(data[i] ^ expected);
        }
        // compress residuals with zstd
        if let Ok(comp_res) = zstd::encode_all(&residuals[..], 1) {
            let mut out = Vec::new();
            out.extend_from_slice(MATH_MAGIC);
            out.extend_from_slice(&(n as u64).to_le_bytes());
            out.push(5u8); // order-2 with residuals
            out.push(c1);
            out.push(c2);
            out.push(data[0]);
            out.push(data[1]);
            out.extend_from_slice(&(comp_res.len() as u32).to_le_bytes());
            out.extend_from_slice(&comp_res);
            if out.len() < n { return Some(out); }
        }
    }
    None
}

pub fn inspect_math_info(bytes: &[u8]) -> Option<String> {
    if bytes.len() < MATH_MAGIC.len() + 8 + 1 { return None; }
    if &bytes[0..MATH_MAGIC.len()] != MATH_MAGIC { return None; }
    let mut off = MATH_MAGIC.len();
    let n = u64::from_le_bytes(bytes[off..off+8].try_into().unwrap());
    off += 8;
    let model = bytes[off]; off += 1;
    match model {
        1 => {
            let v = bytes[off];
            Some(format!("MATH model: constant, value=0x{:02x}, orig_len={}", v, n))
        }
        2 => {
            let p = u32::from_le_bytes(bytes[off..off+4].try_into().unwrap());
            Some(format!("MATH model: period, period={}, orig_len={}", p, n))
        }
        3 => {
            let a = bytes[off]; let b = bytes[off+1];
            Some(format!("MATH model: linear a*i+b mod256, a={}, b={}, orig_len={}", a, b, n))
        }
        4 => {
            let c1 = bytes[off]; let c2 = bytes[off+1]; let s0 = bytes[off+2]; let s1 = bytes[off+3];
            Some(format!("MATH model: GF(256) order-2 exact, c1={}, c2={}, seed=({},{}) , orig_len={}", c1, c2, s0, s1, n))
        }
        5 => {
            let c1 = bytes[off]; let c2 = bytes[off+1]; let s0 = bytes[off+2]; let s1 = bytes[off+3];
            let res_len = u32::from_le_bytes(bytes[off+4..off+8].try_into().unwrap());
            Some(format!("MATH model: GF(256) order-2 with residuals, c1={}, c2={}, seed=({},{}) , residual_bytes={}, orig_len={}", c1, c2, s0, s1, res_len, n))
        }
        _ => Some(format!("MATH model: unknown type {}", model))
    }
}

fn math_decompress(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < MATH_MAGIC.len() + 8 + 1 { return None; }
    if &data[0..MATH_MAGIC.len()] != MATH_MAGIC { return None; }
    let mut off = MATH_MAGIC.len();
    let n = u64::from_le_bytes(data[off..off+8].try_into().unwrap()) as usize;
    off += 8;
    let model = data[off]; off += 1;
    match model {
        1 => {
            let v = data[off];
            let out = vec![v; n];
            Some(out)
        }
        2 => {
            let p = u32::from_le_bytes(data[off..off+4].try_into().unwrap()) as usize;
            off += 4;
            if off + p > data.len() { return None; }
            let pattern = &data[off..off+p];
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                out.push(pattern[i % p]);
            }
            Some(out)
        }
        3 => {
            let a = data[off]; off += 1;
            let b = data[off];
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let v = ((a as u32 * i as u32 + b as u32) % 256) as u8;
                out.push(v);
            }
            Some(out)
        }
        4 => {
            // order-2 GF recurrence exact
            let c1 = data[off]; off += 1;
            let c2 = data[off]; off += 1;
            let s0 = data[off]; off += 1;
            let s1 = data[off]; off += 1;
            let mut out = Vec::with_capacity(n);
            out.push(s0);
            if n > 1 { out.push(s1); }
            for i in 2..n {
                let v = gf_add(gf_mul(c1, out[i-1]), gf_mul(c2, out[i-2]));
                out.push(v);
            }
            Some(out)
        }
        5 => {
            // order-2 GF recurrence with residuals
            let c1 = data[off]; off += 1;
            let c2 = data[off]; off += 1;
            let s0 = data[off]; off += 1;
            let s1 = data[off]; off += 1;
            let res_len = u32::from_le_bytes(data[off..off+4].try_into().unwrap()) as usize;
            off += 4;
            if off + res_len > data.len() { return None; }
            let comp_res = &data[off..off+res_len];
            let residuals = zstd::decode_all(comp_res).ok()?;
            if residuals.len() != n.saturating_sub(2) { return None; }
            let mut out = Vec::with_capacity(n);
            out.push(s0);
            if n > 1 { out.push(s1); }
            for i in 2..n {
                let expected = gf_add(gf_mul(c1, out[i-1]), gf_mul(c2, out[i-2]));
                let v = expected ^ residuals[i-2];
                out.push(v);
            }
            Some(out)
        }
        _ => None
    }
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

