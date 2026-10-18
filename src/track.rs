use std::collections::BTreeMap;
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

pub fn store(from: &Path, info: &Info, id: &str, source: String, out: &Path) -> Result<Package> {
    let track = out.join(TRACK);
    std::fs::create_dir_all(out).map_err(io_err(out))?;
    if from != track {
        std::fs::rename(from, &track).map_err(io_err(&track))?;
    }
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
        id: id.to_owned(),
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
        part: None,
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

const MIGRATING: &str = ".migrate-";

pub fn migrate(data: &Path) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    let maps = data.join("maps");
    let lib = data.join("addon-data").join(ID).join(LIBRARY);
    let mut sources = Vec::new();
    for staging in entries(&lib) {
        let name = package::file_name(&staging);
        let Some(source) = name.strip_prefix(MIGRATING) else {
            continue;
        };
        if Info::read(&staging.join(TRACK)).is_some() {
            sources.push((staging.join(TRACK), source.to_owned()));
        } else {
            std::fs::remove_dir_all(&staging).map_err(io_err(&staging))?;
        }
    }
    sources.extend(entries(&maps).into_iter().map(|d| {
        let source = package::file_name(&d);
        (d, source)
    }));
    for (dir, source) in sources {
        let Some(info) = Info::read(&dir) else {
            continue;
        };
        if let Some(id) = tracks(&lib).remove(&info.code) {
            notes.push(format!(
                "kept {}: the library already has {} as {id}",
                dir.display(),
                info.code
            ));
            continue;
        }
        let id = free_id(&lib, &info.code);
        let staging = lib.join(format!("{MIGRATING}{source}"));
        if !dir.starts_with(&staging) && staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(io_err(&staging))?;
        }
        std::fs::remove_dir_all(dir.join(".art")).ok();
        store(&dir, &info, &id, source, &staging)?;
        let dest = lib.join(&id);
        std::fs::rename(&staging, &dest).map_err(io_err(&dest))?;
        notes.push(format!("moved {} to {}", dir.display(), dest.display()));
    }
    if std::fs::remove_dir(&maps).is_ok() {
        notes.push(format!("removed the empty {}", maps.display()));
    }
    let tracks = tracks(&lib);
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
        let mut kept = 0;
        for code in &codes {
            let Some(id) = tracks.get(code) else {
                notes.push(format!(
                    "{} drops track {code}: it is not in the library",
                    path.display()
                ));
                continue;
            };
            ours.mods.retain(|s| &s.id != id);
            ours.mods.push(Slot {
                id: id.clone(),
                enabled: true,
            });
            kept += 1;
        }
        state[ID] = serde_json::to_value(&ours).expect("serialisable");
        write(&path, &doc)?;
        if kept > 0 {
            notes.push(format!("{} keeps {kept} track(s) on", path.display()));
        }
    }
    Ok(notes)
}

pub fn library_id(lib: &Path, code: &str) -> String {
    tracks(lib).remove(code).unwrap_or_else(|| free_id(lib, code))
}

