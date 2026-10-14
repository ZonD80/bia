//! Kept .deb files in /home/bia/files/debs: custom packages that no repository
//! provides (dpkg -i, apt install ./file.deb). The bia-keep-debs dpkg hook links them
//! there; a fold prunes the store and fills gaps, so unfold can reinstall them.
//! Repository packages are never kept: unfold installs those by name.
use bia_mcp::Ctx;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const STORE_REL: &str = "home/bia/files/debs";

#[derive(Clone, Debug, PartialEq)]
pub struct Deb {
    pub name: String,
    pub version: String,
    pub arch: String,
    pub path: PathBuf,
}

/// Debian archive file name: `name_version_arch.deb`, epoch colon written as %3a.
pub fn file_name(name: &str, version: &str, arch: &str) -> String {
    format!("{name}_{}_{arch}.deb", version.replace(':', "%3a"))
}

/// Inverse of `file_name`. Package names, versions and architectures never contain `_`.
pub fn parse_name(file: &str) -> Option<(String, String, String)> {
    let stem = file.strip_suffix(".deb")?;
    let mut it = stem.split('_');
    let (name, ver, arch) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() || name.is_empty() || ver.is_empty() || arch.is_empty() {
        return None;
    }
    Some((name.into(), ver.replace("%3a", ":").replace("%3A", ":"), arch.into()))
}

/// (name, version, arch) of every package in state `install ok installed`.
pub fn installed(status: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let (mut name, mut ver, mut arch, mut ok) = (String::new(), String::new(), String::new(), false);
    for line in status.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if ok && !name.is_empty() {
                out.push((name.clone(), ver.clone(), arch.clone()));
            }
            name.clear();
            ver.clear();
            arch.clear();
            ok = false;
        } else if let Some(v) = line.strip_prefix("Package: ") {
            name = v.trim().into();
        } else if let Some(v) = line.strip_prefix("Version: ") {
            ver = v.trim().into();
        } else if let Some(v) = line.strip_prefix("Architecture: ") {
            arch = v.trim().into();
        } else if let Some(v) = line.strip_prefix("Status: ") {
            ok = v.trim() == "install ok installed";
        }
    }
    out
}

pub fn installed_at(base: &Path) -> Vec<(String, String, String)> {
    installed(&fs::read_to_string(base.join("var/lib/dpkg/status")).unwrap_or_default())
}

pub fn list(dir: &Path) -> Vec<Deb> {
    let Ok(rd) = fs::read_dir(dir) else {
        return vec![];
    };
    let mut out: Vec<Deb> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let (name, version, arch) = parse_name(&e.file_name().to_string_lossy())?;
            Some(Deb { name, version, arch, path: e.path() })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn link_or_copy(from: &Path, to: &Path) -> bool {
    fs::hard_link(from, to).is_ok() || fs::copy(from, to).is_ok()
}

/// (Package, Version, Architecture) of a .deb file.
fn deb_fields(path: &Path) -> Option<(String, String, String)> {
    let out = Command::new("dpkg-deb")
        .arg("-f")
        .arg(path)
        .args(["Package", "Version", "Architecture"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let field = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string()))
            .filter(|v| !v.is_empty())
    };
    Some((field("Package:")?, field("Version:")?, field("Architecture:")?))
}

/// .deb files under `roots` (a few levels deep), skipping `skip`.
fn find_debs(roots: &[PathBuf], skip: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, u32)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        if dir == skip {
            continue;
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() && depth < 4 {
                stack.push((p, depth + 1));
            } else if ft.is_file() && p.extension().is_some_and(|x| x == "deb") {
                out.push(p);
            }
        }
    }
    out
}

