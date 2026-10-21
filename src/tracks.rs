// What a track package or a data addon that adds a track (track.add) holds, read
// for the report of a track the Mod Importer converts.
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;
use swofty_addon::launcher::Claim;
use swofty_addon::manifest::relative_inside;

use crate::library::{self, Library};
use crate::track::{self, Info};

const ADDON_MANIFEST: &str = "swofty.json";
const ADDON_EXTENSIONS: [&str; 3] = ["swoftyaddon", "swoftymod", "zip"];

// F1 25's own tracks: the gamedb code and the name the game shows.
const STOCK: &[(&str, &str)] = &[
    ("melbourne", "Melbourne"),
    ("paul_ricard", "Paul Ricard"),
    ("shanghai", "Shanghai"),
    ("bahrain", "Sakhir"),
    ("catalunya", "Barcelona"),
    ("monaco", "Monaco"),
    ("montreal", "Montreal"),
    ("silverstone", "Silverstone"),
    ("hanoi", "Hanoi"),
    ("hungaroring", "Hungaroring"),
    ("spa_francorchamps", "Spa-Francorchamps"),
    ("monza", "Monza"),
    ("singapore", "Singapore"),
    ("suzuka", "Suzuka"),
    ("abu_dhabi", "Yas Marina"),
    ("texas", "Austin"),
    ("brazil", "Interlagos"),
    ("austria", "Red Bull Ring"),
    ("sochi", "Sochi"),
    ("mexico", "Mexico City"),
    ("baku", "Baku"),
    ("bahrain_pad", "Sakhir Short"),
    ("silverstone_int", "Silverstone Short"),
    ("texas_nat", "Austin Short"),
    ("suzuka_east", "Suzuka Short"),
    ("styria", "Styria"),
    ("zandvoort", "Zandvoort"),
    ("imola", "Imola"),
    ("portimao", "Portimão"),
    ("jeddah", "Jeddah"),
    ("miami", "Miami"),
    ("las_vegas", "Las Vegas"),
    ("losail", "Lusail"),
    ("imola_prototype", "Imola Prototype"),
    ("melbourne24", "Melbourne 2024"),
    ("bahrain24", "Sakhir 2024"),
    ("suzuka24", "Suzuka 2024"),
    ("imola24", "Imola 2024"),
    ("miami24", "Miami 2024"),
    ("silverstone_reverse", "Silverstone Reverse"),
    ("austria_reverse", "Red Bull Ring Reverse"),
    ("zandvoort_reverse", "Zandvoort Reverse"),
    ("madrid", "Madrid"),
];

fn stock(code: &str) -> Option<&'static str> {
    STOCK.iter().find(|(c, _)| *c == code).map(|(_, name)| *name)
}

pub fn change(info: &Info) -> String {
    if let Some(name) = stock(&info.code) {
        return format!("Replaces {name}");
    }
    match info.base.as_deref() {
        Some(base) => format!("Adds a track based on {}", stock(base).unwrap_or(base)),
        None => "Adds a track".into(),
    }
}

pub struct Scan {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Found>,
}

pub struct Found {
    pub info: Info,
    pub name: String,
}

trait Files {
    fn read(&mut self, rel: &str) -> Option<Vec<u8>>;
}

struct Folder(PathBuf);

impl Files for Folder {
    fn read(&mut self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(rel)).ok()
    }
}

struct Zipped {
    zip: zip::ZipArchive<std::fs::File>,
    prefix: String,
}

impl Files for Zipped {
    fn read(&mut self, rel: &str) -> Option<Vec<u8>> {
        let mut file = self.zip.by_name(&format!("{}{rel}", self.prefix)).ok()?;
        let mut out = Vec::new();
        file.read_to_end(&mut out).ok()?;
        Some(out)
    }
}

