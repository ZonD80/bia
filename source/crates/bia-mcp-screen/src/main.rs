//! Display :0 as a person would see it: resolution, a screenshot, mouse and keys.
use bia_mcp::{arg_str, arg_u32_opt, Server};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() {
    Server::new("screen", env!("CARGO_PKG_VERSION"))
        .tool(
            "info",
            "Width and height of the screen in pixels, where the mouse is, and the name of the focused window. x and y start at the top-left.",
            json_obj(),
            |_, _| info(),
        )
        .tool(
            "shot",
            "Take a picture of the screen. You can see the picture in the result. Call this before you click.",
            json_obj(),
            |_, _| shot(),
        )
        .tool(
            "click",
            "Move the mouse to x,y and click. button 1 is left, 2 is middle, 3 is right. repeat 2 is a double-click. button 0 only moves the mouse.",
            serde_json::json!({"type": "object", "required": ["x", "y"], "properties": {
                "x": {"type": "integer"}, "y": {"type": "integer"},
                "button": {"type": "integer"}, "repeat": {"type": "integer"}
            }}),
            |_, a| click(&a),
        )
        .tool(
            "cursor_move_human",
            "Move the mouse to x,y like a person: a curved path with small pauses, not a straight jump. Call this before a captcha click or when a site watches the cursor. duration_ms is optional. Omit it for a random time between 500 and 1500 ms.",
            serde_json::json!({"type": "object", "required": ["x", "y"], "properties": {
                "x": {"type": "integer"},
                "y": {"type": "integer"},
                "duration_ms": {"type": "integer", "description": "optional total move time in milliseconds. Default is random 500–1500."}
            }}),
            |_, a| cursor_move_human(&a),
        )
        .tool(
            "drag",
            "Hold the left button at x,y, move to x2,y2, and let go.",
            serde_json::json!({"type": "object", "required": ["x", "y", "x2", "y2"], "properties": {
                "x": {"type": "integer"}, "y": {"type": "integer"},
                "x2": {"type": "integer"}, "y2": {"type": "integer"}
            }}),
            |_, a| drag(&a),
        )
        .tool(
            "scroll",
            "Scroll at x,y. dy above 0 scrolls down. dy below 0 scrolls up. Use 1 to 20.",
            serde_json::json!({"type": "object", "required": ["x", "y", "dy"], "properties": {
                "x": {"type": "integer"}, "y": {"type": "integer"}, "dy": {"type": "integer"}
            }}),
            |_, a| scroll(&a),
        )
        .tool(
            "type",
            "Type text into the window that has focus. Does not press Enter. Use screen_key with Return for Enter.",
            serde_json::json!({"type": "object", "required": ["text"], "properties": {
                "text": {"type": "string"}
            }}),
            |_, a| type_text(arg_str(&a, "text")?),
        )
        .tool(
            "key",
            "Press one key or a shortcut. Examples: Return, Tab, Escape, BackSpace, Up, Down, Left, Right, ctrl+a, ctrl+c, ctrl+v, alt+Tab.",
            serde_json::json!({"type": "object", "required": ["key"], "properties": {
                "key": {"type": "string"}
            }}),
            |_, a| key(arg_str(&a, "key")?),
        )
        .run();
}

fn json_obj() -> Value {
    serde_json::json!({"type": "object", "properties": {}})
}

fn files_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIA_FILES").unwrap_or_else(|_| "/home/bia/files".into()))
}

fn display(cmd: &mut Command) {
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
    cmd.env("DISPLAY", display);
    let xauth = if Path::new("/home/bia/.Xauthority").is_file() {
        "/home/bia/.Xauthority"
    } else {
        "/run/bia/Xauthority"
    };
    cmd.env("XAUTHORITY", xauth);
}

fn run(bin: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    display(&mut cmd);
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("{bin} is not installed")
        } else {
            format!("{bin}: {e}")
        }
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        let msg = if !stderr.is_empty() { stderr } else { stdout };
        return Err(format!("{bin} failed: {msg}"));
    }
    Ok(stdout)
}

