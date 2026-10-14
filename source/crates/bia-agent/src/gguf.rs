//! Enough of a GGUF header read to size the KV cache and name the model
//! before llama.cpp is asked to load it.
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub arch: String,
    pub name: String,
    pub n_params: u64,
    pub n_ctx_train: u32,
    pub n_layer: u32,
    pub experts: Option<String>,
    pub chat_template: String,
    pub has_bos: bool,
    /// K+V bytes per token at f16, when the architecture keys are present.
    pub kv_per_token: Option<u64>,
}

pub fn read(path: &Path) -> Meta {
    let Ok(mut f) = File::open(path) else {
        return Meta::default();
    };
    parse(&mut f).unwrap_or_default()
}

fn parse(f: &mut File) -> Option<Meta> {
    let magic = u32_at(f)?;
    if magic != 0x4655_4747 {
        return None;
    }
    let version = u32_at(f)?;
    let (n_tensors, n_kv) = if version >= 2 {
        (u64_at(f)?, u64_at(f)?)
    } else {
        (u32_at(f)? as u64, u32_at(f)? as u64)
    };
    let _ = n_tensors;
    let mut kv: Vec<(String, Val)> = Vec::new();
    for _ in 0..n_kv {
        let key = string_at(f)?;
        let ty = u32_at(f)?;
        let val = read_val(f, ty)?;
        kv.push((key, val));
    }
    let get = |k: &str| kv.iter().find(|(key, _)| key == k).map(|(_, v)| v);
    let arch = get("general.architecture")
        .and_then(Val::str)
        .unwrap_or("")
        .to_string();
    let raw_name = get("general.name").and_then(Val::str).unwrap_or("");
    let name = if raw_name.len() >= 32 && raw_name.chars().all(|c| c.is_ascii_hexdigit()) {
        get("general.basename")
            .and_then(Val::str)
            .unwrap_or("")
            .to_string()
    } else {
        raw_name.to_string()
    };
    let n_params = get("general.parameter_count")
        .and_then(Val::u64)
        .unwrap_or(0);
    let n_ctx_train = arch_u64(&kv, &arch, "context_length").unwrap_or(0) as u32;
    let n_layer = arch_u64(&kv, &arch, "block_count").unwrap_or(0) as u32;
    let experts = arch_u64(&kv, &arch, "expert_count")
        .filter(|n| *n > 0)
        .map(|n| {
            let used = arch_u64(&kv, &arch, "expert_used_count").unwrap_or(0);
            format!("{n} experts, {used} active")
        });
    let chat_template = get("tokenizer.chat_template")
        .and_then(Val::str)
        .unwrap_or("")
        .to_string();
    let has_bos = get("tokenizer.ggml.bos_token_id")
        .and_then(Val::i64)
        .map(|n| n >= 0)
        .unwrap_or(true);
    let kv_per_token = kv_bytes(&kv, &arch);
    Some(Meta {
        arch,
        name,
        n_params,
        n_ctx_train,
        n_layer,
        experts,
        chat_template,
        has_bos,
        kv_per_token,
    })
}

fn arch_u64(kv: &[(String, Val)], arch: &str, leaf: &str) -> Option<u64> {
    if arch.is_empty() {
        return None;
    }
    kv.iter()
        .find(|(k, _)| k == &format!("{arch}.{leaf}"))
        .and_then(|(_, v)| v.u64())
}

fn kv_bytes(kv: &[(String, Val)], arch: &str) -> Option<u64> {
    let n_layer = arch_u64(kv, arch, "block_count")?;
    let n_embd = arch_u64(kv, arch, "embedding_length")?;
    let n_head = arch_u64(kv, arch, "attention.head_count").unwrap_or(0);
    let n_head_kv = arch_u64(kv, arch, "attention.head_count_kv").unwrap_or(n_head);
    let key_len = arch_u64(kv, arch, "attention.key_length")
        .or_else(|| (n_head > 0).then_some(n_embd / n_head))?;
    if n_layer == 0 || key_len == 0 || n_head_kv == 0 {
        return None;
    }
    // K and V, f16.
    Some(
        n_layer
            .saturating_mul(key_len)
            .saturating_mul(n_head_kv)
            .saturating_mul(4),
    )
}

enum Val {
    Num(i64),
    Big(u64),
    Str(String),
    Skip,
}

impl Val {
    fn str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }
    fn u64(&self) -> Option<u64> {
        match self {
            Val::Big(n) => Some(*n),
            Val::Num(n) if *n >= 0 => Some(*n as u64),
            _ => None,
        }
    }
    fn i64(&self) -> Option<i64> {
        match self {
            Val::Num(n) => Some(*n),
            Val::Big(n) if *n <= i64::MAX as u64 => Some(*n as i64),
            _ => None,
        }
    }
}

fn read_val(f: &mut File, ty: u32) -> Option<Val> {
    Some(match ty {
        0 => Val::Num(u8_at(f)? as i64),
        1 => Val::Num(i8_at(f)? as i64),
        2 => Val::Num(u16_at(f)? as i64),
        3 => Val::Num(i16_at(f)? as i64),
        4 => Val::Num(u32_at(f)? as i64),
        5 => Val::Num(i32_at(f)? as i64),
        6 => {
            let _ = f32_at(f)?;
            Val::Skip
        }
        7 => {
            u8_at(f)?;
            Val::Skip
        }
        8 => Val::Str(string_at(f)?),
        9 => {
            let elem = u32_at(f)?;
            let n = u64_at(f)?;
            for _ in 0..n {
                let _ = read_val(f, elem)?;
            }
            Val::Skip
        }
        10 => Val::Big(u64_at(f)?),
        11 => Val::Num(i64_at(f)?),
        12 => {
            let _ = f64_at(f)?;
            Val::Skip
        }
        _ => return None,
    })
}

fn string_at(f: &mut File) -> Option<String> {
    let n = u64_at(f)?;
    if n > 16 * 1024 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; n as usize];
    f.read_exact(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn u8_at(f: &mut File) -> Option<u8> {
    let mut b = [0u8; 1];
    f.read_exact(&mut b).ok()?;
    Some(b[0])
}
fn i8_at(f: &mut File) -> Option<i8> {
    Some(u8_at(f)? as i8)
}
fn u16_at(f: &mut File) -> Option<u16> {
    let mut b = [0u8; 2];
    f.read_exact(&mut b).ok()?;
    Some(u16::from_le_bytes(b))
}
fn i16_at(f: &mut File) -> Option<i16> {
    Some(u16_at(f)? as i16)
}
fn u32_at(f: &mut File) -> Option<u32> {
    let mut b = [0u8; 4];
    f.read_exact(&mut b).ok()?;
    Some(u32::from_le_bytes(b))
}
fn i32_at(f: &mut File) -> Option<i32> {
    Some(u32_at(f)? as i32)
}
fn u64_at(f: &mut File) -> Option<u64> {
    let mut b = [0u8; 8];
    f.read_exact(&mut b).ok()?;
    Some(u64::from_le_bytes(b))
}
fn i64_at(f: &mut File) -> Option<i64> {
    Some(u64_at(f)? as i64)
}
fn f32_at(f: &mut File) -> Option<f32> {
    Some(f32::from_le_bytes(u32_at(f)?.to_le_bytes()))
}
fn f64_at(f: &mut File) -> Option<f64> {
    Some(f64::from_le_bytes(u64_at(f)?.to_le_bytes()))
}