// Where a dropped or picked path holds an addon, found as the launcher finds it: a
// folder with swofty.json or one folder holding it, or a .swoftyaddon, .swoftymod
// or zip with swofty.json at its root or in its one top folder.
fn open(path: &Path) -> Option<Box<dyn Files>> {
    if path.is_dir() {
        if path.join(ADDON_MANIFEST).is_file() {
            return Some(Box::new(Folder(path.to_path_buf())));
        }
        let entries: Vec<PathBuf> = std::fs::read_dir(path)
            .ok()?
            .filter_map(|e| Some(e.ok()?.path()))
            .collect();
        return match entries.as_slice() {
            [only] if only.join(ADDON_MANIFEST).is_file() => Some(Box::new(Folder(only.clone()))),
            _ => None,
        };
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !ADDON_EXTENSIONS.contains(&ext.as_str()) {
        return None;
    }
    let zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let names: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.into_owned()))
        .collect();
    let prefix = if names.iter().any(|n| n == ADDON_MANIFEST) {
        String::new()
    } else {
        let mut tops = names.iter().filter_map(|n| n.split('/').next());
        let top = tops.next()?;
        if !tops.all(|t| t == top) || !names.contains(&format!("{top}/{ADDON_MANIFEST}")) {
            return None;
        }
        format!("{top}/")
    };
    Some(Box::new(Zipped { zip, prefix }))
}

pub fn scan_path(path: &Path) -> Option<Scan> {
    scan(open(path)?.as_mut())
}

pub fn scan_dir(dir: &Path) -> Option<Scan> {
    scan(&mut Folder(dir.to_path_buf()))
}

