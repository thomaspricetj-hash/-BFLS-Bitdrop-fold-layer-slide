use eframe::egui;
use std::path::PathBuf;
use std::fs;
use sha2::{Sha256, Digest};
use bfls_studio::engine;

struct BflsStudio {
    files: Vec<PathBuf>,
    status: String,
    progress: f32,
    log: Vec<String>,
    show_advanced: bool,
    last_stats: Option<Stats>,
}

#[derive(Default)]
struct Stats {
    orig: usize,
    comp: usize,
    ratio: f64,
    encode_ms: u128,
    decode_ms: u128,
    entropy: f64,
    token_size: usize,
    bitdrop: usize,
    fold: usize,
    layer: usize,
    slide: usize,
    raw: usize,
    fold_layer_slide: usize,
    zstd: usize,
    final_size: usize,
}

impl Default for BflsStudio {
    fn default() -> Self {
        Self { files: Vec::new(), status: "Drop files here".into(), progress: 0.0, log: Vec::new(), show_advanced: false, last_stats: None }
    }
}

impl eframe::App for BflsStudio {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("BFLS Compression Studio");
            ui.separator();

            // Drop zone
            let _drop_response = ui.allocate_rect(ui.available_rect_before_wrap(), egui::Sense::drag());
            if ui.input(|i| i.raw.hovered_files.clone()).iter().any(|f| f.path.is_some()) {
                ui.label("Drop files here");
            } else {
                ui.label("Drop files here");
            }
            // Handle dropped files
            if let Some(file) = ui.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone())) {
                if !self.files.contains(&file) {
                    self.files.push(file);
                }
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Compress").clicked() {
                    self.compress(ctx);
                }
                if ui.button("Decompress").clicked() {
                    self.decompress(ctx);
                }
            });

            ui.separator();
            ui.label("Selected Files:");
            for f in &self.files {
                ui.label(f.display().to_string());
            }

            ui.separator();
            if ui.checkbox(&mut self.show_advanced, "Advanced Statistics").changed() {}

            ui.add(egui::ProgressBar::new(self.progress).show_percentage());
            ui.label(&self.status);

            if self.show_advanced {
                ui.separator();
                ui.collapsing("Advanced Panel", |ui| {
                    if let Some(s) = &self.last_stats {
                        ui.label(format!("Original Size: {} B", s.orig));
                        ui.label(format!("Compressed Size: {} B", s.comp));
                        ui.label(format!("Final Size: {} B", s.final_size));
                        ui.label(format!("Ratio: {:.2}%", s.ratio * 100.0));
                        ui.label(format!("Encode Time: {} ms", s.encode_ms));
                        ui.label(format!("Decode Time: {} ms", s.decode_ms));
                        ui.label(format!("Entropy: {:.3}", s.entropy));
                        ui.label(format!("Token Size: {}", s.token_size));
                        ui.separator();
                        ui.label("Stage breakdown:");
                        ui.label(format!("  BitDrop size: {} B", s.bitdrop));
                        ui.label(format!("  Raw payload: {} B", s.raw));
                        ui.label(format!("  Fold refs: {} B", s.fold));
                        ui.label(format!("  Slide refs: {} B", s.slide));
                        ui.label(format!("  Layer refs: {} B", s.layer));
                        ui.label(format!("  Fold/Layer/Slide payload: {} B", s.fold_layer_slide));
                        ui.label(format!("  zstd size: {} B", s.zstd));
                        ui.label(format!("  Final size: {} B", s.final_size));
                    } else {
                        ui.label("No stats available yet");
                    }
                });
            }

            ui.separator();
            ui.label("Log:");
            egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                for line in &self.log {
                    ui.label(line);
                }
            });
        });
    }
}

