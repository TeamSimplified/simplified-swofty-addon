use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use swofty_addon::asset::{AssetOp, Root, Source, TrackOp};
use swofty_addon::launcher::Claim;

use crate::library::{File, LIBRARY, PACKAGE_FILE, Package};
use crate::package::{self, Class};
use crate::{ID, Result, Slot, State, err, io_err, serps};

pub const TRACK: &str = "track";
pub const MANIFEST: &str = "manifest.json";
const ADDON_MANIFEST: &str = "swofty.json";

#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub code: String,
    pub name: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
}

impl Info {
    pub fn read(dir: &Path) -> Option<Self> {
        let v: Value = serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).ok()?).ok()?;
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        let code = text("track_code").filter(|c| {
            !c.is_empty()
                && c.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })?;
        Some(Self {
            code,
            name: text("track_name"),
            version: text("version"),
            author: text("author_name"),
        })
    }
}

pub fn claims(path: &Path) -> Claim {
    let zip = path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    if !zip {
        return Claim::No;
    }
    let Some(names) = zip_names(path) else {
        return Claim::No;
    };
    let nested = names
        .iter()
        .filter(|n| n.to_ascii_lowercase().ends_with(".zip"))
        .count();
    let other = names.iter().any(|n| {
        n == ADDON_MANIFEST
            || n.ends_with(&format!("/{ADDON_MANIFEST}"))
            || n.ends_with(serps::FILE)
            || n.starts_with(&format!("{}/", package::GAME_FOLDER))
    });
    let yes = names.iter().any(|n| n == MANIFEST) || (nested == 1 && !other);
    if yes { Claim::Yes } else { Claim::No }
}

fn zip_names(path: &Path) -> Option<Vec<String>> {
    let zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    zip.file_names()
        .map(|n| n.ok().map(|n| n.replace('\\', "/")))
        .collect()
}

pub fn find(dir: &Path) -> Result<Option<(PathBuf, Info)>> {
    if let Some(info) = Info::read(dir) {
        return Ok(Some((dir.to_path_buf(), info)));
    }
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(io_err(dir))?
        .filter_map(|e| Some(e.ok()?.path()))
        .collect();
    let zips: Vec<&PathBuf> = entries
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
        .collect();
    match (entries.as_slice(), zips.as_slice()) {
        ([only], _) if only.is_dir() => find(only),
        (_, [inner]) => {
            let to = dir.join(".inner");
            package::unpack(inner, &to, &mut |_| {})?;
            Ok(Info::read(&to).map(|info| (to, info)))
        }
        _ => Ok(None),
    }
}