fn xy(args: &Value, xkey: &str, ykey: &str) -> Result<(i32, i32), String> {
    Ok((num(args, xkey)?, num(args, ykey)?))
}

fn num(args: &Value, key: &str) -> Result<i32, String> {
    let v = args.get(key).ok_or_else(|| format!("missing argument: {key}"))?;
    let n = v
        .as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| format!("{key} must be an integer"))?;
    i32::try_from(n).map_err(|_| format!("{key} is out of range"))
}

fn size() -> Result<(u32, u32), String> {
    let text = run("xdpyinfo", &[])?;
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("dimensions:") else { continue };
        let dims = rest.split_whitespace().next().unwrap_or("");
        if let Some((w, h)) = dims.split_once('x') {
            if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                if w > 0 && h > 0 {
                    return Ok((w, h));
                }
            }
        }
    }
    Err("display :0 has no size (X is not up)".into())
}

fn info() -> Result<String, String> {
    let (w, h) = size()?;
    let mouse = run("xdotool", &["getmouselocation", "--shell"]).unwrap_or_default();
    let mx = shell_val(&mouse, "X");
    let my = shell_val(&mouse, "Y");
    let window = run("xdotool", &["getactivewindow", "getwindowname"]).unwrap_or_default();
    let window = if window.is_empty() { "none".into() } else { window };
    Ok(format!("display :0\nresolution {w}x{h}\nmouse {mx},{my}\nwindow {window}"))
}

fn shell_val(text: &str, key: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")))
        .unwrap_or("?")
        .to_string()
}

fn shot() -> Result<String, String> {
    let (w, h) = size()?;
    let dir = files_dir().join("state").join("screen");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let path = dir.join(format!("{stamp}.png"));
    let path_s = path.display().to_string();
    run("scrot", &["-o", &path_s])?;
    if !path.is_file() {
        return Err(format!("scrot wrote nothing at {path_s}"));
    }
    prune(&dir);
    Ok(format!("{w}x{h}\nimage: {path_s}"))
}

fn prune(dir: &Path) {
    let mut files: Vec<(SystemTime, PathBuf)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let t = e.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
        files.push((t, p));
    }
    files.sort_by_key(|(t, _)| *t);
    while files.len() > 6 {
        let (_, p) = files.remove(0);
        let _ = std::fs::remove_file(p);
    }
}

fn click(args: &Value) -> Result<String, String> {
    let (x, y) = xy(args, "x", "y")?;
    let button = arg_u32_opt(args, "button").unwrap_or(1);
    let repeat = arg_u32_opt(args, "repeat").unwrap_or(1).clamp(1, 3);
    if button == 0 {
        run("xdotool", &["mousemove", "--sync", &x.to_string(), &y.to_string()])?;
        return Ok(format!("moved to {x},{y}"));
    }
    if !(1..=3).contains(&button) {
        return Err("button must be 0, 1, 2, or 3".into());
    }
    run(
        "xdotool",
        &[
            "mousemove",
            "--sync",
            &x.to_string(),
            &y.to_string(),
            "click",
            "--repeat",
            &repeat.to_string(),
            "--delay",
            "40",
            &button.to_string(),
        ],
    )?;
    Ok(format!("clicked {button} x{repeat} at {x},{y}"))
}

fn mouse_xy() -> Result<(i32, i32), String> {
    let mouse = run("xdotool", &["getmouselocation", "--shell"])?;
    let mx = shell_val(&mouse, "X").parse::<i32>().map_err(|_| "mouse x unknown".to_string())?;
    let my = shell_val(&mouse, "Y").parse::<i32>().map_err(|_| "mouse y unknown".to_string())?;
    Ok((mx, my))
}

