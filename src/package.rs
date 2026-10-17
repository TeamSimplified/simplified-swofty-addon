// The package root, the content root and the entry path checks are ported from
// ModImport.cs and ModStaging.cs of the Simplified Launcher.
// Based on Simplified Suite by Team Simplified - https://github.com/TeamSimplified/simplified-suite
// Copyright (C) 2026 Team Simplified. GPL-3.0; see THIRD_PARTY.md.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use swofty_addon::launcher::{Claim, GameFiles};

use crate::serps;
use crate::{Result, err, io_err};

pub const GAME_FOLDER: &str = "F1 25";
pub const DATABASE: &str = "game_package/gamedatabases/gamedatabases.2025.bin";
pub const TEXT_FOLDERS: [&str; 2] = ["localisation", "localisatioa"];
const MAX_DEPTH: usize = 3;
const EXACT_MARKERS: [&str; 8] = [
    "asset_groups",
    "actionmaps",
    "audio",
    "videos",
    "game_package",
    "character_package",
    "localisation",
    "localisatioa",
];
const SUFFIX_MARKERS: [&str; 3] = ["_package", "_package_shared", "_asset_groups"];
const PREFIX_MARKERS: [&str; 1] = ["shader_package_"];
const CODE: [&str; 3] = ["exe", "dll", "asi"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Database,
    Text,
    Archive,
    Replacement,
    Addition,
}

pub fn class(rel: &str, game: Option<&dyn GameFiles>) -> Option<Class> {
    let lower = rel.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    if CODE.contains(&ext) {
        return None;
    }
    if lower == DATABASE {
        return Some(Class::Database);
    }
    let mut parts = lower.split('/');
    if let (Some(top), Some(_), Some(name), None) = (parts.next(), parts.next(), parts.next(), parts.next())
        && TEXT_FOLDERS.contains(&top)
        && name.starts_with("language_")
        && ext == "lng"
    {
        return Some(Class::Text);
    }
    match (game.is_none_or(|g| g.is_stock(rel)), ext == "erp") {
        (true, true) => Some(Class::Archive),
        (true, false) => Some(Class::Replacement),
        (false, _) => Some(Class::Addition),
    }
}

pub fn game_path(rel: &str) -> String {
    match rel.split_once('/') {
        Some((top, rest)) if top.eq_ignore_ascii_case(TEXT_FOLDERS[1]) => {
            format!("{}/{rest}", TEXT_FOLDERS[0])
        }
        _ => rel.to_owned(),
    }
}

pub fn claims(path: &Path) -> Claim {
    if path.is_dir() {
        let yes = package_root(path).is_some_and(|r| {
            r.join(serps::FILE).is_file()
                || subdirs(&r)
                    .iter()
                    .any(|d| name(d).eq_ignore_ascii_case(GAME_FOLDER))
        });
        return if yes { Claim::Yes } else { Claim::No };
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "serps" {
        return Claim::Yes;
    }
    if ext != "zip" {
        return Claim::No;
    }
    let Some(names) = zip_names(path) else {
        return Claim::No;
    };
    let tops: Vec<&str> = names.iter().filter_map(|n| n.split('/').next()).collect();
    let single = tops
        .windows(2)
        .all(|w| w[0] == w[1])
        .then(|| tops.first().copied())
        .flatten();
    let at = |rel: &str| {
        names
            .iter()
            .any(|n| n == rel || single.is_some_and(|t| *n == format!("{t}/{rel}")))
    };
    let game = |n: &String| {
        n.starts_with(&format!("{GAME_FOLDER}/"))
            || single.is_some_and(|t| n.starts_with(&format!("{t}/{GAME_FOLDER}/")))
    };
    if at(serps::FILE) || names.iter().any(game) {
        Claim::Yes
    } else if tops.iter().any(|t| is_marker(t)) {
        Claim::Maybe
    } else {
        Claim::No
    }
}

fn zip_names(path: &Path) -> Option<Vec<String>> {
    let zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    zip.file_names()
        .map(|n| n.ok().map(|n| n.replace('\\', "/")))
        .collect()
}

pub fn unpack(src: &Path, dest: &Path, progress: &mut dyn FnMut(f32)) -> Result<usize> {
    if src.is_dir() {
        return copy_dir(src, dest);
    }
    let file = std::fs::File::open(src).map_err(io_err(src))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| err(format!("{} is not a zip archive: {e}", file_name(src))))?;
    let mut refused = 0;
    let total = zip.len().max(1) as f32;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| err(format!("{}: {e}", file_name(src))))?;
        progress(i as f32 / total);
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = relative(&String::from_utf8_lossy(entry.name_raw()).replace('\\', "/")) else {
            refused += 1;
            continue;
        };
        let out = dest.join(rel);
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        let mut f = std::fs::File::create(&out).map_err(io_err(&out))?;
        std::io::copy(&mut entry, &mut f).map_err(io_err(&out))?;
    }
    Ok(refused)
}

