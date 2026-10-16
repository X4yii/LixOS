use std::time::Instant;
#[inline(always)]
pub fn rdtsc_start() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_mm_lfence();
        core::arch::x86_64::_rdtsc()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}
#[inline(always)]
pub fn rdtsc_end() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let mut aux = 0u32;
        core::arch::x86_64::__rdtscp(&raw mut aux)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}
#[allow(dead_code)]
pub struct BenchmarkResult {
    pub name: &'static str,
    pub iterations: u64,
    pub total_cycles: u64,
    pub avg_cycles: f64,
    pub avg_nanos: f64,
    pub ops_per_sec: f64,
    pub mb_per_sec: Option<f64>,
}
impl BenchmarkResult {
    pub fn print_row(&self) {
        if let Some(mb) = self.mb_per_sec {
            println!(
                " │ {:<30} │ {:>10} │ {:>12.1} │ {:>10.2} ns │ {:>10.2} MB/s │",
                self.name, self.iterations, self.avg_cycles, self.avg_nanos, mb
            );
        } else {
            println!(
                " │ {:<30} │ {:>10} │ {:>12.1} │ {:>10.2} ns │ {:>10.2} Mop/s │",
                self.name,
                self.iterations,
                self.avg_cycles,
                self.avg_nanos,
                self.ops_per_sec / 1_000_000.0
            );
        }
    }
}
fn bench_buddy_xor_coalesce() -> BenchmarkResult {
    let iterations = 10_000_000u64;
    let mut block = 0x1000usize;
    let start_cycles = rdtsc_start();
    let start_time = Instant::now();
    for i in 0..iterations {
        let order = (i & 0x7) as usize;
        block ^= (1 << order);
    }
    std::hint::black_box(block);
    let end_cycles = rdtsc_end();
    let elapsed = start_time.elapsed();
    let total_cycles = end_cycles.saturating_sub(start_cycles);
    let avg_cycles = total_cycles as f64 / iterations as f64;
    let avg_nanos = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    BenchmarkResult {
        name: "Buddy: XOR Coalescing",
        iterations,
        total_cycles,
        avg_cycles,
        avg_nanos,
        ops_per_sec,
        mb_per_sec: None,
    }
}
fn bench_vfs_path_normalize() -> BenchmarkResult {
    let iterations = 500_000u64;
    let sample_path = "/usr/local/bin/../../var/log/../run/./app/config.json";
    let start_cycles = rdtsc_start();
    let start_time = Instant::now();
    for _ in 0..iterations {
        let mut components: Vec<&str> = Vec::with_capacity(8);
        for comp in sample_path.split('/') {
            match comp {
                "" | "." => {}
                ".." => {
                    components.pop();
                }
                val => {
                    components.push(val);
                }
            }
        }
        let _ = components.join("/");
    }
    let end_cycles = rdtsc_end();
    let elapsed = start_time.elapsed();
    let total_cycles = end_cycles.saturating_sub(start_cycles);
    let avg_cycles = total_cycles as f64 / iterations as f64;
    let avg_nanos = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    BenchmarkResult {
        name: "VFS: Path Normalization",
        iterations,
        total_cycles,
        avg_cycles,
        avg_nanos,
        ops_per_sec,
        mb_per_sec: None,
    }
}
fn bench_lz4_compression_text() -> BenchmarkResult {
    let iterations = 100_000u64;
    let mut page = [0u8; 4096];
    let sample_text =
        b"fn main() { println!(\"LixOS high performance microbenchmark kernel testing\"); }\n";
    for i in 0..4096 {
        page[i] = sample_text[i % sample_text.len()];
    }
    let mut out_buf = [0u8; 8192];
    let start_cycles = rdtsc_start();
    let start_time = Instant::now();
    for _ in 0..iterations {
        let _ = lz4_flex::compress_into(&page, &mut out_buf);
    }
    let end_cycles = rdtsc_end();
    let elapsed = start_time.elapsed();
    let total_cycles = end_cycles.saturating_sub(start_cycles);
    let avg_cycles = total_cycles as f64 / iterations as f64;
    let avg_nanos = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    let total_bytes = iterations * 4096;
    let mb_per_sec = (total_bytes as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();
    BenchmarkResult {
        name: "LZ4: Comprimir 4 KiB (Código)",
        iterations,
        total_cycles,
        avg_cycles,
        avg_nanos,
        ops_per_sec,
        mb_per_sec: Some(mb_per_sec),
    }
}
fn bench_lz4_decompression_text() -> BenchmarkResult {
    let iterations = 100_000u64;
    let mut page = [0u8; 4096];
    let sample_text =
        b"fn main() { println!(\"LixOS high performance microbenchmark kernel testing\"); }\n";
    for i in 0..4096 {
        page[i] = sample_text[i % sample_text.len()];
    }
    let compressed = lz4_flex::compress(&page);
    let mut decompressed = [0u8; 4096];
    let start_cycles = rdtsc_start();
    let start_time = Instant::now();
    for _ in 0..iterations {
        let _ = lz4_flex::decompress_into(&compressed, &mut decompressed);
    }
    let end_cycles = rdtsc_end();
    let elapsed = start_time.elapsed();
    let total_cycles = end_cycles.saturating_sub(start_cycles);
    let avg_cycles = total_cycles as f64 / iterations as f64;
    let avg_nanos = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    let total_bytes = iterations * 4096;
    let mb_per_sec = (total_bytes as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();
    BenchmarkResult {
        name: "LZ4: Descomprimir 4 KiB",
        iterations,
        total_cycles,
        avg_cycles,
        avg_nanos,
        ops_per_sec,
        mb_per_sec: Some(mb_per_sec),
    }
}
fn bench_pcp_cache_fastpath() -> BenchmarkResult {
    let iterations = 2_000_000u64;
    let mut cache: Vec<usize> = Vec::with_capacity(128);
    for i in 0..64 {
        cache.push(i);
    }
    let start_cycles = rdtsc_start();
    let start_time = Instant::now();
    for i in 0..iterations {
        if let Some(frame) = cache.pop() {
            cache.push(frame + 1);
        } else {
            cache.push(i as usize);
        }
    }
    let end_cycles = rdtsc_end();
    let elapsed = start_time.elapsed();
    let total_cycles = end_cycles.saturating_sub(start_cycles);
    let avg_cycles = total_cycles as f64 / iterations as f64;
    let avg_nanos = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    BenchmarkResult {
        name: "PCP: Cache L1 Fastpath",
        iterations,
        total_cycles,
        avg_cycles,
        avg_nanos,
        ops_per_sec,
        mb_per_sec: None,
    }
}
fn bench_shell_tokenizer() -> BenchmarkResult {
    let iterations = 500_000u64;
    let cmd = "echo \"Hola Mundo desde LixOS con comillas dobles\" > /tmp/nota.txt";
    let start_cycles = rdtsc_start();
    let start_time = Instant::now();
    for _ in 0..iterations {
        let mut tokens = Vec::with_capacity(8);
        let mut current = String::with_capacity(32);
        let mut in_quotes = false;
        let mut quote_char = ' ';
        for c in cmd.chars() {
            if in_quotes {
                if c == quote_char {
                    in_quotes = false;
                } else {
                    current.push(c);
                }
            } else if c == '"' || c == '\'' {
                in_quotes = true;
                quote_char = c;
            } else if c.is_whitespace() {
                if !current.is_empty() {
                    tokens.push(current);
                    current = String::with_capacity(32);
                }
            } else {
                current.push(c);
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
    }
    let end_cycles = rdtsc_end();
    let elapsed = start_time.elapsed();
    let total_cycles = end_cycles.saturating_sub(start_cycles);
    let avg_cycles = total_cycles as f64 / iterations as f64;
    let avg_nanos = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    BenchmarkResult {
        name: "Shell: Tokenizer con Comillas",
        iterations,
        total_cycles,
        avg_cycles,
        avg_nanos,
        ops_per_sec,
        mb_per_sec: None,
    }
}
pub fn run_benchmarks() {
    println!("\n╔══════════════════════════════════════════════════════════════════════════════════════════╗");
    println!("║                   LixOS Microbenchmark Suite (100% Rust / RDTSC)                         ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════════════════╣");
    println!(
        " │ {:<30} │ {:>10} │ {:>12} │ {:>13} │ {:>15} │",
        "Benchmark", "Iteraciones", "Ciclos/Op", "Latencia", "Throughput"
    );
    println!("╟──────────────────────────────────────────────────────────────────────────────────────────╢");
    let results = [
        bench_buddy_xor_coalesce(),
        bench_pcp_cache_fastpath(),
        bench_vfs_path_normalize(),
        bench_shell_tokenizer(),
        bench_lz4_compression_text(),
        bench_lz4_decompression_text(),
    ];
    for r in &results {
        r.print_row();
    }
    println!("╚══════════════════════════════════════════════════════════════════════════════════════════╝\n");
}
