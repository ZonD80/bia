//! llama.cpp runtime plan from sysfs, `/proc` and the GGUF header.
use std::fs;
use std::path::Path;

use crate::config::ModelCfg;
use crate::gguf;
use crate::sys;

#[derive(Clone, Debug)]
pub struct LlamaPlan {
    pub threads: i32,
    pub threads_batch: i32,
    /// Logical batch (`-b`). Prompt prefill.
    pub n_batch: u32,
    /// Physical batch (`-ub`), never above `n_batch`.
    pub n_ubatch: u32,
    pub ctx: u32,
    pub mlock: bool,
    pub mmap: bool,
    pub summary: String,
}

pub fn plan(cfg: &ModelCfg, model: &Path) -> LlamaPlan {
    let meta = gguf::read(model);
    let mem = sys::meminfo();
    let file_size = fs::metadata(model).map(|m| m.len()).unwrap_or(0);
    let (gen, batch_threads) = threads(cfg.threads);
    let mlock = cfg.mlock && file_fits(file_size, mem.available);
    let mut ctx = cfg.ctx.max(512);
    if meta.n_ctx_train > 0 {
        ctx = ctx.min(meta.n_ctx_train.max(512));
    }
    let mut shrunk = false;
    if let Some(per) = meta.kv_per_token {
        if per > 0 && mem.available > 0 {
            let mut budget = mem.available.saturating_sub(768 * 1024 * 1024u64);
            if mlock {
                budget = budget.saturating_sub(file_size);
            }
            let fit = (budget / per) as u32;
            let fit = fit.max(512);
            if ctx > fit {
                ctx = fit;
                shrunk = true;
            }
        }
    }
    let n_batch = batch_size(mem.available).min(ctx).max(32);
    let n_ubatch = n_batch.min(512).max(32);
    let mut summary = format!(
        "llama plan: {gen} gen threads, {batch_threads} prefill threads, batch {n_batch}/{n_ubatch}, ctx {ctx}"
    );
    if shrunk {
        summary.push_str(&format!(
            " (asked {}, trimmed to fit {})",
            cfg.ctx,
            bytes(mem.available)
        ));
    }
    if cfg.mlock && !mlock {
        summary.push_str("; mlock skipped, model is larger than free RAM");
    } else if mlock {
        summary.push_str(", mlock");
    } else if cfg.mmap {
        summary.push_str(", mmap");
    }
    LlamaPlan {
        threads: gen,
        threads_batch: batch_threads,
        n_batch,
        n_ubatch,
        ctx,
        mlock,
        mmap: cfg.mmap,
        summary,
    }
}

/// `(generation threads, prefill threads)`.
/// Generation uses one thread per performance core. Prefill uses every logical CPU.
fn threads(configured: i32) -> (i32, i32) {
    let logical = sys::ncpu().max(1) as i32;
    if configured > 0 {
        return (configured, configured);
    }
    let ids = cpu_ids();
    if ids.is_empty() {
        return (logical, logical);
    }
    let mut cores = Vec::new();
    let mut max_cap = 0u32;
    for id in &ids {
        let base = format!("/sys/devices/system/cpu/cpu{id}/topology");
        let core = read_u32(&format!("{base}/core_id")).unwrap_or(*id);
        let pkg = read_u32(&format!("{base}/physical_package_id")).unwrap_or(0);
        let cap = read_u32(&format!("/sys/devices/system/cpu/cpu{id}/cpu_capacity")).unwrap_or(0);
        max_cap = max_cap.max(cap);
        cores.push((pkg, core, cap));
    }
    let gen_cores: usize = if max_cap > 0 {
        let floor = max_cap.saturating_mul(3) / 4;
        unique_cores(
            cores
                .iter()
                .filter(|(_, _, c)| *c >= floor)
                .map(|(p, c, _)| (*p, *c)),
        )
    } else {
        unique_cores(cores.iter().map(|(p, c, _)| (*p, *c)))
    };
    let gen = (gen_cores as i32).clamp(1, logical);
    (gen, logical.max(gen))
}

fn unique_cores(it: impl Iterator<Item = (u32, u32)>) -> usize {
    let mut v: Vec<(u32, u32)> = it.collect();
    v.sort_unstable();
    v.dedup();
    v.len()
}

fn cpu_ids() -> Vec<u32> {
    let s = fs::read_to_string("/sys/devices/system/cpu/online").unwrap_or_default();
    let mut out = Vec::new();
    for part in s.split(|c: char| c == ',' || c.is_whitespace()) {
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) else {
                continue;
            };
            for id in a..=b {
                out.push(id);
            }
        } else if let Ok(id) = part.parse::<u32>() {
            out.push(id);
        }
    }
    out
}

fn read_u32(path: &str) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn file_fits(file: u64, available: u64) -> bool {
    if file == 0 || available == 0 {
        return false;
    }
    file.saturating_add(512 * 1024 * 1024u64) < available
}

fn batch_size(available: u64) -> u32 {
    const GIB: u64 = 1 << 30;
    if available == 0 || available > 8 * GIB {
        2048
    } else if available > 3 * GIB {
        1024
    } else if available > (1536u64 << 20) {
        512
    } else {
        128
    }
}

fn bytes(n: u64) -> String {
    if n >= (1u64 << 30) {
        format!("{:.1} GiB", n as f64 / (1u64 << 30) as f64)
    } else {
        format!("{} MiB", n / (1u64 << 20))
    }
}

/// `performance` on AC power. Laptops on battery keep the governor Debian set.
pub fn performance_governor() -> Option<String> {
    if on_battery() {
        return None;
    }
    let mut n = 0u32;
    let Ok(rd) = fs::read_dir("/sys/devices/system/cpu") else {
        return None;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("cpu") || !name[3..].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let p = e.path().join("cpufreq/scaling_governor");
        if fs::write(&p, "performance").is_ok() {
            n += 1;
        }
    }
    (n > 0).then(|| format!("governor performance ({n} cpus)"))
}

fn on_battery() -> bool {
    let Ok(rd) = fs::read_dir("/sys/class/power_supply") else {
        return false;
    };
    for e in rd.flatten() {
        let kind = fs::read_to_string(e.path().join("type")).unwrap_or_default();
        if !kind.trim().eq_ignore_ascii_case("Battery") {
            continue;
        }
        let st = fs::read_to_string(e.path().join("status")).unwrap_or_default();
        if st.trim().eq_ignore_ascii_case("Discharging") {
            return true;
        }
    }
    false
}
