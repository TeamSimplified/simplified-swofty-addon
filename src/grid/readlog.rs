use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

const EXAMPLES: usize = 10;

#[derive(Default)]
pub struct Skips(BTreeMap<&'static str, Vec<String>>);

impl Skips {
    pub fn add(&mut self, reason: &'static str, id: &str) {
        self.0.entry(reason).or_default().push(id.to_owned());
    }

    pub fn total(&self) -> usize {
        self.0.values().map(Vec::len).sum()
    }

    pub fn count(&self, reason: &str) -> usize {
        self.0.get(reason).map_or(0, Vec::len)
    }

    pub fn has(&self, reason: &str, id: &str) -> bool {
        self.0.get(reason).is_some_and(|v| v.iter().any(|x| x == id))
    }

    pub fn line(&self) -> String {
        if self.0.is_empty() {
            return "none".into();
        }
        self.0
            .iter()
            .map(|(reason, ids)| {
                let shown: Vec<&str> = ids.iter().take(EXAMPLES).map(String::as_str).collect();
                let more = ids.len().saturating_sub(EXAMPLES);
                let tail = if more > 0 {
                    format!(", +{more} more")
                } else {
                    String::new()
                };
                format!("{reason}: {} ({}{tail})", ids.len(), shown.join(", "))
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

pub struct BuildPick {
    pub chosen: Option<PathBuf>,
    pub folder: PathBuf,
    pub skipped: Vec<(String, &'static str)>,
}

impl BuildPick {
    pub fn line(&self, profile: &str) -> String {
        let name = |p: &Path| {
            p.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        };
        let chosen = self
            .chosen
            .as_deref()
            .map_or_else(|| format!("none under {}", self.folder.display()), name);
        let skipped = if self.skipped.is_empty() {
            "none".to_owned()
        } else {
            self.skipped
                .iter()
                .take(EXAMPLES)
                .map(|(n, why)| format!("{n} ({why})"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!("profile {profile}: build {chosen}; skipped {skipped}")
    }
}

pub fn pick_build(data_root: &Path, profile: &str) -> BuildPick {
    let folder = data_root.join("cache/overlay").join(profile);
    let mut skipped = Vec::new();
    let mut found = Vec::new();
    match std::fs::read_dir(&folder) {
        Ok(entries) => {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let plan = e.path().join("plan.json");
                match plan.metadata().and_then(|m| m.modified()) {
                    Ok(at) => found.push((at, e.path())),
                    Err(_) => skipped.push((name, "no plan.json")),
                }
            }
        }
        Err(_) => skipped.push((folder.display().to_string(), "folder not readable")),
    }
    found.sort();
    let chosen = found.pop().map(|(_, p)| p);
    for (_, p) in found {
        skipped.push((
            p.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            "older plan.json",
        ));
    }
    BuildPick {
        chosen,
        folder,
        skipped,
    }
}

pub fn read_json_checked(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("not valid JSON: {e}"))
}

pub fn data_file(path: &Path) -> String {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    if !path.exists() {
        return format!("{name} missing");
    }
    match read_json_checked(path) {
        Ok(_) => format!("{name} ok"),
        Err(e) => format!("{name} unreadable: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("simplified-readlog-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn skips_show_ten_examples_per_reason() {
        let mut s = Skips::default();
        for i in 0..12 {
            s.add("no head", &format!("d{i}"));
        }
        s.add("off", "x");
        assert_eq!(s.total(), 13);
        assert_eq!(s.count("no head"), 12);
        assert!(s.has("off", "x"));
        let line = s.line();
        assert!(
            line.contains("no head: 12 (d0, d1, d2, d3, d4, d5, d6, d7, d8, d9, +2 more)"),
            "{line}"
        );
        assert!(line.contains("off: 1 (x)"), "{line}");
        assert_eq!(Skips::default().line(), "none");
    }

    #[test]
    fn the_newest_plan_is_the_build_and_the_rest_say_why_they_were_left() {
        let root = dir("pick");
        let profile = root.join("cache/overlay/Original");
        for name in ["old", "new", "empty"] {
            std::fs::create_dir_all(profile.join(name)).unwrap();
        }
        std::fs::write(profile.join("old/plan.json"), "{}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(profile.join("new/plan.json"), "{}").unwrap();
        let pick = pick_build(&root, "Original");
        assert_eq!(pick.chosen.as_deref(), Some(profile.join("new").as_path()));
        assert!(pick.skipped.contains(&("old".to_owned(), "older plan.json")));
        assert!(pick.skipped.contains(&("empty".to_owned(), "no plan.json")));
        let line = pick.line("Original");
        assert!(
            line.starts_with("profile Original: build new; skipped "),
            "{line}"
        );
        let none = pick_build(&root, "Other");
        assert!(none.chosen.is_none());
        assert!(
            none.line("Other").contains("none under"),
            "{}",
            none.line("Other")
        );
    }

    #[test]
    fn a_data_file_is_reported_ok_missing_or_unreadable() {
        let root = dir("data");
        std::fs::write(root.join("a.json"), "{}").unwrap();
        std::fs::write(root.join("b.json"), "{ nope").unwrap();
        assert_eq!(data_file(&root.join("a.json")), "a.json ok");
        assert_eq!(data_file(&root.join("c.json")), "c.json missing");
        assert!(data_file(&root.join("b.json")).starts_with("b.json unreadable: not valid JSON"));
    }
}