fn cursor_move_human(args: &Value) -> Result<String, String> {
    let (x1, y1) = mouse_xy()?;
    let (x2, y2) = xy(args, "x", "y")?;
    let seed = Instant::now().elapsed().as_nanos() as u64
        ^ SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
        ^ ((x1 as u64) << 32)
        ^ (y1 as u64);
    let duration = arg_u32_opt(args, "duration_ms")
        .map(|n| n.clamp(120, 4000))
        .unwrap_or_else(|| 500 + (Rng(seed | 1).next() % 1001) as u32);
    let steps = human_path(x1, y1, x2, y2, duration, seed.wrapping_mul(3));
    let start = Instant::now();
    for (i, step) in steps.iter().enumerate() {
        let target = start + Duration::from_millis(step.at_ms as u64);
        let now = Instant::now();
        if target > now {
            thread::sleep(target - now);
        }
        run(
            "xdotool",
            &["mousemove", "--sync", &step.x.to_string(), &step.y.to_string()],
        )?;
        if i + 1 == steps.len() {
            break;
        }
    }
    Ok(format!(
        "moved like a person from {x1},{y1} to {x2},{y2} in about {duration} ms ({n} steps)",
        n = steps.len()
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Step {
    x: i32,
    y: i32,
    at_ms: u32,
}

/// Cubic Bézier path with overshoot and ease-in-out timing. Pure, for tests.
fn human_path(x1: i32, y1: i32, x2: i32, y2: i32, duration_ms: u32, seed: u64) -> Vec<Step> {
    let mut rng = Rng(seed | 1);
    let dx = (x2 - x1) as f64;
    let dy = (y2 - y1) as f64;
    let dist = dx.hypot(dy).max(1.0);
    let (nx, ny) = (-dy / dist, dx / dist);
    let bend = (dist * (0.12 + rng.f01() * 0.28)).clamp(8.0, 140.0) * if rng.bool() { 1.0 } else { -1.0 };
    let c1x = x1 as f64 + dx * (0.20 + rng.f01() * 0.15) + nx * bend * (0.4 + rng.f01() * 0.6);
    let c1y = y1 as f64 + dy * (0.20 + rng.f01() * 0.15) + ny * bend * (0.4 + rng.f01() * 0.6);
    let c2x = x1 as f64 + dx * (0.65 + rng.f01() * 0.15) - nx * bend * (0.2 + rng.f01() * 0.5);
    let c2y = y1 as f64 + dy * (0.65 + rng.f01() * 0.15) - ny * bend * (0.2 + rng.f01() * 0.5);
    let n = ((dist / 14.0).round() as usize).clamp(8, 48);
    let mut out: Vec<Step> = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let e = ease_in_out(t);
        let (bx, by) = bezier(e, x1 as f64, y1 as f64, c1x, c1y, c2x, c2y, x2 as f64, y2 as f64);
        let jitter = if i == 0 || i == n {
            0.0
        } else {
            (rng.f01() - 0.5) * (1.2 + dist * 0.004).min(3.5)
        };
        let x = (bx + nx * jitter).round() as i32;
        let y = (by + ny * jitter).round() as i32;
        let at = (duration_ms as f64 * e).round() as u32;
        if let Some(prev) = out.last() {
            if prev.x == x && prev.y == y {
                continue;
            }
        }
        out.push(Step { x, y, at_ms: at });
    }
    if out.last().map(|s| (s.x, s.y)) != Some((x2, y2)) {
        out.push(Step {
            x: x2,
            y: y2,
            at_ms: duration_ms,
        });
    }
    out
}

fn ease_in_out(t: f64) -> f64 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
    }
}