pub fn store(from: &Path, info: &Info, source: String, out: &Path) -> Result<Package> {
    let track = out.join(TRACK);
    std::fs::create_dir_all(out).map_err(io_err(out))?;
    std::fs::rename(from, &track).map_err(io_err(&track))?;
    let mut files = Vec::new();
    for path in package::files(&track)? {
        let rel = path.strip_prefix(out).expect("under the package");
        files.push(File {
            path: rel.to_string_lossy().replace('\\', "/"),
            class: Class::Addition,
            size: std::fs::metadata(&path).map_err(io_err(&path))?.len(),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let package = Package {
        id: info.code.clone(),
        name: info.name.clone().unwrap_or_else(|| info.code.clone()),
        author: info.author.clone(),
        version: info.version.clone(),
        game_version: None,
        studio_version: None,
        source,
        sha256: None,
        imported: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        files,
        skipped: Vec::new(),
        track: Some(info.code.clone()),
    };
    let mut serps = serde_json::Map::new();
    serps.insert("mod_name".into(), package.name.clone().into());
    if let Some(a) = &package.author {
        serps.insert("author".into(), a.clone().into());
    }
    if let Some(v) = &package.version {
        serps.insert("version".into(), v.clone().into());
    }
    write(&out.join(serps::FILE), &Value::Object(serps))?;
    write(
        &out.join(PACKAGE_FILE),
        &serde_json::to_value(&package).expect("serialisable"),
    )?;
    Ok(package)
}

fn write(path: &Path, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).expect("serialisable");
    std::fs::write(path, bytes).map_err(io_err(path))
}

pub fn op(p: &Package) -> AssetOp {
    AssetOp::Track(TrackOp::Add {
        source: Source {
            root: Root::Data,
            path: format!("{LIBRARY}/{}/{TRACK}", p.id),
        },
    })
}

pub fn migrate(data: &Path) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    let maps = data.join("maps");
    let lib = data.join("addon-data").join(ID).join(LIBRARY);
    let mut moved = Vec::new();
    for entry in std::fs::read_dir(&maps).into_iter().flatten().flatten() {
        let dir = entry.path();
        let Some(info) = Info::read(&dir) else {
            continue;
        };
        let dest = lib.join(&info.code);
        if dest.exists() {
            notes.push(format!(
                "kept {}: the library already has {}",
                dir.display(),
                info.code
            ));
            continue;
        }
        std::fs::create_dir_all(&lib).map_err(io_err(&lib))?;
        let staging = lib.join(format!(".import-{}", info.code));
        if staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(io_err(&staging))?;
        }
        std::fs::remove_dir_all(dir.join(".art")).ok();
        store(&dir, &info, package::file_name(&dir), &staging)?;
        std::fs::rename(&staging, &dest).map_err(io_err(&dest))?;
        notes.push(format!("moved {} to {}", dir.display(), dest.display()));
        moved.push(info.code);
    }
    if std::fs::remove_dir(&maps).is_ok() {
        notes.push(format!("removed the empty {}", maps.display()));
    }
    let profiles = data.join("profiles");
    for entry in std::fs::read_dir(&profiles).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let mut doc: Value = serde_json::from_slice(&package::read_file(&path)?)
            .map_err(|e| err(format!("{}: {e}", path.display())))?;
        let Some(obj) = doc.as_object_mut() else {
            continue;
        };
        let Some(codes) = obj.remove("maps") else {
            continue;
        };
        let codes: Vec<String> = serde_json::from_value(codes).unwrap_or_default();
        let state = obj.entry("state").or_insert_with(|| json!({}));
        let mut ours: State = state
            .get(ID)
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| err(format!("{}: {e}", path.display())))?
            .unwrap_or_default();
        for code in &codes {
            ours.mods.retain(|s| &s.id != code);
            ours.mods.push(Slot {
                id: code.clone(),
                enabled: true,
            });
        }
        state[ID] = serde_json::to_value(&ours).expect("serialisable");
        write(&path, &doc)?;
        if !codes.is_empty() {
            notes.push(format!("{} keeps {} track(s) on", path.display(), codes.len()));
        }
    }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;

    const MANIFEST_JSON: &str = r#"{"track_code":"testring","track_name":"Test Ring","version":"1.0","author_name":"a","erp":"testring.erp"}"#;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("swofty-track-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn zip(path: &Path, files: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut z, data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn a_track_zip_becomes_a_library_package() {
        let root = tmp("import");
        let inner = root.join("testring.zip");
        zip(
            &inner,
            &[(MANIFEST, MANIFEST_JSON.as_bytes()), ("testring.erp", b"erp")],
        );
        let outer = root.join("download.zip");
        zip(
            &outer,
            &[
                ("readme.txt", b"x"),
                ("testring.zip", &std::fs::read(&inner).unwrap()),
            ],
        );
        assert_eq!(claims(&inner), Claim::Yes);
        assert_eq!(claims(&outer), Claim::Yes);
        let library = Library::new(&root.join("data"));
        let imported = library.import(&outer, None, &mut |_, _| {}).unwrap();
        let p = imported.package;
        assert_eq!((p.id.as_str(), p.name.as_str()), ("testring", "Test Ring"));
        assert_eq!(p.track.as_deref(), Some("testring"));
        let dir = library.dir("testring");
        assert!(dir.join("track/testring.erp").is_file());
        assert!(dir.join(serps::FILE).is_file());
        assert_eq!(library.list().unwrap(), std::slice::from_ref(&p));
        assert_eq!(
            serde_json::to_value(op(&p)).unwrap(),
            json!({"op": "track.add", "source": {"root": "data", "path": "library/testring/track"}})
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn maps_and_profile_maps_move_into_the_library() {
        let data = tmp("migrate");
        let map = data.join("maps/testring");
        std::fs::create_dir_all(map.join(".art")).unwrap();
        std::fs::write(map.join(MANIFEST), MANIFEST_JSON).unwrap();
        std::fs::write(map.join("testring.erp"), b"erp").unwrap();
        std::fs::create_dir_all(data.join("profiles")).unwrap();
        let profile = data.join("profiles/default.json");
        std::fs::write(
            &profile,
            r#"{"schema_version":2,"name":"default","addons":[],"maps":["testring"],
                "state":{"teamsimplified.mods":{"mods":[{"id":"rm.at","enabled":true}]}}}"#,
        )
        .unwrap();
        let other = data.join("profiles/other.json");
        std::fs::write(&other, r#"{"schema_version":2,"name":"other","maps":[]}"#).unwrap();
        let notes = migrate(&data).unwrap();
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(!data.join("maps").exists());
        let library = Library::new(&data.join("addon-data").join(ID));
        let p = library.get("testring").unwrap();
        assert_eq!(p.track.as_deref(), Some("testring"));
        assert!(library.dir("testring").join("track/testring.erp").is_file());
        assert!(!library.dir("testring").join("track/.art").exists());
        let doc: Value = serde_json::from_slice(&std::fs::read(&profile).unwrap()).unwrap();
        assert!(doc.get("maps").is_none());
        assert_eq!(
            doc["state"][ID]["mods"],
            json!([{"id": "rm.at", "enabled": true}, {"id": "testring", "enabled": true}])
        );
        let doc: Value = serde_json::from_slice(&std::fs::read(&other).unwrap()).unwrap();
        assert_eq!(
            doc,
            json!({"schema_version": 2, "name": "other", "state": {ID: {"mods": []}}})
        );
        assert!(migrate(&data).unwrap().is_empty());
        std::fs::remove_dir_all(&data).ok();
    }
}
