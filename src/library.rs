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
pub struct Imported {
    pub package: Package,
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
        if self.data.starts_with(src) {
            return Err(err(format!("{} holds the mod library", src.display())));
        }
        std::fs::create_dir_all(self.root()).map_err(io_err(self.root()))?;
        sweep(&self.data);
        sweep(&self.root());
        let staging = free(&self.data, ".import-");
        let result = self.stage(src, &staging, game, progress);
        std::fs::remove_dir_all(&staging).ok();
        result
    }

    fn stage(
        &self,
        src: &Path,
        staging: &Path,
        game: Option<&dyn GameFiles>,
        progress: &mut dyn FnMut(f32, &str),
    ) -> Result<Imported> {
        let source = package::file_name(src);
        let unpacked = staging.join("unpacked");
        let refused = package::unpack(src, &unpacked, &mut |done| progress(done * 0.7, &source))?;
        if let Some((dir, info)) = track::find(&unpacked)? {
            progress(0.85, &source);
            let out = staging.join("package");
            let id = track::library_id(&self.root(), &info.code);
            let package = track::store(&dir, &info, &id, source, &out)?;
            self.place(&package, &out)?;
            progress(1.0, &package.name);
            return Ok(Imported { package, refused });
        }
        let root = package::package_root(&unpacked).unwrap_or_else(|| unpacked.clone());
        let serps_path = root.join(serps::FILE);
        let info = match serps_path.is_file() {
            true => ModInfo::parse(&package::read_file(&serps_path)?).unwrap_or_default(),
            false => ModInfo::default(),
        };
        let content = package::content_root(&root).ok_or_else(|| {
            err(format!(
                "{source} holds no game files: no {} folder, and no game folders such as \
                     2025_asset_groups or shader_package_2025",
                package::GAME_FOLDER
            ))
        })?;
        progress(0.75, &source);
        let out = staging.join("package");
        let files_dir = out.join(FILES);
        let mut files = Vec::new();
        let mut skipped = Vec::new();
        for path in package::files(&content)? {
            let rel = path
                .strip_prefix(&content)
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
        progress(0.85, &source);
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let package = Package {
            id: id(&info, &stem),
            name: info.title().unwrap_or(&stem).to_owned(),
            author: info.author().map(str::to_owned),
            version: info.version().map(str::to_owned),
            game_version: info.game_version().map(str::to_owned),
            studio_version: info.studio_version().map(str::to_owned),
            source,
            sha256: src.is_file().then(|| sha256(src)).transpose()?,
            imported: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            files,
            skipped,
            track: None,
        };
        if serps_path.is_file() {
            std::fs::copy(&serps_path, out.join(serps::FILE)).map_err(io_err(&out))?;
        }
        let json = serde_json::to_vec_pretty(&package).expect("serialisable");
        let path = out.join(PACKAGE_FILE);
        std::fs::write(&path, json).map_err(io_err(&path))?;
        self.place(&package, &out)?;
        progress(1.0, &package.name);
        Ok(Imported { package, refused })
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
