// The Mod Importer: every mod in the library becomes a standalone Swofty data addon,
// installed and switched through the Launcher API's addon host calls. The library
// stays as the source each addon is made from again.
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use swofty_addon::launcher::{AddonStore, Cx, Progress};
use swofty_addon::manifest::{Category, Kind};

use crate::library::{Library, Package};
use crate::{Loaded, Result, State, err, io_err, package};

pub const RECORD: &str = "importer.json";
const STAGING: &str = ".convert";
pub const UPDATE: &str = "Update Swofty Launcher to 1.3.0 or later to turn mods into addons.";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Made {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<Category>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub type Report<'a> = &'a mut dyn FnMut(f32, &str);

pub trait Convert {
    fn convert(&self, cx: &Cx, library: &Library, p: &Package, out: &Path, progress: Report) -> Result<Made>;
}

pub struct Converter;

impl Convert for Converter {
    fn convert(&self, cx: &Cx, library: &Library, p: &Package, out: &Path, progress: Report) -> Result<Made> {
        let c = crate::convert::convert(library, p, cx.game, out, progress)?;
        Ok(Made {
            id: c.id,
            name: c.name,
            category: Some(c.category),
            kinds: c.kinds.into_iter().map(|k| kind(k).into()).collect(),
            lines: c.report,
            warnings: Vec::new(),
        })
    }
}

fn kind(k: Kind) -> &'static str {
    match k {
        Kind::Database => "Database",
        Kind::Erp => "Archives",
        Kind::Ui => "UI",
        Kind::Files => "Files",
        Kind::Native => "Code",
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub mods: BTreeMap<String, Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<Last>,
}

// One library mod: the addon made from it, and the reason the last conversion
// failed. A failed conversion of a mod made before keeps the addon made before.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made: Option<Made>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Last {
    pub at: u64,
    pub files: Vec<String>,
    #[serde(default)]
    pub mods: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl Record {
    pub fn load(data: &Path) -> Result<Self> {
        let path = data.join(RECORD);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| err(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(io_err(&path)(e)),
        }
    }

    pub fn save(&self, data: &Path) -> Result<()> {
        std::fs::create_dir_all(data).map_err(io_err(data))?;
        let path = data.join(RECORD);
        let tmp = data.join(format!("{RECORD}.tmp"));
        let text = serde_json::to_vec_pretty(self).map_err(|e| err(format!("{}: {e}", path.display())))?;
        std::fs::write(&tmp, text).map_err(io_err(&tmp))?;
        std::fs::rename(&tmp, &path).map_err(io_err(&path))
    }

    pub fn addon(&self, id: &str) -> Option<&Made> {
        self.mods.get(id).and_then(|e| e.made.as_ref())
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// The mods the profile now loads as addons, which the build hook leaves out.
pub fn handed_over(cx: &Cx) -> Result<HashSet<String>> {
    let record = Record::load(cx.data_dir)?;
    let state: State = cx.state()?;
    Ok(state
        .addons
        .into_iter()
        .filter(|id| record.addon(id).is_some())
        .collect())
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pending {
    pub convert: usize,
    pub place: usize,
}

impl Pending {
    pub fn any(&self) -> bool {
        self.convert + self.place > 0
    }
}

pub fn pending(cx: &Cx, record: &Record) -> Result<Pending> {
    let library = Library::new(cx.data_dir).list()?;
    let convert = library
        .iter()
        .filter(|p| !record.mods.contains_key(&p.id))
        .count();
    let place = match cx.profile.vanilla {
        true => 0,
        false => {
            let state: State = cx.state()?;
            state
                .mods
                .iter()
                .filter(|s| record.addon(&s.id).is_some() && !state.addons.contains(&s.id))
                .count()
        }
    };
    Ok(Pending { convert, place })
}

fn staging(data: &Path, id: &str) -> PathBuf {
    data.join(STAGING).join(id)
}

fn convert_one(
    cx: &Cx,
    addons: &dyn AddonStore,
    conv: &dyn Convert,
    library: &Library,
    p: &Package,
    record: &mut Record,
    progress: Report,
) -> Result<Made> {
    let out = staging(cx.data_dir, &p.id);
    let made = (|| -> Result<Made> {
        if out.exists() {
            std::fs::remove_dir_all(&out).map_err(io_err(&out))?;
        }
        std::fs::create_dir_all(&out).map_err(io_err(&out))?;
        let mut made = conv.convert(cx, library, p, &out, progress)?;
        made.id = addons.install(&out)?.id;
        Ok(made)
    })();
    std::fs::remove_dir_all(&out).ok();
    let entry = record.mods.entry(p.id.clone()).or_default();
    entry.name = p.name.clone();
    entry.at = now();
    match &made {
        Ok(m) => {
            entry.made = Some(m.clone());
            entry.error = None;
        }
        Err(e) => entry.error = Some(e.to_string()),
    }
    made
}

// Turns on, in the profile's mod order, the addons made from the mods the profile
// lists, as each mod was on or off. Set in order, they append in that order.
fn place(cx: &Cx, addons: &dyn AddonStore, record: &Record) -> Result<Vec<String>> {
    if cx.profile.vanilla {
        return Ok(Vec::new());
    }
    let mut state: State = cx.state()?;
    let mut lines = Vec::new();
    let mut placed = Vec::new();
    for slot in &state.mods {
        let Some(made) = record
            .addon(&slot.id)
            .filter(|_| !state.addons.contains(&slot.id))
        else {
            continue;
        };
        match addons.set_enabled(&made.id, slot.enabled) {
            Ok(()) => placed.push(slot.id.clone()),
            Err(e) => lines.push(format!("{}: {e}", made.name)),
        }
    }
    if !placed.is_empty() {
        state.addons.extend(placed);
        cx.set_state(&state)?;
    }
    Ok(lines)
}

fn hand_over(cx: &Cx, id: &str) -> Result<()> {
    if cx.profile.vanilla {
        return Ok(());
    }
    let mut state: State = cx.state()?;
    if !state.addons.iter().any(|a| a == id) {
        state.addons.push(id.to_owned());
        cx.set_state(&state)?;
    }
    Ok(())
}

// Converts each library mod that has no addon yet, then hands the profile's mods
// over to their addons. Runs again for each profile, converting nothing twice.
pub fn migrate(cx: &Cx, conv: &dyn Convert, progress: &dyn Progress) -> Result<Vec<String>> {
    let Some(addons) = cx.addons() else {
        return Ok(Vec::new());
    };
    let library = Library::new(cx.data_dir);
    let mut record = Record::load(cx.data_dir)?;
    let todo: Vec<Package> = Loaded::read(cx)?
        .order
        .into_iter()
        .map(|(p, _)| p)
        .filter(|p| !record.mods.contains_key(&p.id))
        .collect();
    let mut lines = Vec::new();
    let (mut done, mut failed) = (0, 0);
    let total = todo.len() as f32;
    for (i, p) in todo.iter().enumerate() {
        let mut report = |f: f32, text: &str| {
            progress.report((i as f32 + f) / total, &format!("Converting {}: {text}", p.name))
        };
        match convert_one(cx, addons, conv, &library, p, &mut record, &mut report) {
            Ok(_) => done += 1,
            Err(e) => {
                failed += 1;
                lines.push(format!("{} was not converted: {e}", p.name));
            }
        }
        record.save(cx.data_dir)?;
    }
    if done > 0 {
        lines.insert(0, format!("Converted {done} mod(s) from the library into addons"));
    }
    if failed > 0 {
        lines.push(format!(
            "{failed} mod(s) still load through Simplified for Swofty's build step"
        ));
    }
    lines.extend(place(cx, addons, &record)?);
    Ok(lines)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Done {
    pub lines: Vec<String>,
    pub show: Option<String>,
    pub failed: bool,
}

pub fn import(cx: &Cx, paths: &[PathBuf], progress: &dyn Progress, conv: &dyn Convert) -> Result<Done> {
    let mut done = Done {
        lines: migrate(cx, conv, progress)?,
        ..Done::default()
    };
    let library = Library::new(cx.data_dir);
    let known: Vec<String> = library.list()?.into_iter().map(|p| p.id).collect();
    let mut record = Record::load(cx.data_dir)?;
    let mut last = Last {
        at: now(),
        ..Last::default()
    };
    let mut kept = Vec::new();
    let total = paths.len().max(1) as f32;
    for (i, path) in paths.iter().enumerate() {
        let file = package::file_name(path);
        last.files.push(file.clone());
        let mut report =
            |f: f32, text: &str| progress.report((i as f32 + f / 2.) / total, &format!("Importing {text}"));
        let imported = match library.import(path, cx.game, &mut report) {
            Ok(imported) => imported,
            Err(e) => {
                last.errors.push(format!("{file}: {e}"));
                continue;
            }
        };
        if imported.refused > 0 {
            done.lines
                .push(format!("{file}: {} unsafe path(s) left out", imported.refused));
        }
        for p in &imported.packages {
            progress.report((i as f32 + 0.5) / total, &format!("Converting {}", p.name));
            if let Some(Err(e)) = cx.game.map(|g| library.diff(p, g)) {
                done.lines
                    .push(format!("{}: not compared with the game: {e}", p.name));
            }
            last.mods.push(p.id.clone());
            let Some(addons) = cx.addons() else {
                done.lines.push(format!("Imported {}. {UPDATE}", p.name));
                if !known.contains(&p.id) {
                    kept.push(p.id.clone());
                }
                continue;
            };
            let mut report = |f: f32, text: &str| {
                progress.report((i as f32 + 0.5 + f / 2.) / total, &format!("Converting {text}"))
            };
            match convert_one(cx, addons, conv, &library, p, &mut record, &mut report) {
                Ok(made) => {
                    done.lines
                        .push(format!("{} is now the addon {}", p.name, made.name));
                    if !cx.profile.vanilla {
                        if let Err(e) = addons.set_enabled(&made.id, true) {
                            done.lines.push(format!("{}: not turned on: {e}", made.name));
                        }
                        hand_over(cx, &p.id)?;
                    }
                    done.show.get_or_insert(made.id);
                }
                Err(e) => {
                    done.lines.push(format!("{} was not converted: {e}", p.name));
                    if !known.contains(&p.id) {
                        kept.push(p.id.clone());
                    }
                }
            }
        }
    }
    if !kept.is_empty() && !cx.profile.vanilla {
        let mut loaded = Loaded::read(cx)?;
        for id in &kept {
            loaded.set(id, true)?;
        }
        loaded.save(cx)?;
    }
    done.failed = !last.errors.is_empty() && last.mods.is_empty();
    done.lines.splice(0..0, last.errors.iter().cloned());
    record.last = Some(last);
    record.save(cx.data_dir)?;
    Ok(done)
}

// Makes a library mod's addon again from the library copy, replacing the
// installed one, and hands it over in the profile when the profile lists the mod.
pub fn reimport(cx: &Cx, id: &str, conv: &dyn Convert) -> Result<Done> {
    let addons = cx.addons().ok_or_else(|| err(UPDATE.into()))?;
    let library = Library::new(cx.data_dir);
    let p = library.get(id)?;
    let mut record = Record::load(cx.data_dir)?;
    let made = convert_one(cx, addons, conv, &library, &p, &mut record, &mut |_, _| {});
    record.last = Some(Last {
        at: now(),
        files: vec![p.source.clone()],
        mods: vec![p.id.clone()],
        errors: Vec::new(),
    });
    record.save(cx.data_dir)?;
    let made = made?;
    let mut lines = vec![format!("{} is now the addon {}", p.name, made.name)];
    lines.extend(place(cx, addons, &record)?);
    Ok(Done {
        lines,
        show: Some(made.id),
        failed: false,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use serde_json::{Value, json};
    use swofty_addon::launcher::{Addon, AddonInfo, ProfileInfo, StateStore};
    use swofty_addon::view::{Block, Body, Effect, Event};

    use super::*;
    use crate::{ID, IMPORT, NoProgress, SECTION, Simplified, Slot, page};

    #[derive(Default)]
    struct Memory(Mutex<Option<Value>>);

    impl StateStore for Memory {
        fn get(&self, _: &str) -> Result<Option<Value>> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn set(&self, _: &str, value: Value) -> Result<()> {
            *self.0.lock().unwrap() = Some(value);
            Ok(())
        }
    }

    // The launcher's addons: installed folders under `root`, shared by every
    // profile, and each profile's addon list in load order, as core keeps it.
    struct Addons<'a> {
        root: &'a Path,
        order: &'a Mutex<BTreeMap<String, Vec<(String, bool)>>>,
        profile: String,
        calls: &'a Mutex<Vec<String>>,
    }

    impl AddonStore for Addons<'_> {
        fn list(&self) -> Result<Vec<AddonInfo>> {
            let order = self.order.lock().unwrap();
            let on = order.get(&self.profile).cloned().unwrap_or_default();
            let mut out = Vec::new();
            for dir in std::fs::read_dir(self.root).into_iter().flatten().flatten() {
                let id = dir.file_name().to_string_lossy().into_owned();
                out.push(AddonInfo {
                    enabled: on.iter().any(|(a, e)| *a == id && *e),
                    name: id.clone(),
                    version: "1.0.0".into(),
                    author: "Team Simplified".into(),
                    dir: dir.path(),
                    id,
                });
            }
            Ok(out)
        }

        fn install(&self, path: &Path) -> Result<AddonInfo> {
            let m: Value = serde_json::from_slice(&std::fs::read(path.join("swofty.json")).unwrap()).unwrap();
            let id = m["id"].as_str().unwrap().to_owned();
            let dest = self.root.join(&id);
            std::fs::remove_dir_all(&dest).ok();
            std::fs::create_dir_all(&dest).unwrap();
            std::fs::copy(path.join("swofty.json"), dest.join("swofty.json")).unwrap();
            self.calls.lock().unwrap().push(format!("install {id}"));
            Ok(self.list()?.into_iter().find(|a| a.id == id).unwrap())
        }

        fn set_enabled(&self, id: &str, on: bool) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{} {id}", if on { "on" } else { "off" }));
            let mut order = self.order.lock().unwrap();
            let list = order.entry(self.profile.clone()).or_default();
            match list.iter_mut().find(|(a, _)| a == id) {
                Some(slot) => slot.1 = on,
                None => list.push((id.into(), on)),
            }
            Ok(())
        }

        fn remove(&self, id: &str) -> Result<()> {
            std::fs::remove_dir_all(self.root.join(id)).unwrap();
            Ok(())
        }
    }

    // Writes an addon for each mod, refusing the mods named in `refuse`.
    #[derive(Default)]
    struct Fake {
        refuse: Vec<&'static str>,
        made: Mutex<Vec<String>>,
    }

    impl Convert for Fake {
        fn convert(&self, _: &Cx, _: &Library, p: &Package, out: &Path, _: Report) -> Result<Made> {
            self.made.lock().unwrap().push(p.id.clone());
            if self.refuse.contains(&p.id.as_str()) {
                return Err(err("its archives could not be read".into()));
            }
            let id = format!("imported.{}", p.id);
            let manifest = json!({"schema_version": 1, "id": id, "name": p.name, "version": "1.0.0",
                "author": "Team Simplified", "category": "liveries"});
            std::fs::write(out.join("swofty.json"), manifest.to_string()).unwrap();
            Ok(Made {
                id,
                name: p.name.clone(),
                category: Some(Category::Liveries),
                kinds: vec!["archives".into()],
                lines: vec![format!("2025_asset_groups/{}.erp: 1 resource", p.id)],
                warnings: Vec::new(),
            })
        }
    }

    struct Host {
        root: PathBuf,
        installed: PathBuf,
        data: PathBuf,
        order: Mutex<BTreeMap<String, Vec<(String, bool)>>>,
        calls: Mutex<Vec<String>>,
        states: BTreeMap<String, Memory>,
        profiles: BTreeMap<String, ProfileInfo>,
    }

    impl Host {
        fn new(name: &str, profiles: &[&str]) -> Self {
            let root = std::env::temp_dir().join(format!("swofty-importer-{name}-{}", std::process::id()));
            std::fs::remove_dir_all(&root).ok();
            std::fs::create_dir_all(root.join("addons")).unwrap();
            Self {
                data: root.join("data"),
                installed: root.join("addons"),
                root,
                order: Mutex::default(),
                calls: Mutex::default(),
                states: profiles
                    .iter()
                    .map(|p| (p.to_string(), Memory::default()))
                    .collect(),
                profiles: profiles
                    .iter()
                    .map(|p| {
                        let info = ProfileInfo {
                            name: p.to_string(),
                            vanilla: false,
                            enabled: true,
                        };
                        (p.to_string(), info)
                    })
                    .collect(),
            }
        }

        fn addons(&self, profile: &str) -> Addons<'_> {
            Addons {
                root: &self.installed,
                order: &self.order,
                profile: profile.into(),
                calls: &self.calls,
            }
        }

        fn mod_dir(&self, name: &str) -> PathBuf {
            let dir = self.root.join("src").join(name);
            let erp = dir.join("F1 25/2025_asset_groups").join(format!("{name}.erp"));
            std::fs::create_dir_all(erp.parent().unwrap()).unwrap();
            std::fs::write(&erp, "erp").unwrap();
            std::fs::write(
                dir.join(crate::serps::FILE),
                format!(r#"{{"mod_name":"{name}"}}"#),
            )
            .unwrap();
            dir
        }

        fn order(&self, profile: &str) -> Vec<(String, bool)> {
            self.order
                .lock()
                .unwrap()
                .get(profile)
                .cloned()
                .unwrap_or_default()
        }

        fn calls(&self) -> Vec<String> {
            std::mem::take(&mut *self.calls.lock().unwrap())
        }
    }

    fn with<T>(host: &Host, profile: &str, f: impl FnOnce(&Cx) -> T) -> T {
        let addons = host.addons(profile);
        let cx = Cx::new(
            ID,
            &host.profiles[profile],
            &host.data,
            &host.root,
            None,
            &host.states[profile],
        )
        .with_addons(&addons);
        f(&cx)
    }

    fn slot(id: &str, enabled: bool) -> Slot {
        Slot {
            id: id.into(),
            enabled,
        }
    }

    fn on(id: &str, enabled: bool) -> (String, bool) {
        (format!("imported.{id}"), enabled)
    }

    #[test]
    fn the_library_converts_once_and_each_profile_keeps_its_mod_order() {
        let host = Host::new("migrate", &["one", "two"]);
        let library = Library::new(&host.data);
        for name in ["a", "b", "c", "d"] {
            library.import(&host.mod_dir(name), None, &mut |_, _| {}).unwrap();
        }
        let first = vec![
            slot("c", true),
            slot("a", false),
            slot("d", true),
            slot("b", false),
        ];
        let second = vec![slot("d", true), slot("b", true), slot("a", true)];
        with(&host, "one", |cx| {
            cx.set_state(&State {
                mods: first.clone(),
                addons: Vec::new(),
            })
        })
        .unwrap();
        with(&host, "two", |cx| {
            cx.set_state(&State {
                mods: second.clone(),
                addons: Vec::new(),
            })
        })
        .unwrap();
        let conv = Fake {
            refuse: vec!["b"],
            ..Fake::default()
        };

        let todo = with(&host, "one", |cx| {
            pending(cx, &Record::load(cx.data_dir).unwrap())
        })
        .unwrap();
        assert_eq!(todo, Pending { convert: 4, place: 0 });
        let lines = with(&host, "one", |cx| migrate(cx, &conv, &NoProgress)).unwrap();
        assert_eq!(lines[0], "Converted 3 mod(s) from the library into addons");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("b was not converted: its archives could not be read"))
        );
        assert_eq!(*conv.made.lock().unwrap(), ["c", "a", "d", "b"]);
        assert_eq!(
            host.calls(),
            [
                "install imported.c",
                "install imported.a",
                "install imported.d",
                "on imported.c",
                "off imported.a",
                "on imported.d"
            ]
        );
        assert_eq!(host.order("one"), [on("c", true), on("a", false), on("d", true)]);
        let handed = with(&host, "one", handed_over).unwrap();
        assert_eq!(
            handed,
            HashSet::from(["a".to_owned(), "c".to_owned(), "d".to_owned()])
        );

        let todo = with(&host, "two", |cx| {
            pending(cx, &Record::load(cx.data_dir).unwrap())
        })
        .unwrap();
        assert_eq!(todo, Pending { convert: 0, place: 2 });
        with(&host, "two", |cx| migrate(cx, &conv, &NoProgress)).unwrap();
        assert_eq!(conv.made.lock().unwrap().len(), 4, "nothing converts twice");
        assert_eq!(host.order("two"), [on("d", true), on("a", true)]);
        assert_eq!(host.calls(), ["on imported.d", "on imported.a"]);

        with(&host, "one", |cx| migrate(cx, &conv, &NoProgress)).unwrap();
        assert!(host.calls().is_empty(), "a profile is handed over once");

        let addon = Simplified::new();
        let summary = with(&host, "two", |cx| addon.summary(cx)).unwrap();
        assert_eq!(summary.items, ["b"], "only b still loads through the build hook");
        assert!(with(&host, "one", |cx| addon.summary(cx)).is_none());
        let record = Record::load(&host.data).unwrap();
        assert!(record.addon("b").is_none());
        assert!(
            record.mods["b"]
                .error
                .as_deref()
                .unwrap()
                .contains("could not be read")
        );
        assert!(
            library.dir("b").is_dir() && library.dir("a").is_dir(),
            "the library stays"
        );
        std::fs::remove_dir_all(&host.root).ok();
    }

    fn cards(view: &swofty_addon::view::View) -> Vec<String> {
        match &view.body {
            Body::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Card(c) => c.title.clone(),
                    Block::Hero(h) => Some(h.title.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn an_import_installs_the_addon_turns_it_on_and_shows_it() {
        let host = Host::new("import", &["one"]);
        let livery = host.mod_dir("livery");
        let conv = Fake::default();
        let addon = Simplified::new();
        let pick = |paths: Vec<PathBuf>| Event::Picked {
            id: IMPORT.into(),
            paths,
        };

        let effect = with(&host, "one", |cx| {
            addon.event_with(cx, SECTION, pick(vec![livery.clone()]), &conv)
        })
        .unwrap();
        assert_eq!(effect, Effect::ShowAddon("imported.livery".into()));
        assert_eq!(
            serde_json::to_value(&effect).unwrap(),
            json!({"show_addon": "imported.livery"})
        );
        assert_eq!(host.calls(), ["install imported.livery", "on imported.livery"]);
        assert_eq!(host.order("one"), [on("livery", true)]);
        let record = Record::load(&host.data).unwrap();
        let last = record.last.clone().unwrap();
        assert_eq!(
            (last.files, last.mods),
            (vec!["livery".to_owned()], vec!["livery".to_owned()])
        );
        assert_eq!(
            record.addon("livery").unwrap().lines,
            ["2025_asset_groups/livery.erp: 1 resource"]
        );
        assert!(with(&host, "one", handed_over).unwrap().contains("livery"));

        let view = with(&host, "one", |cx| addon.view(cx, SECTION)).unwrap();
        assert_eq!(
            cards(&view),
            [
                "Mod Importer",
                "Import a mod",
                "Last import",
                "Converted from your library"
            ]
        );
        assert!(view.notices.is_empty(), "{:?}", view.notices);

        let effect = with(&host, "one", |cx| {
            addon.event_with(cx, SECTION, pick(vec![livery.clone()]), &conv)
        })
        .unwrap();
        assert_eq!(effect, Effect::ShowAddon("imported.livery".into()));
        assert_eq!(host.calls(), ["install imported.livery", "on imported.livery"]);
        assert_eq!(host.order("one").len(), 1, "a re-import replaces its addon");
        assert_eq!(
            with(&host, "one", |cx| cx.addons().unwrap().list())
                .unwrap()
                .len(),
            1
        );

        let click = |id: String| Event::Click { id };
        let effect = with(&host, "one", |cx| {
            addon.event_with(cx, SECTION, click(format!("{}livery", page::REIMPORT)), &conv)
        })
        .unwrap();
        assert_eq!(effect, Effect::ShowAddon("imported.livery".into()));
        assert_eq!(
            host.calls(),
            ["install imported.livery"],
            "re-import keeps the switch"
        );
        let effect = with(&host, "one", |cx| {
            addon.event_with(
                cx,
                SECTION,
                click(format!("{}imported.livery", page::SHOW_ADDON)),
                &conv,
            )
        })
        .unwrap();
        assert_eq!(effect, Effect::ShowAddon("imported.livery".into()));

        let effect = with(&host, "one", |cx| {
            addon.event_with(cx, SECTION, click(page::TEAM.into()), &conv)
        })
        .unwrap();
        assert_eq!(effect, Effect::Reload);
        let view = with(&host, "one", |cx| addon.view(cx, SECTION)).unwrap();
        assert!(cards(&view).contains(&"Team Simplified".to_owned()));
        let effect = with(&host, "one", |cx| {
            addon.event_with(cx, SECTION, click(format!("{}suite", page::LINK)), &conv)
        })
        .unwrap();
        assert_eq!(
            effect,
            Effect::Open("https://github.com/TeamSimplified/simplified-suite".into())
        );
        std::fs::remove_dir_all(&host.root).ok();
    }

    #[test]
    fn a_mod_that_does_not_convert_stays_on_through_the_build_hook() {
        let host = Host::new("refuse", &["one"]);
        let logos = host.mod_dir("logos");
        let conv = Fake {
            refuse: vec!["logos"],
            ..Fake::default()
        };
        let addon = Simplified::new();
        let effect = with(&host, "one", |cx| {
            addon.event_with(
                cx,
                SECTION,
                Event::Picked {
                    id: IMPORT.into(),
                    paths: vec![logos],
                },
                &conv,
            )
        })
        .unwrap();
        assert!(
            matches!(&effect, Effect::Notice(t) if t.contains("logos was not converted")),
            "{effect:?}"
        );
        assert!(host.calls().is_empty());
        let state: State = with(&host, "one", |cx| cx.state()).unwrap();
        assert_eq!(state.mods, [slot("logos", true)]);
        assert_eq!(
            with(&host, "one", |cx| addon.summary(cx)).unwrap().items,
            ["logos"]
        );
        let view = with(&host, "one", |cx| addon.view(cx, SECTION)).unwrap();
        assert!(cards(&view).contains(&"Not converted".to_owned()));
        assert!(
            view.notices.is_empty(),
            "a failed mod is not pending: {:?}",
            view.notices
        );
        std::fs::remove_dir_all(&host.root).ok();
    }
}
