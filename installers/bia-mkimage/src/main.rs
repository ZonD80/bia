//! bia-mkimage <image-or-device> <size-bytes>
//! Writes the GPT (ESP + BIOS boot + Debian root) and prints the partition offsets.
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

fn is_device(path: &str) -> bool {
    Path::new(path).starts_with("/dev")
        || std::fs::metadata(path)
            .map(|m| {
                let t = m.file_type();
                t.is_block_device() || t.is_char_device()
            })
            .unwrap_or(false)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: bia-mkimage <image-or-device> <size-bytes>");
        std::process::exit(2);
    }
    let size: u64 = args[2].parse().context("size")?;
    let device = is_device(&args[1]);
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(!device)
        .truncate(!device)
        .open(&args[1])
        .with_context(|| format!("open {}", args[1]))?;
    if !device {
        f.set_len(size)?;
    }
    let l = bia_disk::write_gpt(&mut f, size)?;
    f.sync_all().ok();
    println!(
        "{} {} {} {} {} {}",
        l.boot_start, l.boot_len, l.bios_start, l.bios_len, l.root_start, l.root_len
    );
    Ok(())
}