impl BflsStudio {
    fn compress(&mut self, _ctx: &egui::Context) {
        if self.files.is_empty() {
            self.status = "No files selected".into();
            return;
        }
        self.log.clear();
        self.status = "Compressing...".into();
        let total = self.files.len();
        // pre-compute total bytes for ETA
        let total_bytes: u64 = self.files.iter().filter_map(|f| std::fs::metadata(f).ok().map(|m| m.len())).sum();
        let mut processed_bytes: u64 = 0;
        let start = std::time::Instant::now();
        for (i, f) in self.files.iter().enumerate() {
            let data = match fs::read(f) {
                Ok(d) => d,
                Err(e) => { self.log.push(format!("Read error {}: {}", f.display(), e)); continue; }
            };
            let orig = data.len();
            processed_bytes += orig as u64;
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let hash = hex::encode(hasher.finalize());
            let entropy = engine::entropy(&data);
            let token_size = if entropy < 3.0 {4} else if entropy < 5.0 {4} else {1};
            let t0 = std::time::Instant::now();
            let (payload, stage) = engine::compress_with_stats(&data);
            let encode_ms = t0.elapsed().as_millis();
            let out_path = f.with_extension("bfls");
            if let Err(e) = fs::write(&out_path, &payload) {
                self.log.push(format!("Write error {}: {}", out_path.display(), e));
                continue;
            }
            let comp = payload.len();
            let ratio = comp as f64 / orig as f64;
            let line = format!("{} → {} : {} → {} B {:.2}% SHA256 {}", f.display(), out_path.display(), orig, comp, ratio*100.0, &hash[..16]);
            self.log.push(line.clone());
            self.last_stats = Some(Stats { orig, comp, ratio, encode_ms, decode_ms: 0, entropy, token_size, bitdrop: stage.bitdrop, fold: stage.fold, layer: stage.layer, slide: stage.slide, raw: stage.raw, fold_layer_slide: stage.fold_layer_slide, zstd: stage.zstd, final_size: stage.final_size });
            let elapsed = start.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 { processed_bytes as f64 / elapsed } else { 0.0 };
            let remaining_bytes = total_bytes.saturating_sub(processed_bytes);
            let eta_secs = if speed > 0.0 { remaining_bytes as f64 / speed } else { 0.0 };
            self.status = format!("Compressing {} ({}/{}) – ETA {:.0}s", f.file_name().unwrap_or_default().to_string_lossy(), i+1, total, eta_secs);
            self.progress = (i+1) as f32 / total as f32;
        }
        self.progress = 1.0;
        self.status = "Compression finished".into();
    }

    fn decompress(&mut self, _ctx: &egui::Context) {
        if self.files.is_empty() {
            self.status = "No files selected".into();
            return;
        }
        self.log.clear();
        self.status = "Decompressing...".into();
        let bfls_files: Vec<_> = self.files.iter().filter(|f| f.extension().and_then(|e| e.to_str()) == Some("bfls")).collect();
        if bfls_files.is_empty() {
            self.status = "No .bfls files selected".into();
            return;
        }
        let total = bfls_files.len();
        for (i, f) in bfls_files.iter().enumerate() {
            let in_bytes = match fs::read(f) {
                Ok(b) => b,
                Err(e) => { self.log.push(format!("Read error {}: {}", f.display(), e)); continue; }
            };
            let t0 = std::time::Instant::now();
            let data = engine::decompress_chunk(&in_bytes);
            let decode_ms = t0.elapsed().as_millis();
            let stem = f.file_stem().unwrap().to_string_lossy().to_string();
            let parent = f.parent().unwrap_or_else(|| std::path::Path::new("."));
            let out_path = parent.join(stem);
            if let Err(e) = fs::write(&out_path, &data) {
                self.log.push(format!("Write error {}: {}", out_path.display(), e));
                continue;
            }
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let hash = hex::encode(hasher.finalize());
            let comp = in_bytes.len();
            let orig = data.len();
            let ratio = if comp>0 { orig as f64 / comp as f64 } else { 0.0 };
            let line = format!("{} → {} : {} B decompressed SHA256 {}", f.display(), out_path.display(), data.len(), &hash[..16]);
            self.log.push(line);
            self.last_stats = Some(Stats { orig, comp, ratio, encode_ms: 0, decode_ms, entropy: 0.0, token_size: 0, bitdrop: 0, fold: 0, layer: 0, slide: 0, raw: 0, fold_layer_slide: 0, zstd: 0, final_size: orig });
            self.status = format!("Decompressed {}/{}", i+1, total);
            self.progress = (i+1) as f32 / total as f32;
        }
        self.progress = 1.0;
        self.status = "Decompression finished".into();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bfls_files: Vec<std::path::PathBuf> = args.iter()
        .filter_map(|a| {
            let p = std::path::Path::new(a);
            if p.extension().and_then(|e| e.to_str()) == Some("bfls") && p.exists() {
                Some(p.to_path_buf())
            } else { None }
        })
        .collect();

    if !bfls_files.is_empty() {
        // Double-click handler: decompress each .bfls file silently
        for f in bfls_files {
            if let Ok(bytes) = std::fs::read(&f) {
                if let Ok(data) = std::panic::catch_unwind(|| bfls_studio::engine::decompress_chunk(&bytes)) {
                    if let Some(stem) = f.file_stem() {
                        let parent = f.parent().unwrap_or_else(|| std::path::Path::new("."));
                        let out_path = parent.join(stem);
                        let _ = std::fs::write(&out_path, &data);
                    }
                }
            }
        }
        // Exit silently for file association
        std::process::exit(0);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "BFLS Compression Studio",
        options,
        Box::new(|_cc| Box::<BflsStudio>::default()),
    ).unwrap();
}
