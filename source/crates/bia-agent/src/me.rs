//! Who this bia is: the name and gender root gave it (`files/state/self.json`). They open the
//! system prompt, so the model keeps them however long the conversation gets; memory alone
//! let them fade. The web MCP and the PKI read the name too (page titles, certificates).
//! bia deploy hands them over once as `EFI/bia/self.json` (main.rs seed_language).
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const GENDERS: [&str; 3] = ["male", "female", "neutral"];
const NAME_MAX: usize = 40;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Me {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub gender: String,
}

impl Me {
    pub fn path(files: &Path) -> PathBuf {
        files.join("state").join("self.json")
    }

    pub fn load(files: &Path) -> Me {
        bia_mcp::statefile::read_or_default(&Self::path(files))
    }

    pub fn save(&self, files: &Path) -> std::io::Result<()> {
        bia_mcp::statefile::write_json(&Self::path(files), self)
    }

    /// The first line of the system prompt.
    pub fn prompt_line(&self) -> String {
        let pron = match self.gender.as_str() {
            "male" => Some(("male (he/him)", "masculine")),
            "female" => Some(("female (she/her)", "feminine")),
            "neutral" => Some(("neither male nor female (it/they)", "neuter (or plural where the language has no neuter)")),
            _ => None,
        };
        match (self.name.is_empty(), pron) {
            (false, Some((g, forms))) => format!(
                "You are {n}, a bia. {n} is your name and you are {g}: in languages with grammatical gender always use {forms} forms for yourself. Only root can change them (self_set).",
                n = self.name
            ),
            (false, None) => format!(
                "You are {n}, a bia. {n} is your name. Your gender is not set yet: ask root whether you are male, female or neutral, then self_set(gender=…).",
                n = self.name
            ),
            (true, Some((g, forms))) => format!(
                "You are a bia without a name yet, {g} (in languages with grammatical gender use {forms} forms for yourself): ask root what your name should be, then self_set(name=…)."
            ),
            (true, None) => "You are a bia without a name or gender yet: ask root what your name should be and whether you are male, female or neutral, then self_set(name=…, gender=…).".into(),
        }
    }

    /// Apply a self_set call. Returns what changed.
    pub fn apply(&mut self, args: &Value) -> Result<String, String> {
        let name = args.get("name").and_then(Value::as_str).map(|s| s.trim().to_string());
        let gender = args.get("gender").and_then(Value::as_str).map(|s| s.trim().to_lowercase());
        let mut done = vec![];
        if let Some(n) = name {
            check_name(&n)?;
            if n != self.name {
                done.push(format!("name {n} (was {})", or_unset(&self.name)));
                self.name = n;
            }
        }
        if let Some(g) = gender {
            if !GENDERS.contains(&g.as_str()) {
                return Err(format!("gender must be one of {}", GENDERS.join(", ")));
            }
            if g != self.gender {
                done.push(format!("gender {g} (was {})", or_unset(&self.gender)));
                self.gender = g;
            }
        }
        Ok(done.join(", "))
    }
}

fn or_unset(s: &str) -> &str {
    if s.is_empty() {
        "unset"
    } else {
        s
    }
}

pub fn check_name(n: &str) -> Result<(), String> {
    if n.is_empty() || n.chars().count() > NAME_MAX {
        return Err(format!("name must be 1..{NAME_MAX} characters"));
    }
    if n.chars().any(|c| c.is_control() || matches!(c, '|' | '\n' | '·')) {
        return Err("name cannot hold control characters, | or ·".into());
    }
    Ok(())
}

/// Why a self_set call may not run: root may always; a system notice (boot, update) may only
/// fill what is still empty; anyone else may not.
pub fn refuse(me: &Me, args: &Value, by_root: bool, by_system: bool) -> Option<String> {
    if by_root {
        return None;
    }
    let changes = |k: &str, cur: &str| args.get(k).and_then(Value::as_str).is_some_and(|v| !cur.is_empty() && !v.trim().eq_ignore_ascii_case(cur));
    if by_system && !changes("name", &me.name) && !changes("gender", &me.gender) {
        return None;
    }
    Some("only root can change your name or gender; the last message is not from root".into())
}

