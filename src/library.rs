use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use swofty_addon::launcher::GameFiles;
use swofty_formats::gvdb::Gvdb;

use crate::diff::{self, Diff};
use crate::package::{self, Class};
use crate::serps::{self, ModInfo};
use crate::track;
use crate::{Result, err, io_err};

pub const PACKAGE_FILE: &str = "package.json";
pub const LIBRARY: &str = "library";
pub const FILES: &str = "files";
pub const DIFF: &str = "diff";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Package {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub game_version: Option<String>,
    #[serde(default)]
    pub studio_version: Option<String>,
    pub source: String,
    #[serde(default)]
    pub sha256: Option<String>,
    pub imported: u64,
    pub files: Vec<File>,
    #[serde(default)]
    pub skipped: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    // A part of a mod that comes in parts: the mod's id and the part's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<Part>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub of: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct File {
    pub path: String,
    pub class: Class,
    pub size: u64,
}

impl Package {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn count(&self, class: Class) -> usize {
        self.files.iter().filter(|f| f.class == class).count()
    }
}

#[derive(Debug)]
// Every part of what was imported, one package each; a mod without parts gives one.
pub struct Imported {
    pub packages: Vec<Package>,
    pub refused: usize,
}

pub struct Library {
    data: PathBuf,
}

