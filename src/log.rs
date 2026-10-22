use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::{Result, io_err};

pub const DIR: &str = "logs";
pub const KEEP: usize = 20;
const PREFIX: &str = "convert-";

pub struct Log {
    lines: Vec<String>,
    start: Instant,
    mark: Instant,
}

impl Default for Log {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            lines: Vec::new(),
            start: now,
            mark: now,
        }
    }
}

impl Log {
    pub fn line(&mut self, text: impl Into<String>) {
        self.lines.push(text.into());
    }

    pub fn section(&mut self, title: &str) {
        if !self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.lines.push(format!("[{title}]"));
    }

    pub fn time(&mut self, phase: &str) {
        let now = Instant::now();
        let took = now.duration_since(self.mark).as_secs_f64();
        self.mark = now;
        self.lines.push(format!("time: {phase} {took:.3} s"));
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn text(&self) -> String {
        let total = self.start.elapsed().as_secs_f64();
        let mut text = self.lines.join("\n");
        text.push_str(&format!("\ntime: total {total:.3} s\n"));
        match std::env::var("HOME").ok().filter(|h| h.len() > 1) {
            Some(home) => text.replace(home.trim_end_matches('/'), "~"),
            None => text,
        }
    }

    pub fn write(&self, data: &Path, id: &str) -> Result<PathBuf> {
        let dir = data.join(DIR);
        std::fs::create_dir_all(&dir).map_err(io_err(&dir))?;
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
        let id: String = id
            .chars()
            .map(
                |c| match c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    true => c,
                    false => '_',
                },
            )
            .collect();
        let mut path = dir.join(format!("{PREFIX}{stamp}-{id}.log"));
        for n in 2.. {
            if !path.exists() {
                break;
            }
            path = dir.join(format!("{PREFIX}{stamp}-{id}-{n}.log"));
        }
        std::fs::write(&path, self.text()).map_err(io_err(&path))?;
        prune(&dir, KEEP);
        Ok(path)
    }
}

pub fn prune(dir: &Path, keep: usize) {
    let mut logs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(".log"))
        })
        .collect();
    logs.sort();
    let extra = logs.len().saturating_sub(keep);
    for old in &logs[..extra] {
        std::fs::remove_file(old).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_logs_and_hides_the_home_folder() {
        let dir = std::env::temp_dir().join(format!("simplified-logs-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let logs = dir.join(DIR);
        std::fs::create_dir_all(&logs).unwrap();
        for n in 0..KEEP + 3 {
            std::fs::write(logs.join(format!("{PREFIX}2020{n:04}-x.log")), "").unwrap();
        }
        std::fs::write(logs.join("other.txt"), "").unwrap();
        let mut log = Log::default();
        log.section("source");
        if let Ok(home) = std::env::var("HOME") {
            log.line(format!("data: {home}/data"));
        }
        let path = log.write(&dir, "mod.a/b").unwrap();
        let names: Vec<String> = std::fs::read_dir(&logs)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.iter().filter(|n| n.starts_with(PREFIX)).count(), KEEP);
        assert!(names.contains(&"other.txt".to_owned()));
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("-mod.a_b.log")
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("[source]\n"));
        if std::env::var("HOME").is_ok_and(|h| h.len() > 1) {
            assert!(text.contains("data: ~/data"), "{text}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
