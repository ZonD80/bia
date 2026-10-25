//! In-flight MCP work. Each model-facing tools/call is a job; the result arrives
//! as `system|job:<id>|issued|delivered|<msgid>|…`. List/logs/cancel are builtins.
use crate::console::Inbound;
use crate::tools::ID_SYSTEM;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const PREFIX: &str = "job:";
const KEEP_DONE: usize = 40;
/// Job logs older than this are deleted at boot and then once a day (each job writes one).
const LOG_KEEP: Duration = Duration::from_secs(30 * 86400);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub tool: String,
    pub mcp: String,
    pub jsonrpc_id: u64,
    #[serde(default)]
    pub call_id: String,
    pub state: String,
    pub issued: u64,
    #[serde(default)]
    pub ended: u64,
    #[serde(default)]
    pub log_path: String,
    #[serde(default)]
    pub pid: u32,
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    next: u32,
    #[serde(default)]
    jobs: Vec<Job>,
}

pub struct Store {
    path: PathBuf,
    log_dir: PathBuf,
    jobs: Vec<Job>,
    next: u32,
    pruned: u64,
}

impl Store {
    pub fn load(state_dir: &Path) -> Store {
        let path = state_dir.join("jobs.json");
        let log_dir = state_dir
            .parent()
            .unwrap_or(state_dir)
            .join("logs")
            .join("jobs");
        let _ = std::fs::create_dir_all(&log_dir);
        let (jobs, next) = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<File>(&b).ok())
            .map(|f| (f.jobs, f.next))
            .unwrap_or_default();
        let mut jobs = jobs;
        // job_cancel was replaced by job_kill.
        for j in jobs.iter_mut().filter(|j| j.state == "cancelling") {
            j.state = "killing".into();
        }
        let mut store = Store {
            path,
            log_dir,
            jobs,
            next,
            pruned: 0,
        };
        store.prune_logs();
        store
    }

    /// Delete job logs older than LOG_KEEP, except those of jobs still running.
    fn prune_logs(&mut self) {
        self.pruned = now();
        let Ok(rd) = std::fs::read_dir(&self.log_dir) else { return };
        let live = SystemTime::now();
        for e in rd.flatten() {
            let path = e.path();
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| live.duration_since(t).ok())
                .is_some_and(|age| age > LOG_KEEP);
            let running = self.jobs.iter().any(|j| {
                (j.state == "running" || j.state == "killing") && Path::new(&j.log_path) == path
            });
            if old && !running && e.file_name().to_string_lossy().ends_with(".log") {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    fn save(&self) {
        if let Some(p) = self.path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let keep = self.compacted();
        let f = File {
            next: self.next,
            jobs: keep,
        };
        let tmp = self.path.with_extension("json.tmp");
        if let Ok(buf) = serde_json::to_vec_pretty(&f) {
            let _ = std::fs::write(&tmp, buf);
            let _ = std::fs::rename(&tmp, &self.path)
                .or_else(|_| std::fs::copy(&tmp, &self.path).map(|_| ()));
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn compacted(&self) -> Vec<Job> {
        let mut running: Vec<Job> = self
            .jobs
            .iter()
            .filter(|j| j.state == "running" || j.state == "killing")
            .cloned()
            .collect();
        let mut done: Vec<Job> = self
            .jobs
            .iter()
            .filter(|j| j.state != "running" && j.state != "killing")
            .cloned()
            .collect();
        if done.len() > KEEP_DONE {
            done.drain(0..done.len() - KEEP_DONE);
        }
        running.append(&mut done);
        running
    }

    pub fn get(&self, id: &str) -> Option<&Job> {
        let id = id.trim().trim_start_matches(PREFIX);
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Job> {
        let id = id.trim().trim_start_matches(PREFIX).to_string();
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    pub fn alloc(&mut self, tool: &str, mcp: &str, call_id: &str) -> Result<Job, String> {
        // No cap on running jobs (boot starts every MCP as one); ids wrap at 16 bits.
        let mut id = String::new();
        for _ in 0..0x10000 {
            self.next = self.next.wrapping_add(1);
            if self.next == 0 {
                self.next = 1;
            }
            let cand = format!("{:04x}", self.next & 0xffff);
            if !self
                .jobs
                .iter()
                .any(|j| j.id == cand && (j.state == "running" || j.state == "killing"))
            {
                id = cand;
                break;
            }
        }
        if id.is_empty() {
            return Err("could not allocate a job id".into());
        }
        if now().saturating_sub(self.pruned) >= 86400 {
            self.prune_logs();
        }
        let log_path = self.log_dir.join(format!("{id}.log"));
        let _ = std::fs::File::create(&log_path);
        // the agent is root; the log sits in bia's files like everything else there
        if let Some(files) = self.log_dir.parent().and_then(Path::parent) {
            crate::repl::own_like_files(&log_path, files);
        }
        let job = Job {
            id: id.clone(),
            tool: tool.into(),
            mcp: mcp.into(),
            jsonrpc_id: 0,
            call_id: call_id.into(),
            state: "running".into(),
            issued: now(),
            ended: 0,
            log_path: log_path.to_string_lossy().into(),
            pid: 0,
        };
        self.jobs.push(job.clone());
        self.save();
        Ok(job)
    }

    pub fn set_rpc(&mut self, id: &str, jsonrpc_id: u64) {
        if let Some(j) = self.get_mut(id) {
            j.jsonrpc_id = jsonrpc_id;
        }
        self.save();
    }

    pub fn set_pid(&mut self, id: &str, pid: u32) {
        if let Some(j) = self.get_mut(id) {
            j.pid = pid;
        }
        self.save();
    }

    /// Returns true if this call won (job was still running).
    pub fn finish(&mut self, id: &str, state: &str, body: &str) -> bool {
        let id = id.trim().trim_start_matches(PREFIX).to_string();
        let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return false;
        };
        if j.state != "running" && j.state != "killing" {
            return false;
        }
        let killing = j.state == "killing";
        j.state = if killing && state != "lost" {
            "killed".into()
        } else {
            state.into()
        };
        j.ended = now();
        self.save();
        let _ = body;
        true
    }

    pub fn mark_killing(&mut self, id: &str) -> Option<Job> {
        let j = self.get_mut(id)?;
        if j.state != "running" {
            return None;
        }
        j.state = "killing".into();
        let out = j.clone();
        self.save();
        Some(out)
    }

    pub fn append_log(&self, id: &str, line: &str) {
        let Some(j) = self.get(id) else { return };
        if j.log_path.is_empty() {
            return;
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&j.log_path)
        {
            let _ = writeln!(f, "{line}");
        }
    }

    pub fn logs(&self, id: &str, tail: usize) -> Result<String, String> {
        let j = self.get(id).ok_or_else(|| format!("no job {id}"))?;
        if j.log_path.is_empty() {
            return Ok("(no log)".into());
        }
        let data = std::fs::read(&j.log_path).unwrap_or_default();
        let text = String::from_utf8_lossy(&data);
        let lines: Vec<&str> = text.lines().collect();
        let n = if tail == 0 { 80 } else { tail };
        let start = lines.len().saturating_sub(n);
        Ok(if lines.is_empty() {
            "(empty)".into()
        } else {
            lines[start..].join("\n")
        })
    }

    /// Running jobs on one line, oldest first (for the `unanswered:` notice).
    pub fn running_brief(&self) -> String {
        let mut running: Vec<&Job> = self
            .jobs
            .iter()
            .filter(|j| j.state == "running" || j.state == "killing")
            .collect();
        running.sort_by_key(|j| j.issued);
        running
            .iter()
            .map(|j| format!("{PREFIX}{} {} started {}", j.id, j.tool, ago(j.issued)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn format_list(&self) -> String {
        let mut running: Vec<&Job> = self
            .jobs
            .iter()
            .filter(|j| j.state == "running" || j.state == "killing")
            .collect();
        let mut done: Vec<&Job> = self
            .jobs
            .iter()
            .filter(|j| j.state != "running" && j.state != "killing")
            .collect();
        running.sort_by_key(|j| j.issued);
        done.sort_by_key(|j| j.ended);
        if done.len() > 10 {
            done = done[done.len() - 10..].to_vec();
        }
        let mut rows = Vec::new();
        for j in running {
            rows.push(format!(
                "{}  {}  {}  {}",
                j.id,
                j.state,
                j.tool,
                ago(j.issued)
            ));
        }
        for j in done {
            rows.push(format!(
                "{}  {}  {}  {}",
                j.id,
                j.state,
                j.tool,
                ago(j.ended.max(j.issued))
            ));
        }
        if rows.is_empty() {
            "(no jobs)".into()
        } else {
            rows.join("\n")
        }
    }

    /// Previous boot left running jobs. Mark lost and return envelopes to deliver.
    pub fn fail_running_lost(&mut self) -> Vec<Inbound> {
        let mut out = vec![];
        let ids: Vec<String> = self
            .jobs
            .iter()
            .filter(|j| j.state == "running" || j.state == "killing")
            .map(|j| j.id.clone())
            .collect();
        for id in ids {
            if self.finish(&id, "lost", "") {
                out.push(inbound(
                    &id,
                    "lost\njob was running when the machine stopped",
                ));
            }
        }
        out
    }

    pub fn by_rpc(&self, mcp: &str, jsonrpc_id: u64) -> Option<&Job> {
        self.jobs.iter().find(|j| {
            j.mcp == mcp
                && j.jsonrpc_id == jsonrpc_id
                && (j.state == "running" || j.state == "killing")
        })
    }

    pub fn running_on(&self, mcp: &str) -> Vec<&Job> {
        self.jobs
            .iter()
            .filter(|j| j.mcp == mcp && (j.state == "running" || j.state == "killing"))
            .collect()
    }
}

pub fn inbound(id: &str, body: &str) -> Inbound {
    let text = if body.len() > 16 * 1024 {
        let mut end = 16 * 1024;
        while end > 0 && !body.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}\n...[truncated]", &body[..end])
    } else {
        body.to_string()
    };
    Inbound {
        identity: ID_SYSTEM.into(),
        channel: format!("{PREFIX}{id}"),
        issued: SystemTime::now(),
        id: String::new(),
        text,
    }
    .ensure_id()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn ago(unix: u64) -> String {
    let n = now().saturating_sub(unix);
    if n < 60 {
        format!("{n}s ago")
    } else if n < 3600 {
        format!("{}m ago", n / 60)
    } else {
        format!("{}h ago", n / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_job_logs_are_pruned() {
        let dir = std::env::temp_dir().join(format!("bia-jobs-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let logs = dir.join("logs/jobs");
        std::fs::create_dir_all(&logs).unwrap();
        let month_ago = SystemTime::now() - Duration::from_secs(31 * 86400);
        for name in ["old.log", "busy.log", "old.txt", "new.log"] {
            let f = std::fs::File::create(logs.join(name)).unwrap();
            if name != "new.log" {
                f.set_modified(month_ago).unwrap();
            }
        }
        let busy = Job {
            id: "busy".into(),
            tool: "t".into(),
            mcp: "m".into(),
            jsonrpc_id: 0,
            call_id: String::new(),
            state: "running".into(),
            issued: 0,
            ended: 0,
            log_path: logs.join("busy.log").to_string_lossy().into(),
            pid: 0,
        };
        let file = File { next: 0, jobs: vec![busy] };
        std::fs::create_dir_all(dir.join("state")).unwrap();
        std::fs::write(dir.join("state/jobs.json"), serde_json::to_vec(&file).unwrap()).unwrap();
        let mut st = Store::load(&dir.join("state"));
        assert!(!logs.join("old.log").exists(), "a log older than 30 days goes");
        assert!(logs.join("busy.log").exists(), "a running job keeps its log");
        assert!(logs.join("old.txt").exists(), "only .log files");
        assert!(logs.join("new.log").exists());
        let job = st.alloc("t", "m", "").unwrap();
        assert!(Path::new(&job.log_path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
