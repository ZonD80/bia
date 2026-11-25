//! Docker in a carry archive. `/var` is never carried (it must match the image), and Docker
//! keeps everything in `/var/lib/docker`, so without this an update, a restore or a flight
//! loses every container. Fold exports what is needed to bring the containers back as they
//! were, into `docker/` next to `manifest.json`:
//!
//! - `docker.json`: `docker inspect` of every container, the user-defined networks and the
//!   volumes they use, the image each container runs on, and which ones were running;
//! - `images.tar`: `docker save` of those images (tags kept, so `docker compose` finds them);
//! - `volumes/<name>.tar`: each volume's data (numeric owners, xattrs), taken while the
//!   running containers are paused, so a database in it is crash-consistent.
//!
//! Unfold (after the packages and the home are back, so Docker is installed again and bind
//! mounts find their folders) starts Docker, loads the images, makes the networks and
//! volumes, puts the volume data back, creates each container from its own inspect
//! (Config + HostConfig + networks: the Engine API takes them as they were, labels too, so
//! compose still owns its containers) and starts those that were running. What already
//! exists under the same name is left alone. If Docker cannot be brought back, the export
//! is kept under `files/carry/docker-<time>/` for a later `--docker-import`.
use bia_mcp::Ctx;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub const DIR: &str = "docker";
const SOCK: &str = "/var/run/docker.sock";
const BUILTIN_NETS: &[&str] = &["bridge", "host", "none", "default"];

/// What an export holds, for the log and the space check.
pub struct Exported {
    pub containers: usize,
    pub images: usize,
    pub volumes: usize,
    pub bytes: u64,
}

fn docker_present() -> bool {
    Path::new(SOCK).exists()
        && std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d.join("docker").is_file()))
            .unwrap_or(false)
}