fn scan(files: &mut dyn Files) -> Option<Scan> {
    let manifest: Value = serde_json::from_slice(&files.read(ADDON_MANIFEST)?).ok()?;
    let text = |k: &str| {
        manifest
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut ops = Vec::new();
    let assets = manifest["operations"]["assets"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for rel in assets
        .iter()
        .filter_map(Value::as_str)
        .filter(|r| relative_inside(r))
    {
        let doc = files
            .read(rel)
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        ops.extend(doc.and_then(|d| d["ops"].as_array().cloned()).unwrap_or_default());
    }
    let mut strings: BTreeMap<String, String> = BTreeMap::new();
    for op in ops.iter().filter(|op| op["op"] == "lang.set") {
        let (Some(key), Some(value)) = (op["key"].as_str(), op["value"].as_str()) else {
            continue;
        };
        if op["language"] == "*" || !strings.contains_key(key) {
            strings.insert(key.to_owned(), value.to_owned());
        }
    }
    let mut tracks = Vec::new();
    for op in ops.iter().filter(|op| op["op"] == "track.add") {
        let source = match &op["source"] {
            Value::String(path) => path.as_str(),
            Value::Object(o) if o.get("root").is_none_or(|r| r == "addon") => {
                o.get("path").and_then(Value::as_str).unwrap_or_default()
            }
            _ => continue,
        };
        if !relative_inside(source) {
            continue;
        }
        let rel = format!("{}/{}", source.trim_end_matches('/'), track::MANIFEST);
        let Some(info) = files.read(&rel).and_then(|b| Info::parse(&b)) else {
            continue;
        };
        let name = own_name(&ops, &strings, &info.code)
            .or_else(|| info.name.clone())
            .unwrap_or_else(|| info.code.clone());
        tracks.push(Found { info, name });
    }
    Some(Scan {
        id: text("id"),
        name: text("name"),
        tracks,
    })
}

// The name an addon gives its track: the string its db.set puts in the track's
// TrackNameData_0.title, as A track with its own name in the launcher docs does.
fn own_name(ops: &[Value], strings: &BTreeMap<String, String>, code: &str) -> Option<String> {
    let row = format!("={code}]");
    ops.iter()
        .filter(|op| op["op"] == "db.set" && op["field"] == "TrackNameData_0.title")
        .filter(|op| op["select"].as_str().is_some_and(|s| s.contains(&row)))
        .find_map(|op| strings.get(op["value"].as_str()?).cloned())
}

pub fn is_package(path: &Path) -> bool {
    if !path.is_dir() {
        return track::claims(path) == Claim::Yes;
    }
    if Info::read(path).is_some() {
        return true;
    }
    let entries: Vec<PathBuf> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(|e| Some(e.ok()?.path()))
        .collect();
    matches!(entries.as_slice(), [only] if Info::read(only).is_some())
}

pub fn files_dir(library: &Library, id: &str) -> PathBuf {
    let dir = library.dir(id);
    match dir.join(track::TRACK).is_dir() {
        true => dir.join(track::TRACK),
        false => dir.join(library::FILES),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const OKAYAMA: &str = r#"{"track_code":"okayama","track_name":"Okayama","version":"0.17.0","author_name":"ryangrobf1",
        "erp":"okayama.erp","activation":{"copy_from_track":"melbourne","copy_slot":"melbourne"}}"#;
    const SEPANG: &str = r#"{"track_code":"bahrain24","track_name":"Sepang","version":"1.0.0","author_name":"Pajko",
        "erp":"bahrain24.erp","activation":{"copy_from_track":"bahrain"}}"#;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("swofty-tracks-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
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

    // A data addon laid out as "A track with its own name" in the launcher docs.
    fn named_addon(dir: &Path) {
        write(
            &dir.join(ADDON_MANIFEST),
            r#"{"schema_version":1,"id":"you.okayama-named","name":"Okayama with its own name","version":"1.0.0",
                "author":"You","compat":{"game":"f1_25"},"operations":{"assets":["assets/track.json"]}}"#,
        );
        write(
            &dir.join("assets/track.json"),
            &json!({"schema_version": 1, "ops": [
                {"op": "track.add", "source": "track"},
                {"op": "lang.set", "language": "*", "key": "lng_okayama_title", "value": "Okayama International"},
                {"op": "db.set", "select": "track_data.game_setup.TrackDataSet[_metadata_=f1].TrackData[_metadata_=okayama]",
                 "field": "TrackNameData_0.title", "value": "lng_okayama_title"}
            ]})
            .to_string(),
        );
        write(&dir.join("track").join(track::MANIFEST), OKAYAMA);
        write(&dir.join("track/okayama.erp"), "erp");
    }

    #[test]
    fn a_data_addon_is_a_track_when_its_ops_add_one() {
        let root = tmp("scan");
        let dir = root.join("named");
        named_addon(&dir);
        let scan = scan_dir(&dir).unwrap();
        assert_eq!(scan.id, "you.okayama-named");
        assert_eq!(scan.tracks.len(), 1);
        assert_eq!(scan.tracks[0].name, "Okayama International");
        assert_eq!(scan.tracks[0].info.base.as_deref(), Some("melbourne"));
        assert_eq!(change(&scan.tracks[0].info), "Adds a track based on Melbourne");

        let packed = root.join("named.swoftyaddon");
        let files: Vec<(String, Vec<u8>)> = ["swofty.json", "assets/track.json", "track/manifest.json"]
            .iter()
            .map(|rel| (format!("named/{rel}"), std::fs::read(dir.join(rel)).unwrap()))
            .collect();
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        zip(&packed, &refs);
        let scan = scan_path(&packed).unwrap();
        assert_eq!(scan.tracks[0].info.code, "okayama");
        assert_eq!(scan.tracks[0].name, "Okayama International");

        let plain = root.join("plain");
        write(
            &plain.join(ADDON_MANIFEST),
            r#"{"id":"you.livery","name":"Livery","version":"1.0.0","author":"You",
                "operations":{"assets":["assets/a.json", "../outside.json"]}}"#,
        );
        write(
            &plain.join("assets/a.json"),
            r#"{"schema_version":1,"ops":[{"op":"lang.set","language":"*","key":"k","value":"v"},
                {"op":"track.add","source":"../elsewhere"}]}"#,
        );
        assert!(scan_path(&plain).unwrap().tracks.is_empty());
        assert!(scan_path(&dir.join("track")).is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_track_says_what_it_replaces_or_adds() {
        let sepang = Info::parse(SEPANG.as_bytes()).unwrap();
        assert_eq!(change(&sepang), "Replaces Sakhir 2024");
        let mut okayama = Info::parse(OKAYAMA.as_bytes()).unwrap();
        assert_eq!(change(&okayama), "Adds a track based on Melbourne");
        okayama.base = Some("testring".into());
        assert_eq!(change(&okayama), "Adds a track based on testring");
        okayama.base = None;
        assert_eq!(change(&okayama), "Adds a track");
    }
}