fn copy_dir(src: &Path, dest: &Path) -> Result<usize> {
    let mut refused = 0;
    for file in files(src)? {
        let rel = file.strip_prefix(src).expect("under src");
        if std::fs::symlink_metadata(&file)
            .map_err(io_err(&file))?
            .is_symlink()
        {
            refused += 1;
            continue;
        }
        let out = dest.join(rel);
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        std::fs::copy(&file, &out).map_err(io_err(&out))?;
    }
    Ok(refused)
}

pub fn files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(io_err(&dir))? {
            let path = entry.map_err(io_err(&dir))?.path();
            if path.is_dir() && !path.is_symlink() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn relative(name: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = name.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty()
        || name.starts_with('/')
        || parts.iter().any(|p| matches!(*p, "." | "..") || p.contains(':'))
    {
        return None;
    }
    let path: PathBuf = parts.iter().collect();
    path.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then_some(path)
}

pub fn package_root(dir: &Path) -> Option<PathBuf> {
    if dir.join(serps::FILE).is_file() {
        return Some(dir.to_path_buf());
    }
    let children = subdirs(dir);
    let all = std::fs::read_dir(dir).ok()?.count();
    match children.as_slice() {
        [only] if all == 1 && only.join(serps::FILE).is_file() => Some(only.clone()),
        _ => Some(dir.to_path_buf()),
    }
}

pub fn content_root(root: &Path) -> Option<PathBuf> {
    named_game(root).or_else(|| {
        std::iter::once(root.to_path_buf())
            .chain(walk(root))
            .find(|d| subdirs(d).iter().any(|c| is_marker(&name(c))))
    })
}

fn named_game(root: &Path) -> Option<PathBuf> {
    walk(root).find(|d| name(d).eq_ignore_ascii_case(GAME_FOLDER))
}

fn walk(root: &Path) -> impl Iterator<Item = PathBuf> {
    let mut out = Vec::new();
    let mut level = vec![root.to_path_buf()];
    for _ in 0..MAX_DEPTH {
        let next: Vec<PathBuf> = level.iter().flat_map(|d| subdirs(d)).collect();
        out.extend(next.iter().cloned());
        level = next;
    }
    out.into_iter()
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn is_marker(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    EXACT_MARKERS.contains(&lower.as_str())
        || SUFFIX_MARKERS.iter().any(|s| lower.ends_with(s))
        || PREFIX_MARKERS.iter().any(|p| lower.starts_with(p))
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut f| f.read_to_end(&mut buf))
        .map_err(io_err(path))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_follow_the_path() {
        assert_eq!(class(DATABASE, None), Some(Class::Database));
        assert_eq!(
            class("localisatioa/2025_release/language_eng.lng", None),
            Some(Class::Text)
        );
        assert_eq!(
            class("localisation/2025_release/language_jap.lng", None),
            Some(Class::Text)
        );
        assert_eq!(
            class("shader_package_2025/ui_texture.nefx2.sm51.erp", None),
            Some(Class::Archive)
        );
        assert_eq!(class("audio/x.bnk", None), Some(Class::Replacement));
        assert_eq!(class("bin/hook.DLL", None), None);
        assert_eq!(
            game_path("localisatioa/2025_release/language_eng.lng"),
            "localisation/2025_release/language_eng.lng"
        );
        assert_eq!(relative("a/../b"), None);
        assert_eq!(relative("/etc/passwd"), None);
        assert_eq!(relative("c:/x"), None);
        assert_eq!(relative("F1 25//a.erp"), Some(PathBuf::from("F1 25/a.erp")));
    }

    #[test]
    fn finds_the_content_root() {
        let dir = std::env::temp_dir().join(format!("simplified-root-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let deep = dir.join("wrap/inner/shader_package_2025");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(content_root(&dir), Some(dir.join("wrap/inner")));
        std::fs::create_dir_all(dir.join("wrap/F1 25/game_package")).unwrap();
        assert_eq!(content_root(&dir), Some(dir.join("wrap/F1 25")));
        assert_eq!(claims(&dir), Claim::No);
        assert_eq!(claims(&dir.join("wrap")), Claim::Yes);
        std::fs::remove_dir_all(&dir).ok();
    }
}