/// Before a fold of `base`: keep only custom debs. A kept deb goes when its version is
/// no longer installed, or when `from_repo` (package name → repository of the installed
/// version) shows a repository provides it. `custom` lists installed packages that no
/// repository provides, (name, version, arch); a missing deb for one is looked up in
/// the user's files, /root and the temp directories (e.g. a downloaded installer).
pub fn refresh(
    ctx: &Ctx,
    base: &Path,
    custom: &[(String, String, String)],
    from_repo: &std::collections::HashMap<String, String>,
) {
    let store = base.join(STORE_REL);
    let inst = installed_at(base);
    let mut kept = 0;
    let mut dropped = 0;
    for d in list(&store) {
        let installed = inst
            .iter()
            .any(|(n, v, a)| *n == d.name && *v == d.version && *a == d.arch);
        if installed && !from_repo.contains_key(&d.name) {
            kept += 1;
        } else if fs::remove_file(&d.path).is_ok() {
            dropped += 1;
        }
    }
    let have = list(&store);
    let mut missing: Vec<&(String, String, String)> = custom
        .iter()
        .filter(|(n, v, a)| !v.is_empty() && !have.iter().any(|d| d.name == *n && d.version == *v && d.arch == *a))
        .collect();
    let mut added = 0;
    if !missing.is_empty() {
        let roots: Vec<PathBuf> = ["home/bia/files", "root", "var/tmp", "tmp"]
            .iter()
            .map(|r| base.join(r))
            .collect();
        for path in find_debs(&roots, &store) {
            if missing.is_empty() {
                break;
            }
            let Some((n, v, a)) = deb_fields(&path) else { continue };
            let Some(i) = missing.iter().position(|m| m.0 == n && m.1 == v && m.2 == a) else {
                continue;
            };
            if fs::create_dir_all(&store).is_ok() && link_or_copy(&path, &store.join(file_name(&n, &v, &a))) {
                missing.remove(i);
                added += 1;
            }
        }
    }
    let absent: Vec<String> = missing.iter().map(|(n, v, _)| format!("{n} {v}")).collect();
    ctx.log(format!(
        "fold: custom debs kept {kept}, removed {dropped} (gone or in a repository), added {added}{}",
        if absent.is_empty() {
            String::new()
        } else {
            format!("; no .deb found for {} (unfold reports them)", absent.join(", "))
        }
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        let f = file_name("google-chrome-stable", "1:155.0.8059.39-1", "amd64");
        assert_eq!(f, "google-chrome-stable_1%3a155.0.8059.39-1_amd64.deb");
        assert_eq!(
            parse_name(&f),
            Some(("google-chrome-stable".into(), "1:155.0.8059.39-1".into(), "amd64".into()))
        );
        assert_eq!(parse_name("jq_1.6-2.1+deb12u2_arm64.deb").unwrap().1, "1.6-2.1+deb12u2");
        assert_eq!(parse_name("bad.deb"), None);
        assert_eq!(parse_name("a_b_c_d.deb"), None);
        assert_eq!(parse_name("jq_1_arm64.txt"), None);
    }

    #[test]
    fn installed_only_counts_installed() {
        let st = "Package: jq\nStatus: install ok installed\nArchitecture: arm64\nVersion: 1.6\n\nPackage: gone\nStatus: deinstall ok config-files\nArchitecture: arm64\nVersion: 2\n\nPackage: tzdata\nStatus: install ok installed\nArchitecture: all\nVersion: 2024a\n";
        assert_eq!(
            installed(st),
            vec![
                ("jq".into(), "1.6".into(), "arm64".into()),
                ("tzdata".into(), "2024a".into(), "all".into())
            ]
        );
    }

    #[test]
    fn refresh_keeps_only_custom_and_finds_downloads() {
        let base = std::env::temp_dir().join(format!("bia-debs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("var/lib/dpkg")).unwrap();
        fs::create_dir_all(base.join(STORE_REL)).unwrap();
        fs::write(
            base.join("var/lib/dpkg/status"),
            "Package: jq\nStatus: install ok installed\nArchitecture: arm64\nVersion: 1.6\n\nPackage: mytool\nStatus: install ok installed\nArchitecture: all\nVersion: 2.1\n\nPackage: vendor\nStatus: install ok installed\nArchitecture: all\nVersion: 3\n",
        )
        .unwrap();
        let store = base.join(STORE_REL);
        fs::write(store.join("vendor_3_all.deb"), b"v").unwrap();
        fs::write(store.join("vendor_2_all.deb"), b"old").unwrap();
        fs::write(store.join("jq_1.6_arm64.deb"), b"jq").unwrap(); // now in a repository
        // a downloaded installer with a non-canonical name, found by its control fields
        let pkg = base.join("pkg");
        fs::create_dir_all(pkg.join("DEBIAN")).unwrap();
        fs::write(pkg.join("DEBIAN/control"), "Package: mytool\nVersion: 2.1\nArchitecture: all\nMaintainer: t <t@t>\nDescription: t\n").unwrap();
        fs::create_dir_all(base.join("home/bia/files/inbox/web")).unwrap();
        let built = Command::new("dpkg-deb")
            .args(["--build", "--root-owner-group"])
            .arg(&pkg)
            .arg(base.join("home/bia/files/inbox/web/mytool-latest.deb"))
            .output();
        let have_dpkg = built.map(|o| o.status.success()).unwrap_or(false);
        let custom = vec![
            ("mytool".to_string(), "2.1".to_string(), "all".to_string()),
            ("vendor".to_string(), "3".to_string(), "all".to_string()),
        ];
        let repo: std::collections::HashMap<String, String> =
            [("jq".to_string(), "http://deb.debian.org/debian bookworm/main".to_string())].into();
        refresh(&Ctx::console(), &base, &custom, &repo);
        let names: Vec<String> = list(&store)
            .iter()
            .map(|d| d.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        if have_dpkg {
            assert_eq!(names, ["mytool_2.1_all.deb", "vendor_3_all.deb"]);
        } else {
            assert_eq!(names, ["vendor_3_all.deb"]);
        }
        let _ = fs::remove_dir_all(&base);
    }
}
