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
// A mod never replaces EA AntiCheat, except its splash image, which is only shown
// while it starts.
const ANTICHEAT: &str = "eaanticheat.";
const ANTICHEAT_SPLASH: &str = "eaanticheat.splash.png";

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
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    if CODE.contains(&ext) || file.starts_with(ANTICHEAT) && file != ANTICHEAT_SPLASH {
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

// Like copying a mod into the game folder: anything that holds game folders, at
// most three folders down, or a folder named after the game is a mod.
pub fn claims(path: &Path) -> Claim {
    if path.is_dir() {
        let yes =
            package_root(path).is_some_and(|r| r.join(serps::FILE).is_file() || content_root(&r).is_some());
        return if yes { Claim::Yes } else { Claim::No };
    }
    match extension(path).as_str() {
        "serps" | "7z" | "rar" => return Claim::Yes,
        "zip" => {}
        _ => return Claim::No,
    }
    let Some(names) = zip_names(path) else {
        return Claim::No;
    };
    let game = |n: &String| {
        let dirs: Vec<&str> = n.split('/').collect();
        dirs[..dirs.len() - 1]
            .iter()
            .take(MAX_DEPTH + 1)
            .any(|d| is_game_folder(d) || is_marker(d))
    };
    let serps_at_root =
        |n: &String| n.ends_with(serps::FILE) && n.split('/').filter(|p| !p.is_empty()).count() <= 2;
    if names.iter().any(|n| game(n) || serps_at_root(n)) {
        Claim::Yes
    } else {
        Claim::No
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
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
    match extension(src).as_str() {
        "7z" => unpack_7z(src, dest, progress).or_else(|e| match extractor() {
            Some(tool) => unpack_with(&tool, src, dest),
            None => Err(e),
        }),
        "rar" => unpack_rar(src, dest, progress).or_else(|e| match extractor() {
            Some(tool) => unpack_with(&tool, src, dest),
            None => Err(e),
        }),
        _ => unpack_zip(src, dest, progress),
    }
}

fn unpack_zip(src: &Path, dest: &Path, progress: &mut dyn FnMut(f32)) -> Result<usize> {
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

fn unpack_7z(src: &Path, dest: &Path, progress: &mut dyn FnMut(f32)) -> Result<usize> {
    let fail = |e: sevenz_rust2::Error| {
        err(format!(
            "{} is not a 7z archive this can read: {e}",
            file_name(src)
        ))
    };
    let mut reader = sevenz_rust2::ArchiveReader::open(src, sevenz_rust2::Password::empty()).map_err(fail)?;
    let total = reader.archive().files.len().max(1) as f32;
    let (mut done, mut refused) = (0, 0);
    let mut failed = None;
    reader
        .for_each_entries(|entry, data| {
            done += 1;
            progress(done as f32 / total);
            if entry.is_directory() || entry.is_anti_item() {
                return Ok(true);
            }
            let Some(rel) = relative(&entry.name().replace('\\', "/")) else {
                refused += 1;
                std::io::copy(data, &mut std::io::sink())?;
                return Ok(true);
            };
            let out = dest.join(rel);
            let written = out
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::File::create(&out))
                .and_then(|mut f| std::io::copy(data, &mut f));
            if let Err(e) = written {
                failed = Some((out, e));
                return Ok(false);
            }
            Ok(true)
        })
        .map_err(fail)?;
    if let Some((out, e)) = failed {
        return Err(io_err(out)(e));
    }
    Ok(refused)
}

fn unpack_rar(src: &Path, dest: &Path, progress: &mut dyn FnMut(f32)) -> Result<usize> {
    let fail = |e: rars::Error| {
        err(format!(
            "{} is not a RAR archive this can read: {e}",
            file_name(src)
        ))
    };
    let archive = rars::ArchiveReader::read_path(src).map_err(fail)?;
    let mut refused = 0;
    archive
        .extract_to(None, |entry| {
            let sink = || -> rars::Result<Box<dyn std::io::Write>> { Ok(Box::new(std::io::sink())) };
            if entry.is_directory {
                return sink();
            }
            let Some(rel) = relative(&String::from_utf8_lossy(&entry.name).replace('\\', "/")) else {
                refused += 1;
                return sink();
            };
            let out = dest.join(rel);
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            Ok(Box::new(std::fs::File::create(&out)?))
        })
        .map_err(fail)?;
    progress(1.0);
    Ok(refused)
}

// 7-Zip or UnRAR, for archives the built-in readers can't read. Either is looked
// for on PATH and where its installer puts it.
#[derive(Debug)]
enum Tool {
    SevenZip(PathBuf),
    UnRar(PathBuf),
}

fn extractor() -> Option<Tool> {
    let on_path = |name: &str| {
        let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .map(|dir| dir.join(&exe))
            .find(|p| p.is_file())
    };
    let installed = |rel: &str| {
        ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|dir| PathBuf::from(dir).join(rel))
            .find(|p| p.is_file())
    };
    ["7z", "7zz", "7za"]
        .iter()
        .find_map(|n| on_path(n))
        .or_else(|| installed("7-Zip/7z.exe"))
        .map(Tool::SevenZip)
        .or_else(|| {
            on_path("unrar")
                .or_else(|| installed("WinRAR/UnRAR.exe"))
                .map(Tool::UnRar)
        })
}

fn unpack_with(tool: &Tool, src: &Path, dest: &Path) -> Result<usize> {
    std::fs::create_dir_all(dest).map_err(io_err(dest))?;
    let (exe, mut cmd) = match tool {
        Tool::SevenZip(exe) => {
            let mut c = std::process::Command::new(exe);
            c.arg("x")
                .arg("-y")
                .arg(format!("-o{}", dest.display()))
                .arg("--")
                .arg(src);
            (exe, c)
        }
        Tool::UnRar(exe) => {
            let mut c = std::process::Command::new(exe);
            let into = format!("{}{}", dest.display(), std::path::MAIN_SEPARATOR);
            c.arg("x").arg("-y").arg("--").arg(src).arg(into);
            (exe, c)
        }
    };
    cmd.stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().map_err(io_err(exe))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        let last = text
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        return Err(err(format!(
            "{} could not unpack {}: {last}",
            file_name(exe),
            file_name(src)
        )));
    }
    // The tools keep entries inside the folder. Links are left out, as they are
    // for folders.
    let mut refused = 0;
    for path in files(dest)? {
        if std::fs::symlink_metadata(&path)
            .map_err(io_err(&path))?
            .is_symlink()
        {
            std::fs::remove_file(&path).map_err(io_err(&path))?;
            refused += 1;
        }
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

// A mod can come in parts: folders that each hold game files of their own, such as
// "Broadcast graphics/shader_package_2025" beside "Tyre colours/shader_package_2025".
// A part is the folder above a game folder, or above a folder named like the game,
// unless it sits inside another part's game files. Ported from list_mods.py of the
// Simplified Launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartDir {
    pub rel: String,
    pub name: String,
    pub segments: Vec<String>,
}

pub fn parts(root: &Path) -> Result<Vec<PartDir>> {
    let mut candidates = std::collections::BTreeSet::new();
    for file in files(root)? {
        let rel = file
            .strip_prefix(root)
            .expect("under root")
            .to_string_lossy()
            .replace('\\', "/");
        let segs: Vec<&str> = rel.split('/').collect();
        let dirs = &segs[..segs.len() - 1];
        if let Some(i) = dirs.iter().position(|d| is_game_folder(d) || is_marker(d)) {
            candidates.insert(dirs[..i].join("/"));
        }
    }
    let mut sorted: Vec<String> = candidates.into_iter().collect();
    sorted.sort_by_key(|c| (c.split('/').filter(|s| !s.is_empty()).count(), c.clone()));
    let mut kept: Vec<String> = Vec::new();
    for c in sorted {
        let inside = kept.iter().any(|k| {
            let rest = match k.is_empty() {
                true => Some(c.as_str()),
                false => c.strip_prefix(&format!("{k}/")),
            };
            rest.is_some_and(|r| {
                let first = r.split('/').next().unwrap_or("");
                is_game_folder(first) || is_marker(first)
            })
        });
        if !inside {
            kept.push(c);
        }
    }
    Ok(kept
        .into_iter()
        .map(|rel| {
            let segments = part_segments(&rel);
            PartDir {
                name: segments.join(" › "),
                segments,
                rel,
            }
        })
        .collect())
}

// "Liveries -c/Ferrari -x" shows as "Liveries › Ferrari", as in the Simplified Launcher.
fn part_segments(rel: &str) -> Vec<String> {
    let mut segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    if segs.last().is_some_and(|s| is_game_folder(s)) {
        segs.pop();
    }
    segs.iter()
        .map(|s| {
            s.strip_suffix(" -c")
                .or_else(|| s.strip_suffix(" -x"))
                .unwrap_or(s)
                .to_owned()
        })
        .collect()
}

pub fn content_root(root: &Path) -> Option<PathBuf> {
    named_game(root).or_else(|| {
        std::iter::once(root.to_path_buf())
            .chain(walk(root))
            .find(|d| subdirs(d).iter().any(|c| is_marker(&name(c))))
    })
}

fn named_game(root: &Path) -> Option<PathBuf> {
    walk(root).find(|d| is_game_folder(&name(d)))
}

// Mods name the game folder "F1 25", or "F1 25 (mod name)" and the like.
fn is_game_folder(name: &str) -> bool {
    name.to_lowercase().starts_with(&GAME_FOLDER.to_lowercase())
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
        std::fs::create_dir_all(dir.join("wrap/F1 25 (my mod)/game_package")).unwrap();
        assert_eq!(content_root(&dir), Some(dir.join("wrap/F1 25 (my mod)")));
        assert_eq!(claims(&dir), Claim::Yes);
        assert_eq!(claims(&dir.join("wrap")), Claim::Yes);
        let other = dir.join("photos/holiday");
        std::fs::create_dir_all(&other).unwrap();
        assert_eq!(claims(&dir.join("photos")), Claim::No);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn zip_of(path: &Path, names: &[&str]) {
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for n in names {
            w.start_file(*n, zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut w, b"x").unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn any_archive_laid_out_like_the_game_is_a_mod() {
        let dir = std::env::temp_dir().join(format!("simplified-claims-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let cases: [(&str, &[&str], Claim); 6] = [
            ("top.zip", &["2025_asset_groups/ui_package/a.erp"], Claim::Yes),
            (
                "wrapped.zip",
                &["My Mod/v2/shader_package_2025/b.erp"],
                Claim::Yes,
            ),
            ("named.zip", &["F1 25 - My Mod/readme.txt"], Claim::Yes),
            ("studio.zip", &["Mod/mod.serps", "Mod/notes.txt"], Claim::Yes),
            ("photos.zip", &["holiday/beach.png"], Claim::No),
            ("deep.zip", &["a/b/c/d/2025_asset_groups/x.erp"], Claim::No),
        ];
        for (name, entries, want) in cases {
            let path = dir.join(name);
            zip_of(&path, entries);
            assert_eq!(claims(&path), want, "{name}");
        }
        std::fs::write(dir.join("mod.7z"), b"").unwrap();
        std::fs::write(dir.join("mod.rar"), b"").unwrap();
        assert_eq!(claims(&dir.join("mod.7z")), Claim::Yes);
        assert_eq!(claims(&dir.join("mod.rar")), Claim::Yes);
        assert_eq!(claims(&dir.join("notes.txt")), Claim::No);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn tree(dir: &Path, files: &[&str]) {
        std::fs::remove_dir_all(dir).ok();
        for f in files {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
    }

    #[test]
    fn a_mod_comes_in_the_parts_that_hold_game_files() {
        let dir = std::env::temp_dir().join(format!("simplified-parts-{}", std::process::id()));
        let names = |files: &[&str]| {
            tree(&dir, files);
            parts(&dir)
                .unwrap()
                .into_iter()
                .map(|p| (p.rel, p.name))
                .collect::<Vec<_>>()
        };
        let part = |rel: &str, name: &str| (rel.to_owned(), name.to_owned());
        assert_eq!(
            names(&[
                "mod.serps",
                "2026 Broadcast graphics/shader_package_2025/ui_texture.nefx2.sm51.erp",
                "Realistic Tyre colours/shader_package_2025/object_steering_screen.nefx2.sm51.erp",
            ]),
            [
                part("2026 Broadcast graphics", "2026 Broadcast graphics"),
                part("Realistic Tyre colours", "Realistic Tyre colours"),
            ]
        );
        assert_eq!(
            names(&["F1 25/game_package/db.bin", "F1 25/2025_asset_groups/a.erp"]),
            [part("", "")]
        );
        assert_eq!(
            names(&["Wrap/v2/2025_asset_groups/a.erp"]),
            [part("Wrap/v2", "Wrap › v2")]
        );
        assert_eq!(
            names(&[
                "Pack/F1 25/game_package/db.bin",
                "Pack/UI/F1 25/shader_package_2025/ui.erp"
            ]),
            [part("Pack", "Pack"), part("Pack/UI", "Pack › UI")]
        );
        assert_eq!(
            names(&["Liveries -c/Ferrari -x/F1 25 (red)/2025_asset_groups/a.erp"]),
            [part("Liveries -c/Ferrari -x", "Liveries › Ferrari")]
        );
        assert_eq!(names(&["readme.txt"]), []);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_game_and_anticheat_are_never_replaced() {
        assert_eq!(class("F1_25.exe", None), None);
        assert_eq!(class("EAAntiCheat.GameServiceLauncher.dll", None), None);
        assert_eq!(class("EAAntiCheat.cfg", None), None);
        assert_eq!(class("bin/plugin.asi", None), None);
        assert_eq!(class("EAAntiCheat.splash.png", None), Some(Class::Replacement));
    }

    #[test]
    fn a_rar_unpacks_whatever_its_version() {
        let dir = std::env::temp_dir().join(format!("simplified-rar-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        for (name, version) in [
            ("rar5.rar", rars::ArchiveVersion::Rar50),
            ("rar29.rar", rars::ArchiveVersion::Rar29),
        ] {
            let mut b = rars::Builder::new(version);
            b.add_bytes(
                b"My Mod/2025_asset_groups/ui_package/a.erp".to_vec(),
                b"erp".to_vec(),
                None,
                None,
            )
            .unwrap();
            let archive = dir.join(name);
            std::fs::write(&archive, b.to_bytes().unwrap()).unwrap();
            assert_eq!(claims(&archive), Claim::Yes);
            let out = dir.join(format!("out-{name}"));
            assert_eq!(unpack(&archive, &out, &mut |_| {}).unwrap(), 0, "{name}");
            assert_eq!(
                std::fs::read(out.join("My Mod/2025_asset_groups/ui_package/a.erp")).unwrap(),
                b"erp",
                "{name}"
            );
            assert_eq!(content_root(&out), Some(out.join("My Mod")), "{name}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_zip_packed_with_bzip2_unpacks() {
        let dir = std::env::temp_dir().join(format!("simplified-bzip2-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("mod.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        for method in [zip::CompressionMethod::Bzip2, zip::CompressionMethod::Zstd] {
            let options = zip::write::SimpleFileOptions::default().compression_method(method);
            w.start_file(format!("F1 25/game_package/{method:?}.bin"), options)
                .unwrap();
            std::io::Write::write_all(&mut w, b"packed").unwrap();
        }
        w.finish().unwrap();
        let out = dir.join("out");
        unpack(&archive, &out, &mut |_| {}).unwrap();
        for method in ["Bzip2", "Zstd"] {
            assert_eq!(
                std::fs::read(out.join(format!("F1 25/game_package/{method}.bin"))).unwrap(),
                b"packed"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_7z_unpacks_and_unsafe_paths_stay_out() {
        let dir = std::env::temp_dir().join(format!("simplified-7z-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("mod.7z");
        let mut w = sevenz_rust2::ArchiveWriter::create(&archive).unwrap();
        for (name, data) in [
            ("My Mod/2025_asset_groups/ui_package/a.erp", &b"erp"[..]),
            ("../escape.txt", &b"no"[..]),
        ] {
            w.push_archive_entry(sevenz_rust2::ArchiveEntry::new_file(name), Some(data))
                .unwrap();
        }
        w.finish().unwrap();
        let out = dir.join("out");
        let refused = unpack(&archive, &out, &mut |_| {}).unwrap();
        assert_eq!(refused, 1);
        assert_eq!(
            std::fs::read(out.join("My Mod/2025_asset_groups/ui_package/a.erp")).unwrap(),
            b"erp"
        );
        assert!(!dir.join("escape.txt").exists());
        assert_eq!(content_root(&out), Some(out.join("My Mod")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