impl Library {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            data: data_dir.to_path_buf(),
        }
    }

    pub fn root(&self) -> PathBuf {
        self.data.join(LIBRARY)
    }

    pub fn dir(&self, id: &str) -> PathBuf {
        self.root().join(id)
    }

    pub fn list(&self) -> Result<Vec<Package>> {
        let root = self.root();
        let rd = match std::fs::read_dir(&root) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_err(&root)(e)),
        };
        let mut out = Vec::new();
        for entry in rd.flatten() {
            let path = entry.path().join(PACKAGE_FILE);
            if entry.file_name().to_string_lossy().starts_with('.') || !path.is_file() {
                continue;
            }
            out.push(read_package(&path)?);
        }
        out.sort_by_key(|p| p.name.to_lowercase());
        Ok(out)
    }

    pub fn get(&self, id: &str) -> Result<Package> {
        let path = self.dir(id).join(PACKAGE_FILE);
        if !path.is_file() {
            return Err(err(format!("no mod {id} in the library")));
        }
        read_package(&path)
    }

    fn diff_path(&self, id: &str, key: &str, ext: &str) -> PathBuf {
        self.dir(id).join(DIFF).join(format!("{key}.{ext}"))
    }

    pub fn diff_file(&self, id: &str, key: &str) -> PathBuf {
        self.diff_path(id, key, "json")
    }

    pub fn cached(&self, id: &str, key: &str) -> Option<Diff> {
        let bytes = std::fs::read(self.diff_path(id, key, "json")).ok()?;
        serde_json::from_slice::<Diff>(&bytes)
            .ok()
            .filter(|d| d.build_key == key && d.version == diff::VERSION)
    }

    pub fn diff(&self, p: &Package, game: &dyn GameFiles) -> Result<Diff> {
        let key = game.build_key();
        if p.track.is_some() {
            return Ok(Diff {
                build_key: key,
                ..Diff::default()
            });
        }
        if let Some(diff) = self.cached(&p.id, &key) {
            return Ok(diff);
        }
        let (diff, records) = diff::compute(p, &self.dir(&p.id).join(FILES), game)?;
        let dir = self.dir(&p.id).join(DIFF);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir).map_err(io_err(&dir))?;
        }
        std::fs::create_dir_all(&dir).map_err(io_err(&dir))?;
        let db = self.diff_path(&p.id, &key, "db");
        std::fs::write(&db, records.to_bytes_unsorted()).map_err(io_err(&db))?;
        let json = self.diff_path(&p.id, &key, "json");
        let tmp = json.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&diff).expect("serialisable"))
            .map_err(io_err(&tmp))?;
        std::fs::rename(&tmp, &json).map_err(io_err(&json))?;
        Ok(diff)
    }

    pub fn records(&self, id: &str, key: &str) -> Result<Gvdb> {
        let path = self.diff_path(id, key, "db");
        Gvdb::parse(&package::read_file(&path)?).map_err(|e| err(format!("{}: {e}", path.display())))
    }

    pub fn remove(&self, id: &str) -> Result<Package> {
        let package = self.get(id)?;
        let aside = self.root().join(format!(".removed-{id}"));
        std::fs::rename(self.dir(id), &aside).map_err(io_err(&aside))?;
        std::fs::remove_dir_all(&aside).ok();
        Ok(package)
    }

    pub fn import(
        &self,
        src: &Path,
        game: Option<&dyn GameFiles>,
        progress: &mut dyn FnMut(f32, &str),
    ) -> Result<Imported> {
        self.import_as(src, game, progress, false)
    }

    pub fn import_track(&self, src: &Path, progress: &mut dyn FnMut(f32, &str)) -> Result<Package> {
        let mut imported = self.import_as(src, None, progress, true)?;
        Ok(imported.packages.remove(0))
    }

    fn import_as(
        &self,
        src: &Path,
        game: Option<&dyn GameFiles>,
        progress: &mut dyn FnMut(f32, &str),
        only_tracks: bool,
    ) -> Result<Imported> {
        if self.data.starts_with(src) {
            return Err(err(format!("{} holds the mod library", src.display())));
        }
        std::fs::create_dir_all(self.root()).map_err(io_err(self.root()))?;
        sweep(&self.data);
        sweep(&self.root());
        let staging = free(&self.data, ".import-");
        let result = self.stage(src, &staging, game, progress, only_tracks);
        std::fs::remove_dir_all(&staging).ok();
        result
    }

    fn stage(
        &self,
        src: &Path,
        staging: &Path,
        game: Option<&dyn GameFiles>,
        progress: &mut dyn FnMut(f32, &str),
        only_tracks: bool,
    ) -> Result<Imported> {
        let source = package::file_name(src);
        let sha256 = src.is_file().then(|| sha256(src)).transpose()?;
        let unpacked = staging.join("unpacked");
        let refused = package::unpack(src, &unpacked, &mut |done| progress(done * 0.7, &source))?;
        if let Some((dir, info)) = track::find(&unpacked)? {
            progress(0.85, &source);
            let out = staging.join("package");
            let id = track::library_id(&self.root(), &info.code);
            let package = track::store(&dir, &info, &id, source, sha256, &out)?;
            self.place(&package, &out)?;
            progress(1.0, &package.name);
            return Ok(Imported {
                packages: vec![package],
                refused,
            });
        }
        if only_tracks {
            return Err(err(format!(
                "{source} holds no track package: no {} with a track_code",
                track::MANIFEST
            )));
        }
        let root = package::package_root(&unpacked).unwrap_or_else(|| unpacked.clone());
        let serps_path = root.join(serps::FILE);
        let info = match serps_path.is_file() {
            true => ModInfo::parse(&package::read_file(&serps_path)?).unwrap_or_default(),
            false => ModInfo::default(),
        };
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let label = match src.is_dir() {
            true => package::file_name(src),
            false => stem.clone(),
        };
        let (plain, stem_version) = plain_name(&label);
        let (plain, by) = byline(plain);
        let base = Package {
            id: id(&info, &stem),
            name: info.title().unwrap_or(plain).to_owned(),
            author: info.author().or(by).map(str::to_owned),
            version: info.version().or(stem_version).map(str::to_owned),
            game_version: info.game_version().map(str::to_owned),
            studio_version: info.studio_version().map(str::to_owned),
            source: source.clone(),
            sha256,
            imported: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            files: Vec::new(),
            skipped: Vec::new(),
            track: None,
            part: None,
        };
        let parts = package::parts(&root)?;
        let mut packages = Vec::new();
        if parts.len() < 2 {
            let content = package::content_root(&root).ok_or_else(|| {
                err(format!(
                    "{source} holds no game files: no {} folder, and no game folders such as \
                     2025_asset_groups or shader_package_2025",
                    package::GAME_FOLDER
                ))
            })?;
            packages.push((base.clone(), content));
        } else {
            for (part, named) in parts.iter().zip(named_parts(&base, &label, &parts)) {
                let dir = root.join(&part.rel);
                let content = package::content_root(&dir).unwrap_or(dir);
                let legacy = format!("{}.{}", base.id, slug(&part.rel));
                let known = |id: &str| self.dir(id).join(PACKAGE_FILE).is_file();
                let id = match !known(&named.id) && known(&legacy) {
                    true => legacy,
                    false => named.id,
                };
                let p = Package {
                    id,
                    name: named.name,
                    part: Some(Part {
                        of: base.id.clone(),
                        name: named.part,
                    }),
                    ..base.clone()
                };
                packages.push((p, content));
            }
        }
        progress(0.75, &source);
        let mut placed = Vec::new();
        for (n, (mut p, content)) in packages.into_iter().enumerate() {
            let out = staging.join(format!("package-{n}"));
            let (files, skipped) = gather(&content, &out.join(FILES), game, &source)?;
            p.files = files;
            p.skipped = skipped;
            if serps_path.is_file() {
                std::fs::copy(&serps_path, out.join(serps::FILE)).map_err(io_err(&out))?;
            }
            let json = serde_json::to_vec_pretty(&p).expect("serialisable");
            let path = out.join(PACKAGE_FILE);
            std::fs::write(&path, json).map_err(io_err(&path))?;
            placed.push((p, out));
        }
        for (p, out) in &placed {
            self.place(p, out)?;
        }
        let packages: Vec<Package> = placed.into_iter().map(|(p, _)| p).collect();
        progress(1.0, &packages[0].name);
        Ok(Imported { packages, refused })
    }

    fn place(&self, package: &Package, out: &Path) -> Result<()> {
        let dest = self.dir(&package.id);
        let aside = dest.exists().then(|| free(&self.root(), ".replaced-"));
        if let Some(aside) = &aside {
            std::fs::rename(&dest, aside).map_err(io_err(aside))?;
        }
        if let Err(e) = std::fs::rename(out, &dest) {
            if let Some(aside) = &aside {
                std::fs::rename(aside, &dest).ok();
            }
            return Err(io_err(&dest)(e));
        }
        if let Some(aside) = aside {
            std::fs::remove_dir_all(aside).ok();
        }
        Ok(())
    }
}

