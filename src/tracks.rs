// The Tracks page: track packages from the library and installed data addons whose
// asset ops add a track (track.add), in one list. Track packages stay library
// mods, so their files and their place in each profile's state don't move; data
// addons stay launcher addons, managed through the Launcher API's addon calls.
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;
use swofty_addon::launcher::{AddonInfo, AddonStore, Claim, Cx};
use swofty_addon::manifest::relative_inside;
use swofty_addon::view::{
    Art, Block, Body, Button, ButtonKind, Control, Effect, Event, Fact, Icon, Notice, Row, Style, Tag, Tone,
    View,
};

use crate::library::{self, Library, Package};
use crate::track::{self, Info};
use crate::{Loaded, Result, date, err, monogram, package, size, subtitle};

pub const SECTION: &str = "tracks";
pub const IMPORT: &str = "import-tracks";
pub const ADDON: &str = "addon:";
pub const REMOVE_ADDON: &str = "remove-addon:";
pub const SHOW_ADDON: &str = "show-addon:";
const ADDON_MANIFEST: &str = "swofty.json";
const ADDON_EXTENSIONS: [&str; 3] = ["swoftyaddon", "swoftymod", "zip"];
const FOOTER: &str = "Track packages and data addons that add a track. Made by Team Simplified.";

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

fn manager<'a>(cx: &Cx<'a>) -> Result<&'a dyn AddonStore> {
    cx.addons()
        .ok_or_else(|| err("this Swofty Launcher can't manage addons from here: update it".into()))
}

pub fn import(cx: &Cx, paths: &[PathBuf]) -> Result<Vec<String>> {
    let library = Library::new(cx.data_dir);
    let known: Vec<String> = library.list()?.into_iter().map(|p| p.id).collect();
    let mut done = Vec::new();
    let mut failed = Vec::new();
    let mut added = Vec::new();
    for path in paths {
        let file = package::file_name(path);
        if let Some(scan) = scan_path(path) {
            if scan.tracks.is_empty() {
                failed.push(format!(
                    "{file} is an addon that adds no track: install it on the Addons page"
                ));
                continue;
            }
            let Some(addons) = cx.addons() else {
                failed.push(format!(
                    "{file} adds a track as a data addon, which this Swofty Launcher can't install from here: \
                     drop it on the window and it shows on the Addons page"
                ));
                continue;
            };
            let installed = addons.install(path).and_then(|a| match cx.profile.vanilla {
                true => Ok(a),
                false => addons.set_enabled(&a.id, true).map(|()| a),
            });
            match installed {
                Ok(a) => done.push(format!("Installed {} {}", a.name, a.version)),
                Err(e) => failed.push(format!("{file}: {e}")),
            }
        } else if is_package(path) {
            match library.import_track(path, &mut |_, _| {}) {
                Ok(p) => {
                    done.push(format!("Imported {}", p.name));
                    if !known.contains(&p.id) {
                        added.push(p.id);
                    }
                }
                Err(e) => failed.push(format!("{file}: {e}")),
            }
        } else if package::claims(path) == Claim::Yes {
            failed.push(format!(
                "{file} is a mod, not a track: import it on the Mods page"
            ));
        } else {
            failed.push(format!(
                "{file} is not a track: pick a track package (a zip with {}) or a data addon that adds a track",
                track::MANIFEST
            ));
        }
    }
    if !added.is_empty() && !cx.profile.vanilla {
        let mut loaded = Loaded::read(cx)?;
        for id in &added {
            loaded.set(id, true)?;
        }
        loaded.save(cx)?;
    }
    if failed.is_empty() {
        Ok(done)
    } else {
        failed.extend(done);
        Err(err(failed.join("\n")))
    }
}

pub fn import_button(primary: bool) -> Button {
    Button {
        icon: Some(Icon::Upload),
        style: if primary { Style::Primary } else { Style::Outline },
        kind: ButtonKind::Pick {
            title: "Import tracks".into(),
            extensions: vec!["zip".into(), "swoftyaddon".into(), "swoftymod".into()],
            folders: true,
            multiple: true,
        },
        ..Button::new(IMPORT, "Import…")
    }
}