/// The system prompt with who this bia is as its first line.
pub fn with_self(prompt: &str, me: &Me) -> String {
    format!("{}\n{prompt}", me.prompt_line())
}

/// Name and gender chosen in bia deploy, carried in `EFI/bia/self.json`
/// (`{"name": "Fred", "gender": "male"}`): taken only while none is set.
pub fn seed(files: &Path, esp_json: &Value) -> Option<String> {
    let mut me = Me::load(files);
    if !me.name.is_empty() || !me.gender.is_empty() {
        return None;
    }
    let mut args = serde_json::json!({});
    for k in ["name", "gender"] {
        if let Some(v) = esp_json.get(k).and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
            args[k] = v.into();
        }
    }
    if args.as_object().is_some_and(|o| o.is_empty()) {
        return None;
    }
    Some(match me.apply(&args) {
        Ok(done) => match me.save(files) {
            Ok(()) => format!("self from bia deploy: {done}"),
            Err(e) => format!("self from bia deploy not saved: {e}"),
        },
        Err(e) => format!("self from bia deploy ignored: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_prompt_line_names_and_genders() {
        let fred = Me { name: "Fred".into(), gender: "male".into() };
        let l = fred.prompt_line();
        assert!(l.starts_with("You are Fred, a bia."), "{l}");
        assert!(l.contains("male (he/him)") && l.contains("masculine forms"), "{l}");
        let p = with_self("You are bia, this Debian machine.", &fred);
        assert!(p.starts_with("You are Fred, a bia.") && p.ends_with("\nYou are bia, this Debian machine."));
        assert!(Me { name: "Ann".into(), gender: "female".into() }.prompt_line().contains("feminine"));
        assert!(Me { name: "R2".into(), gender: "neutral".into() }.prompt_line().contains("it/they"));
        assert!(Me::default().prompt_line().contains("self_set(name=…, gender=…)"));
        assert!(Me { name: "Fred".into(), gender: String::new() }.prompt_line().contains("self_set(gender=…)"));
        assert!(Me { name: String::new(), gender: "male".into() }.prompt_line().contains("self_set(name=…)"));
    }

    #[test]
    fn apply_checks_and_reports() {
        let mut m = Me::default();
        assert_eq!(m.apply(&json!({"name": " Фред ", "gender": "Male"})).unwrap(), "name Фред (was unset), gender male (was unset)");
        assert_eq!(m, Me { name: "Фред".into(), gender: "male".into() });
        assert_eq!(m.apply(&json!({"name": "Фред"})).unwrap(), "");
        assert!(m.apply(&json!({"gender": "robot"})).is_err());
        assert!(m.apply(&json!({"name": "a|b"})).is_err());
        assert!(m.apply(&json!({"name": ""})).is_err());
        assert!(m.apply(&json!({"name": "x".repeat(41)})).is_err());
        assert_eq!(m.name, "Фред");
    }

    #[test]
    fn only_root_renames() {
        let empty = Me::default();
        let fred = Me { name: "Fred".into(), gender: "male".into() };
        let rename = json!({"name": "Bob"});
        assert!(refuse(&fred, &rename, true, false).is_none());
        assert!(refuse(&fred, &rename, false, false).is_some());
        assert!(refuse(&fred, &rename, false, true).is_some(), "a notice cannot rename");
        assert!(refuse(&empty, &json!({"name": "Fred", "gender": "male"}), false, true).is_none(), "the update fills empty ones");
        assert!(refuse(&fred, &json!({"name": "Fred"}), false, true).is_none(), "the same value again is fine");
        assert!(refuse(&empty, &rename, false, false).is_some(), "a stranger cannot name it either");
    }

    #[test]
    fn deploy_seeds_once_and_is_saved() {
        let d = std::env::temp_dir().join(format!("bia-me-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        assert!(seed(&d, &json!({})).is_none());
        let msg = seed(&d, &json!({"name": "Fred", "gender": "male"})).unwrap();
        assert!(msg.contains("name Fred"), "{msg}");
        assert_eq!(Me::load(&d), Me { name: "Fred".into(), gender: "male".into() });
        // already named: a later file does not rename it
        assert!(seed(&d, &json!({"name": "Bob"})).is_none());
        assert_eq!(Me::load(&d).name, "Fred");
        let _ = std::fs::remove_dir_all(&d);
    }
}