fn docker(args: &[&str]) -> Result<String, String> {
    let out = Command::new("docker")
        .args(args)
        .output()
        .map_err(|e| format!("docker: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "docker {}: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn docker_json(args: &[&str]) -> Result<Vec<Value>, String> {
    let text = docker(args)?;
    serde_json::from_str::<Vec<Value>>(&text).map_err(|e| format!("docker {}: {e}", args.join(" ")))
}

fn lines(s: &str) -> Vec<String> {
    s.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// A volume docker named itself (64 hex digits): kept only while a container uses it.
fn anonymous(name: &str) -> bool {
    name.len() == 64 && name.chars().all(|c| c.is_ascii_hexdigit())
}

/// The container's name without docker's leading slash.
fn cname(c: &Value) -> String {
    c["Name"].as_str().unwrap_or("").trim_start_matches('/').to_string()
}

/// Networks a container is attached to that must be made again (not the built-in ones).
fn own_networks(c: &Value) -> Vec<String> {
    c["NetworkSettings"]["Networks"]
        .as_object()
        .map(|m| m.keys().filter(|k| !BUILTIN_NETS.contains(&k.as_str())).cloned().collect())
        .unwrap_or_default()
}

/// Named volumes a container mounts.
fn used_volumes(c: &Value) -> Vec<String> {
    c["Mounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["Type"] == "volume")
        .filter_map(|m| m["Name"].as_str().map(str::to_string))
        .collect()
}

/// A volume whose data lives elsewhere (local driver bound to a host folder, NFS…): only its
/// definition is carried; the folder comes back with the home, or not at all.
fn data_elsewhere(v: &Value) -> bool {
    v["Driver"].as_str().unwrap_or("local") != "local"
        || v["Options"].as_object().is_some_and(|o| o.contains_key("device"))
}

fn dir_bytes(p: &Path) -> u64 {
    let mut n = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(m) = fs::symlink_metadata(e.path()) else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else if m.is_file() {
                n += m.len();
            }
        }
    }
    n
}

/// Everything docker would export: the containers' inspects, their images, networks and
/// volumes. None when Docker is not here or has no containers.
struct Plan {
    containers: Vec<Value>,
    images: Vec<String>,
    networks: Vec<Value>,
    volumes: Vec<Value>,
    estimate: u64,
}

fn plan() -> Result<Option<Plan>, String> {
    if !docker_present() {
        return Ok(None);
    }
    let ids = lines(&docker(&["ps", "-aq", "--no-trunc"])?);
    if ids.is_empty() {
        return Ok(None);
    }
    let mut args = vec!["inspect", "--type", "container"];
    args.extend(ids.iter().map(String::as_str));
    let containers = docker_json(&args)?;
    // the image each container runs on: its tag when the tag still points at that image
    // (so compose and `docker run` find it by name), else the image id
    let mut images: Vec<String> = vec![];
    let mut estimate = 0u64;
    let mut seen = HashSet::new();
    for c in &containers {
        let id = c["Image"].as_str().unwrap_or("").to_string();
        let tag = c["Config"]["Image"].as_str().unwrap_or("").to_string();
        let r = match docker_json(&["image", "inspect", &tag]) {
            Ok(v) if !tag.is_empty() && v.first().and_then(|i| i["Id"].as_str()) == Some(id.as_str()) => tag,
            _ => id.clone(),
        };
        if !r.is_empty() && seen.insert(r.clone()) {
            if let Ok(v) = docker_json(&["image", "inspect", &r]) {
                estimate += v.first().and_then(|i| i["Size"].as_u64()).unwrap_or(0);
            }
            images.push(r);
        }
    }
    let mut nets: Vec<String> = containers.iter().flat_map(own_networks).collect();
    nets.sort();
    nets.dedup();
    let networks = if nets.is_empty() {
        vec![]
    } else {
        let mut a = vec!["network", "inspect"];
        a.extend(nets.iter().map(String::as_str));
        docker_json(&a)?
    };
    let mut vols: Vec<String> = containers.iter().flat_map(used_volumes).collect();
    vols.extend(lines(&docker(&["volume", "ls", "-q"])?).into_iter().filter(|n| !anonymous(n)));
    vols.sort();
    vols.dedup();
    let volumes = if vols.is_empty() {
        vec![]
    } else {
        let mut a = vec!["volume", "inspect"];
        a.extend(vols.iter().map(String::as_str));
        docker_json(&a)?
    };
    for v in volumes.iter().filter(|v| !data_elsewhere(v)) {
        if let Some(mp) = v["Mountpoint"].as_str() {
            estimate += dir_bytes(Path::new(mp));
        }
    }
    Ok(Some(Plan { containers, images, networks, volumes, estimate }))
}

/// Bytes an export would take before compression (images + volume data); 0 without Docker.
pub fn estimate() -> u64 {
    plan().ok().flatten().map_or(0, |p| p.estimate)
}

/// Export Docker into `dir` (created). Ok(None) when there is nothing to carry.
pub fn export(ctx: &Ctx, dir: &Path) -> Result<Option<Exported>, String> {
    let Some(p) = plan()? else { return Ok(None) };
    fs::create_dir_all(dir.join("volumes")).map_err(|e| format!("{}: {e}", dir.display()))?;
    ctx.log(format!(
        "fold: docker: {} containers, {} images, {} volumes, {} networks",
        p.containers.len(),
        p.images.len(),
        p.volumes.len(),
        p.networks.len()
    ));
    if !p.images.is_empty() {
        ctx.log(format!("fold: docker: saving images {}", p.images.join(", ")));
        let out = dir.join("images.tar");
        let o = out.to_string_lossy().to_string();
        let mut a = vec!["save", "-o", o.as_str()];
        a.extend(p.images.iter().map(String::as_str));
        docker(&a)?;
    }
    // running containers are paused while their volumes are read: the data on disk is then
    // what a crash at that moment would leave, which databases recover from
    let running: Vec<String> = p
        .containers
        .iter()
        .filter(|c| c["State"]["Running"] == true)
        .map(cname)
        .collect();
    let to_pause: Vec<String> = p
        .containers
        .iter()
        .filter(|c| c["State"]["Running"] == true && c["State"]["Paused"] != true)
        .map(cname)
        .collect();
    let data: Vec<&Value> = p.volumes.iter().filter(|v| !data_elsewhere(v)).collect();
    let mut paused = vec![];
    if !data.is_empty() {
        for n in &to_pause {
            if docker(&["pause", n]).is_ok() {
                paused.push(n.clone());
            }
        }
    }
    let mut res = Ok(());
    for v in &data {
        let name = v["Name"].as_str().unwrap_or("");
        let mp = v["Mountpoint"].as_str().unwrap_or("");
        if name.is_empty() || name.contains('/') || !Path::new(mp).is_dir() {
            continue;
        }
        ctx.log(format!("fold: docker: volume {name}"));
        let st = Command::new("tar")
            .args(["--numeric-owner", "--xattrs", "--xattrs-include=*", "--acls", "-C", mp, "-cf"])
            .arg(dir.join("volumes").join(format!("{name}.tar")))
            .arg(".")
            .status();
        match st {
            Ok(s) if s.success() || s.code() == Some(1) => {}
            Ok(s) => {
                res = Err(format!("docker volume {name}: tar exited {s}"));
                break;
            }
            Err(e) => {
                res = Err(format!("docker volume {name}: tar: {e}"));
                break;
            }
        }
    }
    for n in &paused {
        let _ = docker(&["unpause", n]);
    }
    res?;
    let doc = json!({
        "format": "bia-docker",
        "containers": p.containers,
        "networks": p.networks,
        "volumes": p.volumes,
        "images": p.images,
        "running": running,
    });
    fs::write(dir.join("docker.json"), serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?)
        .map_err(|e| format!("docker.json: {e}"))?;
    Ok(Some(Exported {
        containers: p.containers.len(),
        images: p.images.len(),
        volumes: p.volumes.len(),
        bytes: dir_bytes(dir),
    }))
}

// ---------------------------------------------------------------- import

/// One request to the Docker Engine API on its socket; (status, body).
fn api(method: &str, path: &str, body: Option<&Value>) -> Result<(u16, String), String> {
    use std::os::unix::net::UnixStream;
    let mut s = UnixStream::connect(SOCK).map_err(|e| format!("{SOCK}: {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(120)));
    let b = body.map(|v| v.to_string()).unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: docker\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}",
        b.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    s.read_to_end(&mut out).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| format!("docker API: no answer to {method} {path}"))?;
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    Ok((status, body))
}

/// The Engine API's error text, or the body as it came.
fn api_err(body: &str) -> String {
    let body = body.trim();
    // a chunked body: its JSON sits between the size lines
    let j = match (body.find('{'), body.rfind('}')) {
        (Some(a), Some(b)) if a < b => &body[a..=b],
        _ => body,
    };
    serde_json::from_str::<Value>(j)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.chars().take(200).collect())
}

/// `POST /networks/create` from a network's inspect.
pub fn network_body(n: &Value) -> Value {
    let mut ipam = json!({"Driver": n["IPAM"]["Driver"].as_str().unwrap_or("default")});
    if let Some(c) = n["IPAM"]["Config"].as_array().filter(|c| !c.is_empty()) {
        ipam["Config"] = json!(c);
    }
    if let Some(o) = n["IPAM"]["Options"].as_object().filter(|o| !o.is_empty()) {
        ipam["Options"] = json!(o);
    }
    json!({
        "Name": n["Name"],
        "Driver": n["Driver"].as_str().unwrap_or("bridge"),
        "Internal": n["Internal"].as_bool().unwrap_or(false),
        "Attachable": n["Attachable"].as_bool().unwrap_or(false),
        "EnableIPv6": n["EnableIPv6"].as_bool().unwrap_or(false),
        "IPAM": ipam,
        "Options": n["Options"].as_object().cloned().unwrap_or_default(),
        "Labels": n["Labels"].as_object().cloned().unwrap_or_default(),
    })
}

/// `POST /containers/create` from a container's inspect: its own Config and HostConfig, and
/// for each network what the owner chose (aliases, fixed addresses, links), not what docker
/// assigned. `image` is what the container should run on here.
pub fn container_body(c: &Value, image: &str) -> Value {
    let mut body = c["Config"].clone();
    if !body.is_object() {
        body = json!({});
    }
    body["Image"] = json!(image);
    body["HostConfig"] = c["HostConfig"].clone();
    // a hostname docker made up (the old id) would only be wrong now; the new id gives one
    let id = c["Id"].as_str().unwrap_or("");
    if id.len() >= 12 && body["Hostname"].as_str() == Some(&id[..12]) {
        body.as_object_mut().map(|o| o.remove("Hostname"));
    }
    // sharing another container's network: docker refuses what belongs to that one
    if c["HostConfig"]["NetworkMode"].as_str().is_some_and(|m| m.starts_with("container:")) {
        if let Some(o) = body.as_object_mut() {
            for k in ["Hostname", "Domainname", "ExposedPorts", "MacAddress"] {
                o.remove(k);
            }
        }
        if let Some(h) = body["HostConfig"].as_object_mut() {
            for k in ["Dns", "DnsOptions", "DnsSearch", "ExtraHosts", "PortBindings", "PublishAllPorts", "Links"] {
                h.remove(k);
            }
        }
    }
    let mut eps = serde_json::Map::new();
    for (name, ep) in c["NetworkSettings"]["Networks"].as_object().into_iter().flatten() {
        let mut e = serde_json::Map::new();
        for k in ["Aliases", "IPAMConfig", "Links", "DriverOpts", "MacAddress", "GwPriority"] {
            if let Some(v) = ep.get(k).filter(|v| !v.is_null()) {
                if k == "MacAddress" && v.as_str().is_none_or(str::is_empty) {
                    continue;
                }
                e.insert(k.to_string(), v.clone());
            }
        }
        eps.insert(name.clone(), Value::Object(e));
    }
    body["NetworkingConfig"] = json!({"EndpointsConfig": eps});
    body
}

/// Other containers this one needs at creation (`--network container:x`, `--volumes-from`,
/// links), by name: they are created first.
pub fn needs(c: &Value, by_id: &HashMap<String, String>) -> Vec<String> {
    let name_of = |r: &str| -> String {
        let r = r.split(':').next().unwrap_or(r).trim_start_matches('/');
        by_id.get(r).cloned().unwrap_or_else(|| r.to_string())
    };
    let hc = &c["HostConfig"];
    let mut out = vec![];
    if let Some(m) = hc["NetworkMode"].as_str().and_then(|m| m.strip_prefix("container:")) {
        out.push(name_of(m));
    }
    for v in hc["VolumesFrom"].as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
        out.push(name_of(v));
    }
    for l in hc["Links"].as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
        out.push(name_of(l));
    }
    out
}

