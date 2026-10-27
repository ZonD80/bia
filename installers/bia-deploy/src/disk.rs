#[cfg(not(target_os = "linux"))]
use anyhow::bail;
use anyhow::{Context, Result};
use std::path::PathBuf;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::process::Command;

#[derive(Debug, Clone)]
pub struct DiskInfo {
    /// Raw device path suitable for writing (e.g. `/dev/rdisk4`, `/dev/sdb`, `\\.\PhysicalDrive2`).
    pub path: PathBuf,
    /// Human label shown in UIs.
    pub name: String,
    pub size_bytes: u64,
    pub removable: bool,
}

/// List candidate disks. Prefer removable/USB; include all whole disks when `all` is true.
pub fn list_removable(all: bool) -> Result<Vec<DiskInfo>> {
    #[cfg(target_os = "macos")]
    {
        list_macos(all)
    }
    #[cfg(target_os = "linux")]
    {
        list_linux(all)
    }
    #[cfg(target_os = "windows")]
    {
        list_windows(all)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = all;
        bail!("disk listing is not supported on this OS");
    }
}

#[cfg(target_os = "macos")]
fn list_macos(all: bool) -> Result<Vec<DiskInfo>> {
    let out = Command::new("diskutil")
        .args(["list", "-plist"])
        .output()
        .context("diskutil list")?;
    if !out.status.success() {
        bail!(
            "diskutil list failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // Prefer plutil → JSON so we do not depend on a plist crate.
    let json = Command::new("plutil")
        .args(["-convert", "json", "-o", "-", "--", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&out.stdout)?;
            }
            child.wait_with_output()
        })
        .context("plutil convert")?;
    if !json.status.success() {
        bail!("plutil failed: {}", String::from_utf8_lossy(&json.stderr));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&json.stdout).context("parse diskutil json")?;
    let mut disks = Vec::new();
    let Some(arr) = v.get("AllDisksAndPartitions").and_then(|a| a.as_array()) else {
        return Ok(disks);
    };
    for d in arr {
        let Some(id) = d.get("DeviceIdentifier").and_then(|s| s.as_str()) else {
            continue;
        };
        // Whole disks only (diskN), not partitions (diskNsM).
        if !id
            .strip_prefix("disk")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let size = d
            .get("Size")
            .and_then(|s| s.as_u64())
            .or_else(|| {
                // Fallback: diskutil info
                diskutil_size(id).ok()
            })
            .unwrap_or(0);
        let info = diskutil_info(id)?;
        // Disk images and APFS/CoreStorage synthesized disks are never flash targets.
        if info.3 {
            continue;
        }
        if !all && !info.1 {
            continue;
        }
        // Prefer raw character device for speed.
        let path = PathBuf::from(format!("/dev/r{id}"));
        disks.push(DiskInfo {
            path,
            name: info.0,
            size_bytes: if size > 0 { size } else { info.2 },
            removable: info.1,
        });
    }
    Ok(disks)
}

#[cfg(target_os = "macos")]
fn diskutil_size(id: &str) -> Result<u64> {
    let out = Command::new("diskutil")
        .args(["info", "-plist", id])
        .output()?;
    let json = Command::new("plutil")
        .args(["-convert", "json", "-o", "-", "--", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&out.stdout)?;
            }
            child.wait_with_output()
        })?;
    let v: serde_json::Value = serde_json::from_slice(&json.stdout)?;
    Ok(v.get("TotalSize").and_then(|s| s.as_u64()).unwrap_or(0))
}