fn gather(
    content: &Path,
    files_dir: &Path,
    game: Option<&dyn GameFiles>,
    source: &str,
) -> Result<(Vec<File>, Vec<String>)> {
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    for path in package::files(content)? {
        let rel = path
            .strip_prefix(content)
            .expect("under content")
            .to_string_lossy()
            .replace('\\', "/");
        let Some(class) = package::class(&rel, game) else {
            skipped.push(rel);
            continue;
        };
        if class == Class::Database {
            Gvdb::parse(&package::read_file(&path)?)
                .map_err(|e| err(format!("{source}: {rel} is not an F1 25 database: {e}")))?;
        }
        let rel = package::game_path(&rel);
        let dest = files_dir.join(&rel);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        std::fs::rename(&path, &dest).map_err(io_err(&dest))?;
        let size = std::fs::metadata(&dest).map_err(io_err(&dest))?.len();
        files.push(File {
            path: rel,
            class,
            size,
        });
    }
    if files.is_empty() {
        return Err(err(format!("{source} holds no game files")));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((files, skipped))
}

fn read_package(path: &Path) -> Result<Package> {
    serde_json::from_slice(&package::read_file(path)?).map_err(|e| err(format!("{}: {e}", path.display())))
}

pub fn id(info: &ModInfo, fallback: &str) -> String {
    let parts: Vec<String> = [info.author(), info.title()]
        .into_iter()
        .flatten()
        .map(slug)
        .filter(|s| !s.is_empty())
        .collect();
    match (parts.is_empty(), slug(fallback)) {
        (false, _) => parts.join("."),
        (true, s) if !s.is_empty() => s,
        _ => "mod".into(),
    }
}

const PART_SLUG: usize = 32;

struct Named {
    id: String,
    name: String,
    part: String,
}

// A part is named by the folders that tell it apart, the innermost first, then the
// mod: folders every part sits in and folders named like the mod say nothing.
fn named_parts(base: &Package, stem: &str, parts: &[package::PartDir]) -> Vec<Named> {
    let common = parts
        .iter()
        .map(|p| p.segments.as_slice())
        .reduce(|a, b| &a[..a.iter().zip(b).take_while(|(x, y)| x == y).count()])
        .map_or(0, <[String]>::len);
    let mut named: Vec<Named> = parts
        .iter()
        .map(|p| {
            let own: Vec<&str> = p.segments[common.min(p.segments.len())..]
                .iter()
                .map(String::as_str)
                .filter(|s| !same(s, &base.name) && !same(s, stem))
                .rev()
                .collect();
            let own = match own.is_empty() {
                true => p.segments.iter().rev().map(String::as_str).take(1).collect(),
                false => own,
            };
            let part = own.join(" · ");
            Named {
                id: format!("{}.{}", base.id, capped(&slug(&own.join(" ")), &p.rel, PART_SLUG)),
                name: match part.is_empty() {
                    true => base.name.clone(),
                    false => format!("{part} — {}", base.name),
                },
                part,
            }
        })
        .collect();
    let ids: Vec<String> = named.iter().map(|n| n.id.clone()).collect();
    for (n, p) in named.iter_mut().zip(parts) {
        if ids.iter().filter(|id| **id == n.id).count() > 1 {
            n.id = format!("{}-{}", n.id, short_hash(&p.rel));
        }
    }
    named
}

fn same(a: &str, b: &str) -> bool {
    let key = |s: &str| -> String {
        plain_name(s)
            .0
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .flat_map(char::to_lowercase)
            .collect()
    };
    key(a) == key(b)
}

// "Pirelli Tyres Collection v1.0.6" is the mod "Pirelli Tyres Collection" at 1.0.6.
pub fn plain_name(name: &str) -> (&str, Option<&str>) {
    let name = name.trim();
    let Some((head, last)) = name.rsplit_once(char::is_whitespace) else {
        return (name, None);
    };
    let v = last.trim_matches(['(', ')', '[', ']']);
    let numeric = |s: &str| {
        s.split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    let version = match v.strip_prefix(['v', 'V']) {
        Some(bare) => numeric(bare),
        None => v.contains('.') && numeric(v),
    };
    match version {
        true => (head.trim_end_matches([' ', '-', '_']), Some(v)),
        false => (name, None),
    }
}

// "F1 25 with V10 Engines - By Jor" is the mod "F1 25 with V10 Engines" by Jor.
pub fn byline(name: &str) -> (&str, Option<&str>) {
    let Some(at) = name.to_ascii_lowercase().rfind(" by ") else {
        return (name, None);
    };
    let author = name[at + 4..].trim_matches(['(', ')', '[', ']', '-', ' ']);
    let head = name[..at].trim_end_matches(['(', '[', '-', '_', ' ']);
    match author.is_empty() || head.is_empty() || author.split_whitespace().count() > 3 {
        true => (name, None),
        false => (head, Some(author)),
    }
}

pub fn short_hash(key: &str) -> String {
    Sha256::digest(key.as_bytes())[..3]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// A slug longer than `max` keeps its first words and gets a hash of `key`, so it
// stays short and the same for the same `key`.
pub fn capped(slug: &str, key: &str, max: usize) -> String {
    if slug.len() <= max {
        return slug.to_owned();
    }
    let room = max - 7;
    let cut = slug[..=room].rfind(['-', '.']).unwrap_or(room);
    let head = slug[..cut].trim_end_matches(['-', '.']);
    match head.is_empty() {
        true => short_hash(key),
        false => format!("{head}-{}", short_hash(key)),
    }
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

fn sha256(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).map_err(io_err(path))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 1 << 20];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_err(path)(e)),
        }
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn free(dir: &Path, prefix: &str) -> PathBuf {
    (0..)
        .map(|n| dir.join(format!("{prefix}{n}")))
        .find(|p| !p.exists())
        .expect("a free name")
}

fn sweep(dir: &Path) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if [".import-", ".replaced-", ".removed-"]
            .iter()
            .any(|p| name.starts_with(p))
        {
            std::fs::remove_dir_all(entry.path()).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_part_of_a_mod_is_a_library_entry_of_its_own() {
        let root = std::env::temp_dir().join(format!("simplified-import-parts-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let src = root.join("Realism Patch");
        for (rel, data) in [
            (
                "mod.serps",
                &br#"{"mod_name":"Realism Patch","author":"Venturi"}"#[..],
            ),
            (
                "Broadcast -c/shader_package_2025/ui_texture.nefx2.sm51.erp",
                b"ui",
            ),
            ("Tyres/shader_package_2025/ui_texture.nefx2.sm51.erp", b"tyres"),
            (
                "Tyres/shader_package_2025/object_steering_screen.nefx2.sm51.erp",
                b"steering",
            ),
        ] {
            let p = src.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, data).unwrap();
        }
        let library = Library::new(&root.join("data"));
        let imported = library.import(&src, None, &mut |_, _| {}).unwrap();
        let got: Vec<(&str, &str, usize)> = imported
            .packages
            .iter()
            .map(|p| (p.id.as_str(), p.name.as_str(), p.files.len()))
            .collect();
        assert_eq!(
            got,
            [
                ("venturi.realism-patch.broadcast", "Broadcast — Realism Patch", 1),
                ("venturi.realism-patch.tyres", "Tyres — Realism Patch", 2),
            ]
        );
        let part = imported.packages[1].part.as_ref().unwrap();
        assert_eq!(
            (part.of.as_str(), part.name.as_str()),
            ("venturi.realism-patch", "Tyres")
        );
        let ui = |id: &str| {
            std::fs::read(
                library
                    .dir(id)
                    .join(FILES)
                    .join("shader_package_2025/ui_texture.nefx2.sm51.erp"),
            )
            .unwrap()
        };
        assert_eq!(ui("venturi.realism-patch.broadcast"), b"ui");
        assert_eq!(ui("venturi.realism-patch.tyres"), b"tyres");
        assert_eq!(library.list().unwrap().len(), 2);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn parts_are_named_by_what_tells_them_apart() {
        let root = std::env::temp_dir().join(format!("simplified-import-names-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let src = root.join("Pirelli Tyres Collection v1.0.6");
        for grip in ["Default Weather Grip Track", "Improved Weather Grip Track"] {
            for season in ["Pirelli Season 2011", "Pirelli Season 2026"] {
                let p = src
                    .join("Pirelli Tyres Collection v1.0.6")
                    .join(grip)
                    .join(season)
                    .join("F1 25/2025_asset_groups/f1_2026_vehicle_package/teams/common.erp");
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, season).unwrap();
            }
        }
        let library = Library::new(&root.join("data"));
        let imported = library.import(&src, None, &mut |_, _| {}).unwrap();
        let first = &imported.packages[0];
        assert_eq!(
            first.name,
            "Pirelli Season 2011 · Default Weather Grip Track — Pirelli Tyres Collection"
        );
        assert_eq!(first.version.as_deref(), Some("v1.0.6"));
        assert_eq!(
            first.part.as_ref().unwrap().name,
            "Pirelli Season 2011 · Default Weather Grip Track"
        );
        let ids: Vec<&str> = imported.packages.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.iter().all(|id| id.len() <= 64), "{ids:?}");
        let of = first.part.as_ref().unwrap().of.clone();
        assert!(
            ids[0].starts_with(&format!("{of}.pirelli-season-2011-")),
            "{ids:?}"
        );
        assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), 4);

        let legacy =
            format!("{of}.pirelli-tyres-collection-v1-0-6-default-weather-grip-track-pirelli-season-2011");
        std::fs::rename(library.dir(ids[0]), library.dir(&legacy)).unwrap();
        let again = library.import(&src, None, &mut |_, _| {}).unwrap();
        assert_eq!(
            again.packages[0].id, legacy,
            "a part imported before keeps its id"
        );
        assert_eq!(library.list().unwrap().len(), 4);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn versions_come_off_the_name() {
        assert_eq!(
            plain_name("Pirelli Tyres Collection v1.0.6"),
            ("Pirelli Tyres Collection", Some("v1.0.6"))
        );
        assert_eq!(
            plain_name("F1 25 with V10 Engines - By Jor (V10.3)"),
            ("F1 25 with V10 Engines - By Jor", Some("V10.3"))
        );
        assert_eq!(plain_name("Mod 1.2"), ("Mod", Some("1.2")));
        assert_eq!(plain_name("Season 2021"), ("Season 2021", None));
        assert_eq!(plain_name("V10"), ("V10", None));
        assert_eq!(
            byline("F1 25 with V10 Engines - By Jor"),
            ("F1 25 with V10 Engines", Some("Jor"))
        );
        let long = "Liveries made by the community of F1 modders";
        assert_eq!(byline(long), (long, None));
        assert_eq!(capped("short", "k", 32), "short");
        let long = capped("pirelli-season-2011-default-weather-grip-track", "k", 32);
        assert_eq!(long, format!("pirelli-season-2011-{}", short_hash("k")));
    }

    #[test]
    fn ids_come_from_author_and_name() {
        let info = ModInfo {
            name: Some("Scuderia AlphaTauri ORLEN".into()),
            author: Some("RM Designs".into()),
            ..ModInfo::default()
        };
        assert_eq!(id(&info, "x"), "rm-designs.scuderia-alphatauri-orlen");
        assert_eq!(id(&ModInfo::default(), "My Mod (v2)"), "my-mod-v2");
    }
}