/// `container:<old id>` in a HostConfig names a container by an id that is new here: use
/// its name instead.
fn rename_refs(body: &mut Value, by_id: &HashMap<String, String>) {
    let hc = &mut body["HostConfig"];
    if let Some(m) = hc["NetworkMode"].as_str().and_then(|m| m.strip_prefix("container:")) {
        if let Some(n) = by_id.get(m) {
            hc["NetworkMode"] = json!(format!("container:{n}"));
        }
    }
    if let Some(a) = hc["VolumesFrom"].as_array_mut() {
        for v in a.iter_mut() {
            if let Some(s) = v.as_str() {
                let (id, mode) = s.split_once(':').map(|(a, b)| (a, Some(b))).unwrap_or((s, None));
                if let Some(n) = by_id.get(id) {
                    *v = json!(match mode {
                        Some(m) => format!("{n}:{m}"),
                        None => n.clone(),
                    });
                }
            }
        }
    }
}

fn wait_for_docker(limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if Path::new(SOCK).exists() && docker(&["info", "--format", "{{.ServerVersion}}"]).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    false
}

/// Bring Docker back from an export in `dir`. Returns one line for the boot notice, or None
/// when the archive has no Docker part. On failure the export is kept in `keep` (moved).
pub fn import(ctx: &Ctx, dir: &Path, keep: &Path) -> Option<String> {
    let doc: Value = serde_json::from_slice(&fs::read(dir.join("docker.json")).ok()?).ok()?;
    if doc["format"] != "bia-docker" {
        return None;
    }
    match import_doc(ctx, dir, &doc) {
        Ok(line) => Some(line),
        Err(e) => {
            ctx.log(format!("unfold: docker: {e}"));
            let kept = fs::create_dir_all(keep.parent().unwrap_or(keep))
                .and_then(|_| fs::rename(dir, keep).or_else(|_| copy_dir(dir, keep)))
                .is_ok();
            Some(if kept {
                format!(" Docker did not come back ({e}). Its containers, images and volumes are kept in {}: once Docker works, run `sudo /home/bia/MCPs/carry/bin/$(uname -m)/bia-mcp-carry --docker-import {}`.", keep.display(), keep.display())
            } else {
                format!(" Docker did not come back ({e}), and its export could not be kept.")
            })
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    let st = Command::new("cp").arg("-a").arg(from).arg(to).status()?;
    if st.success() { Ok(()) } else { Err(std::io::Error::other("cp failed")) }
}

fn import_doc(ctx: &Ctx, dir: &Path, doc: &Value) -> Result<String, String> {
    let containers: Vec<Value> = doc["containers"].as_array().cloned().unwrap_or_default();
    if containers.is_empty() {
        return Ok(String::new());
    }
    if !std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join("docker").is_file()))
        .unwrap_or(false)
    {
        return Err("docker is not installed here".into());
    }
    ctx.log("unfold: docker: starting the Docker service");
    let _ = Command::new("systemctl")
        .args(["start", "docker"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if !wait_for_docker(Duration::from_secs(120)) {
        return Err("the Docker service did not start".into());
    }
    let mut problems: Vec<String> = vec![];
    let images = dir.join("images.tar");
    if images.is_file() {
        ctx.log("unfold: docker: loading images");
        if let Err(e) = docker(&["load", "-i", &images.to_string_lossy()]) {
            problems.push(e);
        }
    }
    let have_nets: HashSet<String> = lines(&docker(&["network", "ls", "--format", "{{.Name}}"])?).into_iter().collect();
    let mut nets = 0;
    for n in doc["networks"].as_array().into_iter().flatten() {
        let name = n["Name"].as_str().unwrap_or("");
        if name.is_empty() || have_nets.contains(name) {
            continue;
        }
        match api("POST", "/networks/create", Some(&network_body(n))) {
            Ok((s, _)) if s < 300 => nets += 1,
            Ok((_, b)) => problems.push(format!("network {name}: {}", api_err(&b))),
            Err(e) => problems.push(format!("network {name}: {e}")),
        }
    }
    let have_vols: HashSet<String> = lines(&docker(&["volume", "ls", "-q"])?).into_iter().collect();
    let mut vols = 0;
    for v in doc["volumes"].as_array().into_iter().flatten() {
        let name = v["Name"].as_str().unwrap_or("");
        if name.is_empty() || name.contains('/') || have_vols.contains(name) {
            continue;
        }
        let body = json!({
            "Name": name,
            "Driver": v["Driver"].as_str().unwrap_or("local"),
            "DriverOpts": v["Options"].as_object().cloned().unwrap_or_default(),
            "Labels": v["Labels"].as_object().cloned().unwrap_or_default(),
        });
        match api("POST", "/volumes/create", Some(&body)) {
            Ok((s, b)) if s < 300 => {
                vols += 1;
                let data = dir.join("volumes").join(format!("{name}.tar"));
                let mp = serde_json::from_str::<Value>(&b)
                    .ok()
                    .and_then(|v| v["Mountpoint"].as_str().map(str::to_string))
                    .or_else(|| docker(&["volume", "inspect", "-f", "{{.Mountpoint}}", name]).ok().map(|s| s.trim().to_string()));
                if let (true, Some(mp)) = (data.is_file(), mp) {
                    ctx.log(format!("unfold: docker: volume {name}"));
                    let st = Command::new("tar")
                        .args(["--numeric-owner", "--same-permissions", "--xattrs", "--xattrs-include=*", "--acls", "-C", &mp, "-xf"])
                        .arg(&data)
                        .status();
                    if !st.is_ok_and(|s| s.success()) {
                        problems.push(format!("volume {name}: its data did not unpack"));
                    }
                }
            }
            Ok((_, b)) => problems.push(format!("volume {name}: {}", api_err(&b))),
            Err(e) => problems.push(format!("volume {name}: {e}")),
        }
    }
    // containers: in an order that has what each needs (network of, volumes from, links) first
    let by_id: HashMap<String, String> = containers
        .iter()
        .map(|c| (c["Id"].as_str().unwrap_or("").to_string(), cname(c)))
        .collect();
    let mut by_short = by_id.clone();
    for (id, n) in &by_id {
        if id.len() >= 12 {
            by_short.insert(id[..12].to_string(), n.clone());
        }
    }
    let have: HashSet<String> = lines(&docker(&["ps", "-a", "--format", "{{.Names}}"])?).into_iter().collect();
    let mut todo: Vec<&Value> = containers.iter().filter(|c| !have.contains(&cname(c))).collect();
    let skipped = containers.len() - todo.len();
    let mut made: HashSet<String> = have.clone();
    let mut created = 0;
    // creation order has what each needs first: starting goes the same way
    let mut order: Vec<String> = vec![];
    let mut last_err: HashMap<String, String> = HashMap::new();
    loop {
        let before = todo.len();
        let mut rest = vec![];
        for c in todo {
            let name = cname(c);
            if needs(c, &by_short).iter().any(|n| !made.contains(n) && by_id.values().any(|x| x == n)) {
                rest.push(c);
                continue;
            }
            let id = c["Image"].as_str().unwrap_or("");
            let tag = c["Config"]["Image"].as_str().unwrap_or("");
            let image = if !tag.is_empty() && docker(&["image", "inspect", tag]).is_ok() { tag } else { id };
            let mut body = container_body(c, image);
            rename_refs(&mut body, &by_short);
            let path = format!("/containers/create?name={}", urlenc(&name));
            match api("POST", &path, Some(&body)) {
                Ok((s, _)) if s < 300 => {
                    created += 1;
                    order.push(name.clone());
                    made.insert(name);
                }
                Ok((_, b)) => {
                    last_err.insert(name, api_err(&b));
                    rest.push(c);
                }
                Err(e) => {
                    last_err.insert(name, e);
                    rest.push(c);
                }
            }
        }
        todo = rest;
        if todo.is_empty() || todo.len() == before {
            break;
        }
    }
    for c in &todo {
        let n = cname(c);
        problems.push(format!("container {n}: {}", last_err.get(&n).cloned().unwrap_or_else(|| "what it needs did not come back".into())));
    }
    let mut started = 0;
    let running: HashSet<&str> = doc["running"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
    for n in order.iter().filter(|n| running.contains(n.as_str())) {
        match docker(&["start", n]) {
            Ok(_) => started += 1,
            Err(e) => problems.push(format!("start {n}: {e}")),
        }
    }
    let mut line = format!(
        " Docker came back: {created} of {} containers ({started} started), {} images, {vols} volumes, {nets} networks.",
        containers.len(),
        doc["images"].as_array().map_or(0, |a| a.len())
    );
    if skipped > 0 {
        line.push_str(&format!(" {skipped} containers were already here under the same names and were left as they are."));
    }
    if !problems.is_empty() {
        line.push_str(&format!(" Docker problems: {}.", problems.join("; ")));
    }
    ctx.log(format!("unfold: docker:{line}"));
    Ok(line)
}

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Where a failed import keeps the export.
pub fn keep_dir(files: &Path) -> PathBuf {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    files.join("carry").join(format!("docker-{secs}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container() -> Value {
        json!({
            "Id": "a".repeat(64),
            "Name": "/market-app-1",
            "Image": "sha256:1111",
            "Config": {"Image": "biamarket:latest", "Env": ["A=1"], "Labels": {"com.docker.compose.project": "market"}, "Hostname": "aaaaaaaaaaaa"},
            "HostConfig": {"NetworkMode": "market_default", "RestartPolicy": {"Name": "unless-stopped"}, "Binds": ["/home/bia/files/x:/x"],
                           "VolumesFrom": [format!("{}:ro", "b".repeat(64))]},
            "NetworkSettings": {"Networks": {"market_default": {"Aliases": ["app"], "NetworkID": "n1", "EndpointID": "e1", "IPAddress": "172.18.0.2", "MacAddress": "", "IPAMConfig": null}}},
            "Mounts": [{"Type": "volume", "Name": "market_db"}, {"Type": "bind", "Source": "/home/bia/files/x"}],
            "State": {"Running": true, "Paused": false}
        })
    }

    #[test]
    fn a_container_is_created_from_its_own_inspect() {
        let c = container();
        let b = container_body(&c, "biamarket:latest");
        assert_eq!(b["Image"], "biamarket:latest");
        assert!(b.get("Hostname").is_none(), "docker's own hostname (the old id) goes");
        assert_eq!(b["Env"], json!(["A=1"]));
        assert_eq!(b["Labels"]["com.docker.compose.project"], "market");
        assert_eq!(b["HostConfig"]["RestartPolicy"]["Name"], "unless-stopped");
        // what the owner chose stays, what docker assigned goes
        assert_eq!(b["NetworkingConfig"]["EndpointsConfig"]["market_default"], json!({"Aliases": ["app"]}));
        assert_eq!(own_networks(&c), vec!["market_default".to_string()]);
        assert_eq!(used_volumes(&c), vec!["market_db".to_string()]);
    }

    #[test]
    fn references_by_old_id_become_names() {
        let mut by_id = HashMap::new();
        by_id.insert("b".repeat(64), "market-db-1".to_string());
        let c = container();
        assert_eq!(needs(&c, &by_id), vec!["market-db-1".to_string()]);
        let mut b = container_body(&c, "x");
        b["HostConfig"]["NetworkMode"] = json!(format!("container:{}", "b".repeat(64)));
        rename_refs(&mut b, &by_id);
        assert_eq!(b["HostConfig"]["NetworkMode"], "container:market-db-1");
        assert_eq!(b["HostConfig"]["VolumesFrom"], json!(["market-db-1:ro"]));
        let mut shared = c.clone();
        shared["Config"]["Hostname"] = json!("chosen");
        shared["HostConfig"]["NetworkMode"] = json!("container:x");
        shared["HostConfig"]["Dns"] = json!(["1.1.1.1"]);
        let b = container_body(&shared, "x");
        assert!(b.get("Hostname").is_none() && b["HostConfig"].get("Dns").is_none(), "what docker refuses with a shared network goes");
    }

    #[test]
    fn networks_and_volumes() {
        let n = json!({"Name": "market_default", "Driver": "bridge", "Internal": false, "Attachable": false, "EnableIPv6": false,
                       "IPAM": {"Driver": "default", "Config": [{"Subnet": "172.18.0.0/16"}], "Options": null},
                       "Options": {}, "Labels": {"com.docker.compose.network": "default"}});
        let b = network_body(&n);
        assert_eq!(b["IPAM"]["Config"][0]["Subnet"], "172.18.0.0/16");
        assert_eq!(b["Labels"]["com.docker.compose.network"], "default");
        assert!(anonymous(&"c".repeat(64)) && !anonymous("market_db"));
        assert!(data_elsewhere(&json!({"Driver": "local", "Options": {"type": "none", "o": "bind", "device": "/home/bia/db"}})));
        assert!(!data_elsewhere(&json!({"Driver": "local", "Options": null})));
        assert_eq!(api_err("{\"message\":\"Conflict. The container name is in use\"}\n"), "Conflict. The container name is in use");
        assert_eq!(api_err("4a\r\n{\"message\":\"no such image\"}\n\r\n0\r\n\r\n"), "no such image");
        assert_eq!(urlenc("market-app_1.x"), "market-app_1.x");
    }
}