fn free_id(lib: &Path, code: &str) -> String {
    (0..)
        .map(|n| match n {
            0 => code.to_owned(),
            1 => format!("{code}-track"),
            n => format!("{code}-track-{n}"),
        })
        .find(|id| !lib.join(id).exists())
        .expect("a free id")
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

fn tracks(lib: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for dir in entries(lib) {
        let id = package::file_name(&dir);
        if id.starts_with('.') {
            continue;
        }
        let package = std::fs::read(dir.join(PACKAGE_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<Package>(&b).ok());
        if let Some(code) = package.and_then(|p| p.track) {
            out.entry(code).or_insert(id);
        }
    }
    out
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
        let mut imported = library.import(&outer, None, &mut |_, _| {}).unwrap();
        let p = imported.packages.remove(0);
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

    fn map_profile(data: &Path) -> PathBuf {
        let map = data.join("maps/testring");
        std::fs::create_dir_all(&map).unwrap();
        std::fs::write(map.join(MANIFEST), MANIFEST_JSON).unwrap();
        std::fs::write(map.join("testring.erp"), b"erp").unwrap();
        std::fs::create_dir_all(data.join("profiles")).unwrap();
        let profile = data.join("profiles/default.json");
        std::fs::write(
            &profile,
            r#"{"schema_version":2,"name":"default","maps":["testring"],"state":{}}"#,
        )
        .unwrap();
        profile
    }

    #[test]
    fn a_track_whose_code_is_taken_gets_its_own_id() {
        let data = tmp("migrate-taken");
        let profile = map_profile(&data);
        let library = Library::new(&data.join("addon-data").join(ID));
        let serp = library.dir("testring");
        std::fs::create_dir_all(&serp).unwrap();
        let held = Package {
            id: "testring".into(),
            name: "A livery".into(),
            author: None,
            version: None,
            game_version: None,
            studio_version: None,
            source: "livery.zip".into(),
            sha256: None,
            imported: 0,
            files: Vec::new(),
            skipped: Vec::new(),
            track: None,
            part: None,
        };
        write(&serp.join(PACKAGE_FILE), &serde_json::to_value(&held).unwrap()).unwrap();
        migrate(&data).unwrap();
        assert!(!data.join("maps").exists());
        assert_eq!(library.get("testring").unwrap(), held);
        let p = library.get("testring-track").unwrap();
        assert_eq!(
            (p.id.as_str(), p.track.as_deref()),
            ("testring-track", Some("testring"))
        );
        assert!(library.dir("testring-track").join("track/testring.erp").is_file());
        assert_eq!(
            serde_json::to_value(op(&p)).unwrap()["source"]["path"],
            "library/testring-track/track"
        );
        let doc: Value = serde_json::from_slice(&std::fs::read(&profile).unwrap()).unwrap();
        assert_eq!(
            doc["state"][ID]["mods"],
            json!([{"id": "testring-track", "enabled": true}])
        );
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn a_track_import_keeps_a_mod_with_its_code() {
        let root = tmp("import-taken");
        let library = Library::new(&root.join("data"));
        let mod_dir = root.join("livery");
        std::fs::create_dir_all(mod_dir.join("F1 25/2025_asset_groups")).unwrap();
        std::fs::write(mod_dir.join("F1 25/2025_asset_groups/livery.erp"), b"erp").unwrap();
        std::fs::write(mod_dir.join(serps::FILE), r#"{"mod_name":"testring"}"#).unwrap();
        let held = library
            .import(&mod_dir, None, &mut |_, _| {})
            .unwrap()
            .packages
            .remove(0);
        assert_eq!((held.id.as_str(), held.track.as_deref()), ("testring", None));
        let track = root.join("testring.zip");
        zip(
            &track,
            &[(MANIFEST, MANIFEST_JSON.as_bytes()), ("testring.erp", b"erp")],
        );
        let p = library
            .import(&track, None, &mut |_, _| {})
            .unwrap()
            .packages
            .remove(0);
        assert_eq!(
            (p.id.as_str(), p.track.as_deref()),
            ("testring-track", Some("testring"))
        );
        assert_eq!(library.get("testring").unwrap(), held);
        zip(
            &track,
            &[(MANIFEST, MANIFEST_JSON.as_bytes()), ("testring.erp", b"erp 2")],
        );
        let again = library
            .import(&track, None, &mut |_, _| {})
            .unwrap()
            .packages
            .remove(0);
        assert_eq!(again.id, "testring-track");
        assert_eq!(
            std::fs::read(library.dir("testring-track").join("track/testring.erp")).unwrap(),
            b"erp 2"
        );
        let mut ids: Vec<String> = library.list().unwrap().into_iter().map(|p| p.id).collect();
        ids.sort();
        assert_eq!(ids, ["testring", "testring-track"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_interrupted_migration_finishes_on_the_next_run() {
        let data = tmp("migrate-resume");
        let profile = map_profile(&data);
        let lib = data.join("addon-data").join(ID).join(LIBRARY);
        let staging = lib.join(format!("{MIGRATING}testring"));
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::rename(data.join("maps/testring"), staging.join(TRACK)).unwrap();
        std::fs::create_dir_all(lib.join(format!("{MIGRATING}empty"))).unwrap();
        let library = Library::new(&data.join("addon-data").join(ID));
        let other = data.join("other.zip");
        zip(
            &other,
            &[(MANIFEST, br#"{"track_code":"other"}"#), ("other.erp", b"erp")],
        );
        library.import(&other, None, &mut |_, _| {}).unwrap();
        assert!(staging.join("track/testring.erp").is_file());
        let notes = migrate(&data).unwrap();
        assert!(notes.iter().any(|n| n.starts_with("moved")), "{notes:?}");
        let p = library.get("testring").unwrap();
        assert_eq!(
            (p.source.as_str(), p.track.as_deref()),
            ("testring", Some("testring"))
        );
        assert!(library.dir("testring").join("track/testring.erp").is_file());
        assert_eq!(
            entries(&lib)
                .iter()
                .map(|d| package::file_name(d))
                .collect::<Vec<_>>(),
            ["other", "testring"]
        );
        let doc: Value = serde_json::from_slice(&std::fs::read(&profile).unwrap()).unwrap();
        assert_eq!(
            doc["state"][ID]["mods"],
            json!([{"id": "testring", "enabled": true}])
        );
        std::fs::remove_dir_all(&data).ok();
    }
}
