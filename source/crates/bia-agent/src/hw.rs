//! Machine identity from DMI / device-tree, PCI, DRM, USB — no extra userspace database.
use std::fs;
use std::path::Path;

fn read_trim(p: impl AsRef<Path>) -> String {
    fs::read_to_string(p)
        .map(|s| s.trim().trim_end_matches('\0').trim().to_string())
        .unwrap_or_default()
}

fn junk(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    l.is_empty()
        || l == "none"
        || l == "not specified"
        || l == "not available"
        || l == "default string"
        || l == "to be filled by o.e.m."
        || l.starts_with("to be filled")
        || l == "system product name"
        || l == "system version"
        || l == "system manufacturer"
}

fn clean(s: &str) -> Option<String> {
    let s = s.trim();
    if junk(s) {
        None
    } else {
        Some(s.to_string())
    }
}

fn hex_u32(s: &str) -> u32 {
    let t = s.trim().trim_start_matches("0x");
    u32::from_str_radix(t, 16).unwrap_or(0)
}

fn join_nonempty(parts: &[String], sep: &str) -> String {
    parts
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(sep)
}

fn pci_vendor(id: u16) -> Option<&'static str> {
    Some(match id {
        0x8086 | 0x8087 => "Intel",
        0x10de => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x1af4 => "Virtio",
        0x1b36 | 0x1234 => "QEMU",
        0x15ad => "VMware",
        0x14e4 => "Broadcom",
        0x10ec => "Realtek",
        0x168c | 0x17cb => "Qualcomm",
        0x1969 => "Atheros",
        0x14c3 => "MediaTek",
        0x10b5 => "PLX",
        0x1b21 => "ASMedia",
        0x1b4b | 0x11ab => "Marvell",
        0x15b3 => "Mellanox",
        0x144d => "Samsung",
        0x1d6a => "Fungible",
        0x1cc1 => "ADATA",
        0x1987 => "Phison",
        0x1e0f => "KIOXIA",
        0xc0a9 => "Micron",
        0x1179 => "Toshiba",
        0x2646 => "Kingston",
        0x1d0f => "Amazon",
        0x1ae0 => "Google",
        0x1414 => "Microsoft",
        0x106b => "Apple",
        0x1912 => "Renesas",
        0x197b => "JMicron",
        0x1a03 => "ASPEED",
        0x15d9 => "Supermicro",
        _ => return None,
    })
}

fn pci_class_name(class: u32) -> &'static str {
    match class >> 16 {
        0x01 => "storage",
        0x02 => "network",
        0x03 => match (class >> 8) & 0xff {
            0x00 => "vga",
            0x02 => "3d",
            _ => "display",
        },
        0x04 => "multimedia",
        0x06 => "bridge",
        0x07 => "comm",
        0x08 => "system",
        0x09 => "input",
        0x0c => match (class >> 8) & 0xff {
            0x03 => "usb",
            0x05 => "smbus",
            _ => "serial",
        },
        0x0d => "wireless",
        0x12 => "processing",
        _ => "other",
    }
}

struct Pci {
    slot: String,
    vendor: u16,
    device: u16,
    class: u32,
    driver: String,
}