#[cfg(target_os = "macos")]
fn diskutil_info(id: &str) -> Result<(String, bool, u64, bool)> {
    let out = Command::new("diskutil")
        .args(["info", "-plist", id])
        .output()
        .context("diskutil info")?;
    let json = Command::new("plutil")
        .args(["-convert", "json", "-o", "-", "--", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&out.stdout)?;
            }
            child.wait_with_output()
        })?;
    let v: serde_json::Value = serde_json::from_slice(&json.stdout)?;
    let name = v
        .get("MediaName")
        .or_else(|| v.get("VolumeName"))
        .and_then(|s| s.as_str())
        .unwrap_or(id)
        .to_string();
    let removable = v
        .get("Removable")
        .or_else(|| v.get("RemovableMedia"))
        .and_then(|b| b.as_bool())
        .unwrap_or(false)
        || v.get("Protocol")
            .and_then(|s| s.as_str())
            .map(|p| p.eq_ignore_ascii_case("USB"))
            .unwrap_or(false);
    let size = v.get("TotalSize").and_then(|s| s.as_u64()).unwrap_or(0);
    let virtual_disk = v
        .get("VirtualOrPhysical")
        .and_then(|s| s.as_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("Virtual"));
    Ok((name, removable, size, virtual_disk))
}

#[cfg(target_os = "linux")]
fn list_linux(all: bool) -> Result<Vec<DiskInfo>> {
    let mut disks = Vec::new();
    let rd = std::fs::read_dir("/sys/block").context("read /sys/block")?;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("loop")
            || name.starts_with("ram")
            || name.starts_with("zram")
            || name.starts_with("dm-")
        {
            continue;
        }
        let sys = e.path();
        // Skip partitions appearing as block devices with a parent.
        if sys.join("partition").is_file() {
            continue;
        }
        let removable = std::fs::read_to_string(sys.join("removable"))
            .ok()
            .map(|s| s.trim() == "1")
            .unwrap_or(false);
        let tran = std::fs::read_to_string(sys.join("device/../../../transport"))
            .ok()
            .or_else(|| {
                // virtio / usb via uevent
                std::fs::read_to_string(sys.join("device/uevent")).ok()
            })
            .unwrap_or_default();
        let usb = removable
            || tran.to_lowercase().contains("usb")
            || std::fs::read_to_string(sys.join("device/../../removable"))
                .ok()
                .map(|s| s.trim() == "1")
                .unwrap_or(false);
        if !all && !usb {
            continue;
        }
        let sectors: u64 = std::fs::read_to_string(sys.join("size"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let size = sectors.saturating_mul(512);
        let model = std::fs::read_to_string(sys.join("device/model"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| name.clone());
        disks.push(DiskInfo {
            path: PathBuf::from(format!("/dev/{name}")),
            name: model,
            size_bytes: size,
            removable: usb,
        });
    }
    disks.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(disks)
}

#[cfg(target_os = "windows")]
fn list_windows(all: bool) -> Result<Vec<DiskInfo>> {
    // PowerShell is always available; avoids a heavy windows SDK surface for listing.
    let script = r#"
Get-CimInstance Win32_DiskDrive | ForEach-Object {
  $rem = $_.MediaType -match 'Removable' -or $_.InterfaceType -eq 'USB'
  [PSCustomObject]@{
    Index = $_.Index
    Model = $_.Model
    Size = [uint64]$_.Size
    Removable = [bool]$rem
  }
} | ConvertTo-Json -Compress
"#;
    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .output()
        .context("powershell disk list")?;
    if !out.status.success() {
        bail!(
            "powershell failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    if text.is_empty() {
        return Ok(vec![]);
    }
    let v: serde_json::Value = serde_json::from_str(text).context("parse disk json")?;
    let arr = if v.is_array() {
        v.as_array().cloned().unwrap_or_default()
    } else {
        vec![v]
    };
    let mut disks = Vec::new();
    for d in arr {
        let rem = d.get("Removable").and_then(|b| b.as_bool()).unwrap_or(false);
        if !all && !rem {
            continue;
        }
        let idx = d.get("Index").and_then(|i| i.as_u64()).unwrap_or(0);
        let model = d
            .get("Model")
            .and_then(|s| s.as_str())
            .unwrap_or("disk")
            .to_string();
        let size = d.get("Size").and_then(|s| s.as_u64()).unwrap_or(0);
        disks.push(DiskInfo {
            path: PathBuf::from(format!(r"\\.\PhysicalDrive{idx}")),
            name: model,
            size_bytes: size,
            removable: rem,
        });
    }
    Ok(disks)
}