enum Source {
    Package(Package),
    Addon(AddonInfo),
}

struct Listed {
    row: String,
    title: String,
    subtitle: String,
    on: bool,
    tracks: Vec<Info>,
    source: Source,
}

impl Listed {
    fn name(&self) -> String {
        match &self.source {
            Source::Package(p) => p.name.clone(),
            Source::Addon(a) => a.name.clone(),
        }
    }
}

fn listed(cx: &Cx) -> Result<(Vec<Listed>, bool)> {
    let library = Library::new(cx.data_dir);
    let mut out = Vec::new();
    for (p, on) in Loaded::read(cx)?.order {
        let Some(code) = p.track.clone() else {
            continue;
        };
        let info = Info::read(&library.dir(&p.id).join(track::TRACK)).unwrap_or(Info {
            code,
            name: Some(p.name.clone()),
            version: p.version.clone(),
            author: p.author.clone(),
            base: None,
        });
        out.push(Listed {
            row: p.id.clone(),
            title: p.name.clone(),
            subtitle: subtitle(&p),
            on,
            tracks: vec![info],
            source: Source::Package(p),
        });
    }
    let Some(addons) = cx.addons() else {
        return Ok((out, false));
    };
    for a in addons.list()? {
        let Some(scan) = scan_dir(&a.dir).filter(|s| !s.tracks.is_empty()) else {
            continue;
        };
        let names: Vec<String> = scan.tracks.iter().map(|f| f.name.clone()).collect();
        let by = [a.author.as_str(), a.version.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        out.push(Listed {
            row: format!("{ADDON}{}", a.id),
            title: names.join(", "),
            subtitle: by,
            on: a.enabled,
            tracks: scan.tracks.into_iter().map(|f| f.info).collect(),
            source: Source::Addon(a),
        });
    }
    out.sort_by_key(|l| l.title.to_lowercase());
    Ok((out, true))
}

fn remove_button(l: &Listed) -> Button {
    let (id, text) = match &l.source {
        Source::Package(p) => (
            format!("{}{}", crate::REMOVE, p.id),
            "It is deleted from the library and turned off in every profile.",
        ),
        Source::Addon(a) => (
            format!("{REMOVE_ADDON}{}", a.id),
            "The data addon is deleted and taken out of every profile.",
        ),
    };
    Button {
        icon: Some(Icon::Trash),
        style: Style::Danger,
        kind: ButtonKind::Confirm {
            title: format!("Remove {}?", l.title),
            text: text.into(),
        },
        ..Button::new(id, "Remove")
    }
}

fn row(l: &Listed, all: &[Listed], switchable: bool) -> Row {
    let dash = |v: Option<&str>| v.filter(|s| !s.is_empty()).unwrap_or("—").to_owned();
    let first = &l.tracks[0];
    let codes: Vec<&str> = l.tracks.iter().map(|t| t.code.as_str()).collect();
    let mut tags: Vec<Tag> = l
        .tracks
        .iter()
        .map(|t| Tag {
            text: change(t),
            tone: Tone::Accent,
        })
        .collect();
    tags.push(Tag {
        text: match l.source {
            Source::Package(_) => "Track package".into(),
            Source::Addon(_) => "Data addon".into(),
        },
        tone: Tone::Muted,
    });
    let clashes: Vec<&Listed> = all
        .iter()
        .filter(|o| l.on && o.on && o.row != l.row)
        .filter(|o| o.tracks.iter().any(|t| codes.contains(&t.code.as_str())))
        .collect();
    for o in &clashes {
        tags.push(Tag {
            text: format!("Also added by {}", o.title),
            tone: Tone::Warn,
        });
    }
    let mut stats = vec![
        Fact::new("STATUS", if l.on { "On" } else { "Off" }),
        Fact::new("TRACK CODE", codes.join(", ")),
        Fact::new(
            "BASED ON",
            dash(first.base.as_deref().map(|b| stock(b).unwrap_or(b))),
        ),
    ];
    let (show, from) = match &l.source {
        Source::Package(p) => {
            stats.extend([
                Fact::new("VERSION", dash(p.version.as_deref())),
                Fact::new("AUTHOR", dash(p.author.as_deref())),
                Fact::new("SIZE", size(p.size())),
                Fact::new("IMPORTED", date(p.imported)),
            ]);
            (
                format!("{}{}", crate::SHOW, p.id),
                format!("A track package in the library, imported from {}.", p.source),
            )
        }
        Source::Addon(a) => {
            stats.extend([
                Fact::new("VERSION", dash(Some(&a.version))),
                Fact::new("AUTHOR", dash(Some(&a.author))),
                Fact::new("ADDON", a.id.clone()),
            ]);
            (
                format!("{SHOW_ADDON}{}", a.id),
                format!(
                    "The data addon {}. It is on the Addons page too, and turning it on or off here does the same there.",
                    a.name
                ),
            )
        }
    };
    let mut detail = vec![
        Block::Stats(stats),
        Block::Text {
            text: from,
            tone: Tone::Muted,
        },
    ];
    if l.tracks.len() > 1 {
        detail.push(Block::Names {
            label: "TRACKS".into(),
            items: l
                .tracks
                .iter()
                .map(|t| format!("{} · {}", t.name.as_deref().unwrap_or(&t.code), change(t)))
                .collect(),
        });
    }
    for o in &clashes {
        detail.push(Block::Text {
            text: format!(
                "{} adds the same track. A build with both on fails: turn one off.",
                o.name()
            ),
            tone: Tone::Warn,
        });
    }
    detail.push(Block::Buttons(vec![
        Button {
            icon: Some(Icon::FolderOpen),
            ..Button::new(show, "Show files")
        },
        remove_button(l),
    ]));
    Row {
        id: l.row.clone(),
        title: l.title.clone(),
        subtitle: l.subtitle.clone(),
        art: Art::Monogram(monogram(&l.title)),
        tags,
        trailing: Some(Control::Switch {
            id: l.row.clone(),
            on: l.on,
            enabled: switchable,
        }),
        detail,
        menu: vec![remove_button(l)],
    }
}

pub fn view(cx: &Cx, mut notices: Vec<Notice>) -> Result<View> {
    let (listed, addons) = listed(cx)?;
    if !addons {
        notices.push(Notice {
            text: "Update Swofty Launcher to list the data addons that add a track here too.".into(),
            tone: Tone::Muted,
            action: None,
        });
    }
    let on = listed.iter().filter(|l| l.on).count();
    let body = if listed.is_empty() {
        Body::Empty {
            title: "No tracks yet".into(),
            text: "Drop a track's zip on the window, or import a track package or a data addon that adds a track."
                .into(),
            action: Some(import_button(true)),
        }
    } else {
        Body::List {
            rows: listed
                .iter()
                .map(|l| row(l, &listed, !cx.profile.vanilla))
                .collect(),
            reorder: false,
            search: None,
        }
    };
    Ok(View {
        subtitle: format!(
            "{} {} · {on} on in {}",
            listed.len(),
            if listed.len() == 1 { "track" } else { "tracks" },
            cx.profile.name
        ),
        actions: vec![import_button(false)],
        notices,
        body,
        footer: Some(FOOTER.into()),
        tick: false,
    })
}

pub fn event(cx: &Cx, event: Event) -> Result<Option<Effect>> {
    let effect = match event {
        Event::Toggle { id, on } => match id.strip_prefix(ADDON) {
            Some(addon) => {
                manager(cx)?.set_enabled(addon, on)?;
                Effect::Reload
            }
            None => {
                let mut loaded = Loaded::read(cx)?;
                loaded.set(&id, on)?;
                loaded.save(cx)?;
                Effect::Reload
            }
        },
        Event::Picked { id, paths } if id == IMPORT => Effect::Notice(import(cx, &paths)?.join("\n")),
        Event::Click { id } if id.starts_with(REMOVE_ADDON) => {
            let id = &id[REMOVE_ADDON.len()..];
            let addons = manager(cx)?;
            let name = addons
                .list()?
                .into_iter()
                .find(|a| a.id == id)
                .map_or_else(|| id.to_owned(), |a| a.name);
            addons.remove(id)?;
            Effect::Notice(format!("Removed {name}"))
        }
        Event::Click { id } if id.starts_with(SHOW_ADDON) => {
            let id = &id[SHOW_ADDON.len()..];
            let dir = manager(cx)?
                .list()?
                .into_iter()
                .find(|a| a.id == id)
                .map(|a| a.dir)
                .ok_or_else(|| err(format!("no addon named {id} is installed")))?;
            Effect::Open(dir)
        }
        _ => return Ok(None),
    };
    Ok(Some(effect))
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
    use std::sync::Mutex;

    use serde_json::json;
    use swofty_addon::launcher::{ProfileInfo, StateStore};

    use super::*;
    use crate::{ID, Simplified, Slot, State};
    use swofty_addon::launcher::Addon;

    const OKAYAMA: &str = r#"{"track_code":"okayama","track_name":"Okayama","version":"0.17.0","author_name":"ryangrobf1",
        "erp":"okayama.erp","activation":{"copy_from_track":"melbourne","copy_slot":"melbourne"}}"#;
    const SEPANG: &str = r#"{"track_code":"bahrain24","track_name":"Sepang","version":"1.0.0","author_name":"Pajko",
        "erp":"bahrain24.erp","activation":{"copy_from_track":"bahrain"}}"#;

    #[derive(Default)]
    struct Memory(Mutex<Option<Value>>);

    impl StateStore for Memory {
        fn get(&self, _: &str) -> swofty_addon::Result<Option<Value>> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn set(&self, _: &str, value: Value) -> swofty_addon::Result<()> {
            *self.0.lock().unwrap() = Some(value);
            Ok(())
        }
    }

    // The launcher's addon store, for a profile, with each addon installed by copying
    // its folder under `root`.
    struct Addons {
        root: PathBuf,
        on: Mutex<Vec<String>>,
    }

    impl AddonStore for Addons {
        fn list(&self) -> swofty_addon::Result<Vec<AddonInfo>> {
            let mut out = Vec::new();
            for dir in std::fs::read_dir(&self.root).into_iter().flatten().flatten() {
                let m: Value =
                    serde_json::from_slice(&std::fs::read(dir.path().join(ADDON_MANIFEST)).unwrap()).unwrap();
                let id = m["id"].as_str().unwrap().to_owned();
                out.push(AddonInfo {
                    enabled: self.on.lock().unwrap().contains(&id),
                    id,
                    name: m["name"].as_str().unwrap().into(),
                    version: m["version"].as_str().unwrap().into(),
                    author: m["author"].as_str().unwrap().into(),
                    dir: dir.path(),
                });
            }
            out.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(out)
        }

        fn install(&self, path: &Path) -> swofty_addon::Result<AddonInfo> {
            let scan = scan_path(path).unwrap();
            let dest = self.root.join(&scan.id);
            std::fs::create_dir_all(&dest).unwrap();
            copy(path, &dest);
            Ok(self.list()?.into_iter().find(|a| a.id == scan.id).unwrap())
        }

        fn set_enabled(&self, id: &str, on: bool) -> swofty_addon::Result<()> {
            let mut list = self.on.lock().unwrap();
            list.retain(|i| i != id);
            if on {
                list.push(id.into());
            }
            Ok(())
        }

        fn remove(&self, id: &str) -> swofty_addon::Result<()> {
            std::fs::remove_dir_all(self.root.join(id)).unwrap();
            self.on.lock().unwrap().retain(|i| i != id);
            Ok(())
        }
    }

    fn copy(from: &Path, to: &Path) {
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            let dest = to.join(entry.file_name());
            if entry.path().is_dir() {
                std::fs::create_dir_all(&dest).unwrap();
                copy(&entry.path(), &dest);
            } else {
                std::fs::copy(entry.path(), dest).unwrap();
            }
        }
    }

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

    struct Setup {
        root: PathBuf,
        data: PathBuf,
        state: Memory,
        addons: Addons,
        profile: ProfileInfo,
    }

    impl Setup {
        fn new(name: &str) -> Self {
            let root = tmp(name);
            std::fs::create_dir_all(root.join("addons")).unwrap();
            Self {
                data: root.join("data"),
                addons: Addons {
                    root: root.join("addons"),
                    on: Mutex::default(),
                },
                root,
                state: Memory::default(),
                profile: ProfileInfo {
                    name: "default".into(),
                    vanilla: false,
                    enabled: true,
                },
            }
        }

        fn cx(&self, manages: bool) -> Cx<'_> {
            let cx = Cx::new(ID, &self.profile, &self.data, &self.root, None, &self.state);
            match manages {
                true => cx.with_addons(&self.addons),
                false => cx,
            }
        }

        fn state(&self) -> Vec<Slot> {
            self.cx(false).state::<State>().unwrap().mods
        }
    }

    fn rows(view: &View) -> Vec<(String, String, bool)> {
        match &view.body {
            Body::List { rows, .. } => rows
                .iter()
                .map(|r| {
                    let on = matches!(r.trailing, Some(Control::Switch { on, .. }) if on);
                    (r.id.clone(), r.title.clone(), on)
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn tags(view: &View, id: &str) -> Vec<String> {
        match &view.body {
            Body::List { rows, .. } => rows
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.tags.iter().map(|t| t.text.clone()).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
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

    #[test]
    fn the_picker_imports_track_packages_and_installs_track_addons() {
        let s = Setup::new("picker");
        let src = s.root.join("src");
        let package = src.join("sepang.zip");
        std::fs::create_dir_all(&src).unwrap();
        zip(
            &package,
            &[(track::MANIFEST, SEPANG.as_bytes()), ("bahrain24.erp", b"erp")],
        );
        let addon = src.join("okayama-named");
        named_addon(&addon);
        let lines = import(&s.cx(true), &[package, addon]).unwrap();
        assert_eq!(
            lines,
            ["Imported Sepang", "Installed Okayama with its own name 1.0.0"]
        );
        let library = Library::new(&s.data);
        assert!(library.dir("bahrain24").join("track/bahrain24.erp").is_file());
        assert_eq!(
            s.state(),
            [Slot {
                id: "bahrain24".into(),
                enabled: true
            }]
        );
        assert_eq!(*s.addons.on.lock().unwrap(), ["you.okayama-named"]);
        let view = Simplified::new().view(&s.cx(true), SECTION).unwrap();
        assert_eq!(
            rows(&view),
            [
                (
                    "addon:you.okayama-named".into(),
                    "Okayama International".into(),
                    true
                ),
                ("bahrain24".into(), "Sepang".into(), true),
            ]
        );
        assert_eq!(
            tags(&view, "addon:you.okayama-named"),
            ["Adds a track based on Melbourne", "Data addon"]
        );
        assert_eq!(
            tags(&view, "bahrain24"),
            ["Replaces Sakhir 2024", "Track package"]
        );
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn the_picker_refuses_what_is_not_a_track() {
        let s = Setup::new("refuse");
        let src = s.root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let serps = src.join("livery.serps");
        std::fs::write(&serps, "x").unwrap();
        let livery = src.join("livery");
        write(
            &livery.join(ADDON_MANIFEST),
            r#"{"id":"you.livery","name":"Livery","version":"1.0.0","author":"You"}"#,
        );
        let notes = src.join("notes.txt");
        std::fs::write(&notes, "x").unwrap();
        let other = src.join("other.zip");
        zip(&other, &[("readme.txt", b"x")]);
        let e = import(&s.cx(true), &[serps, livery, notes, other])
            .unwrap_err()
            .to_string();
        let lines: Vec<&str> = e.lines().collect();
        assert_eq!(lines.len(), 4, "{e}");
        assert!(lines[0].contains("livery.serps is a mod, not a track"), "{e}");
        assert!(lines[1].contains("livery is an addon that adds no track"), "{e}");
        assert!(lines[2].contains("notes.txt is not a track"), "{e}");
        assert!(lines[3].contains("other.zip is not a track"), "{e}");
        assert!(Library::new(&s.data).list().unwrap().is_empty());
        assert!(s.addons.list().unwrap().is_empty());

        let addon = src.join("okayama-named");
        named_addon(&addon);
        let e = import(&s.cx(false), &[addon]).unwrap_err().to_string();
        assert!(e.contains("drop it on the window"), "{e}");
        assert!(s.addons.list().unwrap().is_empty());
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn imported_tracks_move_to_the_tracks_page_with_their_state_and_files() {
        let s = Setup::new("upgrade");
        let src = s.root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let library = Library::new(&s.data);
        for (name, manifest, erp) in [
            ("okayama", OKAYAMA, "okayama.erp"),
            ("sepang", SEPANG, "bahrain24.erp"),
        ] {
            let path = src.join(format!("{name}.zip"));
            zip(&path, &[(track::MANIFEST, manifest.as_bytes()), (erp, b"erp")]);
            library.import(&path, None, &mut |_, _| {}).unwrap();
        }
        for name in ["livery", "logos"] {
            let dir = src.join(name);
            write(
                &dir.join("F1 25/2025_asset_groups").join(format!("{name}.erp")),
                "erp",
            );
            write(
                &dir.join(crate::serps::FILE),
                &format!(r#"{{"mod_name":"{name}"}}"#),
            );
            library.import(&dir, None, &mut |_, _| {}).unwrap();
        }
        let before = vec![
            Slot {
                id: "livery".into(),
                enabled: true,
            },
            Slot {
                id: "okayama".into(),
                enabled: false,
            },
            Slot {
                id: "logos".into(),
                enabled: true,
            },
            Slot {
                id: "bahrain24".into(),
                enabled: true,
            },
        ];
        s.cx(false).set_state(&State { mods: before.clone() }).unwrap();
        let files = |id: &str| package::files(&library.dir(id)).unwrap();
        let held: Vec<_> = ["livery", "okayama", "logos", "bahrain24"].map(files).to_vec();

        let addon = Simplified::new();
        let cx = s.cx(false);
        let mods = addon.view(&cx, crate::SECTION).unwrap();
        assert_eq!(
            rows(&mods),
            [
                ("livery".into(), "livery".into(), true),
                ("logos".into(), "logos".into(), true)
            ]
        );
        assert!(
            mods.subtitle.starts_with("2 in library · 2 on"),
            "{}",
            mods.subtitle
        );
        let tracks = addon.view(&cx, SECTION).unwrap();
        assert_eq!(
            rows(&tracks),
            [
                ("okayama".into(), "Okayama".into(), false),
                ("bahrain24".into(), "Sepang".into(), true)
            ]
        );
        assert!(
            tracks.subtitle.starts_with("2 tracks · 1 on"),
            "{}",
            tracks.subtitle
        );
        assert!(
            tracks
                .notices
                .iter()
                .any(|n| n.text.contains("Update Swofty Launcher"))
        );
        assert_eq!(s.state(), before);

        addon
            .event(
                &cx,
                crate::SECTION,
                Event::Move {
                    row: "logos".into(),
                    to: 0,
                },
            )
            .unwrap();
        assert_eq!(
            s.state(),
            [
                Slot {
                    id: "logos".into(),
                    enabled: true
                },
                Slot {
                    id: "livery".into(),
                    enabled: true
                },
                Slot {
                    id: "okayama".into(),
                    enabled: false
                },
                Slot {
                    id: "bahrain24".into(),
                    enabled: true
                },
            ]
        );
        addon
            .event(
                &cx,
                crate::SECTION,
                Event::Move {
                    row: "logos".into(),
                    to: 1,
                },
            )
            .unwrap();
        let ids: Vec<String> = s.state().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["livery", "logos", "okayama", "bahrain24"]);
        addon
            .event(
                &cx,
                SECTION,
                Event::Toggle {
                    id: "okayama".into(),
                    on: true,
                },
            )
            .unwrap();
        assert!(s.state().iter().all(|s| s.enabled));
        assert_eq!(
            ["livery", "okayama", "logos", "bahrain24"].map(files).to_vec(),
            held
        );
        let batch = addon
            .build(&swofty_addon::launcher::BuildCx {
                cx: s.cx(false),
                options: &Default::default(),
                features: &Default::default(),
            })
            .unwrap();
        assert!(batch.ops.is_empty(), "no game, so nothing builds");
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn a_track_addon_switches_and_removes_from_its_row() {
        let s = Setup::new("addon-row");
        named_addon(&s.addons.root.join("you.okayama-named"));
        let livery = s.addons.root.join("you.livery");
        write(
            &livery.join(ADDON_MANIFEST),
            r#"{"id":"you.livery","name":"Livery","version":"1.0.0","author":"You"}"#,
        );
        let src = s.root.join("okayama.zip");
        zip(
            &src,
            &[(track::MANIFEST, OKAYAMA.as_bytes()), ("okayama.erp", b"erp")],
        );
        import(&s.cx(true), &[src]).unwrap();
        let addon = Simplified::new();
        let cx = s.cx(true);
        let view = addon.view(&cx, SECTION).unwrap();
        assert_eq!(
            rows(&view),
            [
                ("okayama".into(), "Okayama".into(), true),
                (
                    "addon:you.okayama-named".into(),
                    "Okayama International".into(),
                    false
                ),
            ]
        );
        let row = "addon:you.okayama-named".to_owned();
        addon
            .event(
                &cx,
                SECTION,
                Event::Toggle {
                    id: row.clone(),
                    on: true,
                },
            )
            .unwrap();
        assert_eq!(*s.addons.on.lock().unwrap(), ["you.okayama-named"]);
        let view = addon.view(&cx, SECTION).unwrap();
        assert!(tags(&view, &row).contains(&"Also added by Okayama".to_owned()));
        assert!(tags(&view, "okayama").contains(&"Also added by Okayama International".to_owned()));
        let effect = addon
            .event(
                &cx,
                SECTION,
                Event::Click {
                    id: format!("{SHOW_ADDON}you.okayama-named"),
                },
            )
            .unwrap();
        assert_eq!(effect, Effect::Open(s.addons.root.join("you.okayama-named")));
        let effect = addon
            .event(
                &cx,
                SECTION,
                Event::Click {
                    id: format!("{REMOVE_ADDON}you.okayama-named"),
                },
            )
            .unwrap();
        assert_eq!(effect, Effect::Notice("Removed Okayama with its own name".into()));
        assert_eq!(
            s.addons
                .list()
                .unwrap()
                .into_iter()
                .map(|a| a.id)
                .collect::<Vec<_>>(),
            ["you.livery"]
        );
        let effect = addon
            .event(
                &cx,
                SECTION,
                Event::Click {
                    id: format!("{}okayama", crate::SHOW),
                },
            )
            .unwrap();
        assert_eq!(
            effect,
            Effect::Open(Library::new(&s.data).dir("okayama").join(track::TRACK))
        );
        std::fs::remove_dir_all(&s.root).ok();
    }
}