fn pci_list() -> Vec<Pci> {
    let Ok(rd) = fs::read_dir("/sys/bus/pci/devices") else {
        return vec![];
    };
    let mut out = vec![];
    for e in rd.flatten() {
        let p = e.path();
        let slot = e.file_name().to_string_lossy().to_string();
        let vendor = hex_u32(&read_trim(p.join("vendor"))) as u16;
        let device = hex_u32(&read_trim(p.join("device"))) as u16;
        let class = hex_u32(&read_trim(p.join("class")));
        let mut driver = String::new();
        if let Ok(t) = fs::read_link(p.join("driver")) {
            driver = t
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        out.push(Pci {
            slot,
            vendor,
            device,
            class,
            driver,
        });
    }
    out.sort_by(|a, b| a.slot.cmp(&b.slot));
    out
}

pub fn chassis() -> String {
    let t = read_trim("/sys/class/dmi/id/chassis_type");
    match t.parse::<u32>().unwrap_or(0) {
        3..=7 => "desktop",
        8 | 9 | 10 | 14 => "laptop",
        11 => "handheld",
        13 => "all-in-one",
        17 | 23 => "server",
        30 => "tablet",
        31 | 32 => "convertible",
        _ => "",
    }
    .into()
}

pub fn machine_name() -> String {
    let vendor = clean(&read_trim("/sys/class/dmi/id/sys_vendor")).unwrap_or_default();
    let product = clean(&read_trim("/sys/class/dmi/id/product_name")).unwrap_or_default();
    let version = clean(&read_trim("/sys/class/dmi/id/product_version")).unwrap_or_default();
    let sku = clean(&read_trim("/sys/class/dmi/id/product_sku")).unwrap_or_default();
    let mut name = join_nonempty(&[vendor, product], " ");
    if !version.is_empty() && !name.contains(&version) {
        name = join_nonempty(&[name, version], " ");
    }
    if !sku.is_empty() && !name.contains(&sku) {
        name = join_nonempty(&[name, format!("sku {sku}")], " ");
    }
    if name.is_empty() {
        if let Some(dt) = clean(&read_trim("/sys/firmware/devicetree/base/model")) {
            name = dt;
        }
    }
    if name.is_empty() {
        name = "unknown".into();
    }
    let ch = chassis();
    if !ch.is_empty() && !name.to_ascii_lowercase().contains(&ch) {
        format!("{name} ({ch})")
    } else {
        name
    }
}

pub fn cpu_model() -> String {
    let s = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    for key in ["model name", "Hardware", "cpu model"] {
        for line in s.lines() {
            if let Some((k, v)) = line.split_once(':') {
                if k.trim().eq_ignore_ascii_case(key) {
                    let v = v.trim();
                    if !v.is_empty() {
                        return v.to_string();
                    }
                }
            }
        }
    }
    format!("{} cpu", crate::sys::ncpu())
}

fn gpus(pci: &[Pci]) -> Vec<String> {
    pci.iter()
        .filter(|p| p.class >> 16 == 0x03)
        .map(|p| {
            let vend = pci_vendor(p.vendor).unwrap_or("GPU");
            let kind = pci_class_name(p.class);
            let drv = if p.driver.is_empty() {
                "no kernel driver".into()
            } else {
                p.driver.clone()
            };
            format!("{vend} {kind} {:04x}:{:04x} ({drv})", p.vendor, p.device)
        })
        .collect()
}

fn displays() -> Vec<String> {
    let Ok(rd) = fs::read_dir("/sys/class/drm") else {
        return vec![];
    };
    let mut out = vec![];
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.contains('-') || name.starts_with("version") {
            continue;
        }
        let st = read_trim(e.path().join("status"));
        if st.is_empty() {
            continue;
        }
        out.push(format!("{name} {st}"));
    }
    out.sort();
    out
}

/// One line for the boot system message and `/status`.
pub fn summary() -> String {
    let pci = pci_list();
    let g = gpus(&pci);
    let d = displays();
    let gpu = if g.is_empty() {
        "no GPU".into()
    } else {
        g.join("; ")
    };
    let disp = if d.is_empty() {
        String::new()
    } else {
        let connected: Vec<_> = d
            .iter()
            .filter(|s| s.ends_with(" connected"))
            .cloned()
            .collect();
        if connected.is_empty() {
            format!("; displays: {}", d.join(", "))
        } else {
            format!("; displays: {}", connected.join(", "))
        }
    };
    format!(
        "machine: {}; CPU: {}× {}; GPU: {}{disp}",
        machine_name(),
        crate::sys::ncpu(),
        cpu_model(),
        gpu
    )
}

pub struct DisplayGpu {
    pub vendor: u16,
}

/// QEMU, virtio, VMware and VirtualBox displays are not compute devices.
pub fn virtual_gpu(vendor: u16) -> bool {
    matches!(vendor, 0x1234 | 0x1af4 | 0x1b36 | 0x15ad | 0x80ee)
}

pub fn display_gpus() -> Vec<DisplayGpu> {
    pci_list()
        .into_iter()
        .filter(|p| p.class >> 16 == 0x03)
        .map(|p| DisplayGpu { vendor: p.vendor })
        .collect()
}