fn bezier(t: f64, x0: f64, y0: f64, x1: f64, y1: f64, x2: f64, y2: f64, x3: f64, y3: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let uu = u * u;
    let tt = t * t;
    let uuu = uu * u;
    let ttt = tt * t;
    let x = uuu * x0 + 3.0 * uu * t * x1 + 3.0 * u * tt * x2 + ttt * x3;
    let y = uuu * y0 + 3.0 * uu * t * y1 + 3.0 * u * tt * y2 + ttt * y3;
    (x, y)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.0
    }
    fn f01(&mut self) -> f64 {
        (self.next() >> 11) as f64 / ((1u64 << 53) as f64)
    }
    fn bool(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

fn drag(args: &Value) -> Result<String, String> {
    let (x, y) = xy(args, "x", "y")?;
    let (x2, y2) = xy(args, "x2", "y2")?;
    run(
        "xdotool",
        &[
            "mousemove",
            "--sync",
            &x.to_string(),
            &y.to_string(),
            "mousedown",
            "1",
            "mousemove",
            "--sync",
            &x2.to_string(),
            &y2.to_string(),
            "mouseup",
            "1",
        ],
    )?;
    Ok(format!("dragged {x},{y} -> {x2},{y2}"))
}

fn scroll(args: &Value) -> Result<String, String> {
    let (x, y) = xy(args, "x", "y")?;
    let dy = num(args, "dy")?;
    if dy == 0 {
        return Err("dy must not be 0".into());
    }
    let button = if dy > 0 { "5" } else { "4" };
    let n = dy.unsigned_abs().clamp(1, 20);
    run(
        "xdotool",
        &[
            "mousemove",
            "--sync",
            &x.to_string(),
            &y.to_string(),
            "click",
            "--repeat",
            &n.to_string(),
            "--delay",
            "30",
            button,
        ],
    )?;
    Ok(format!("scrolled {} {n} at {x},{y}", if dy > 0 { "down" } else { "up" }))
}

fn type_text(text: &str) -> Result<String, String> {
    let text = text.replace('\r', "");
    if text.is_empty() {
        return Err("text is empty".into());
    }
    if text.chars().count() > 4000 {
        return Err("text is longer than 4000 characters".into());
    }
    // Newlines are separate Return presses so a multi-line paste still lands.
    for (i, part) in text.split('\n').enumerate() {
        if i > 0 {
            run("xdotool", &["key", "--", "Return"])?;
        }
        if !part.is_empty() {
            run("xdotool", &["type", "--delay", "12", "--", part])?;
        }
    }
    Ok(format!("typed {} characters", text.chars().count()))
}

fn key(combo: &str) -> Result<String, String> {
    let combo = combo.trim();
    if combo.is_empty() || combo.chars().count() > 64 {
        return Err("key is empty or too long".into());
    }
    if combo.chars().any(|c| c.is_control() || matches!(c, ' ' | '\'' | '"' | ';' | '|' | '&')) {
        return Err("key must be one xdotool name, such as Return or ctrl+l".into());
    }
    run("xdotool", &["key", "--", combo])?;
    Ok(format!("pressed {combo}"))
}

#[cfg(test)]
mod tests {
    use super::{ease_in_out, human_path};

    #[test]
    fn human_path_ends_at_target_and_is_not_a_line() {
        let steps = human_path(10, 10, 400, 300, 800, 42);
        assert!(steps.len() >= 8);
        let last = steps.last().unwrap();
        assert_eq!((last.x, last.y), (400, 300));
        assert_eq!(last.at_ms, 800);
        assert_eq!(steps[0].at_ms, 0);
        let mid = &steps[steps.len() / 2];
        let line_y = 10.0 + (300.0 - 10.0) * ((mid.x - 10) as f64 / 390.0);
        assert!((mid.y as f64 - line_y).abs() > 2.0, "path should bend off the straight line");
    }

    #[test]
    fn ease_in_out_is_slow_at_ends() {
        assert!((ease_in_out(0.0) - 0.0).abs() < 1e-9);
        assert!((ease_in_out(1.0) - 1.0).abs() < 1e-9);
        assert!(ease_in_out(0.25) < 0.25);
        assert!(ease_in_out(0.75) > 0.75);
    }
}
