// The Simplified Importer: every imported mod becomes a standalone Swofty data addon,
// installed and switched through the Launcher API's addon host calls. A mod is unpacked
// into the library only while it converts; once it is an addon, its copy is deleted.
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use swofty_addon::launcher::{AddonStore, Cx, Progress, TaskItem, TaskList, TaskState};
use swofty_addon::manifest::{AddonManifest, Category, Kind, MANIFEST_FILE};

use crate::library::{Library, Package};
use crate::{Loaded, Result, Slot, State, err, io_err, package};

pub const RECORD: &str = "importer.json";
// The record's fields that belong to this machine, beside the record. Hidden, so the
// launcher's multiplayer fingerprint leaves it out.
pub const LOCAL: &str = ".importer-local.json";
const STAGING: &str = ".convert";
pub const UPDATE: &str = "Update Swofty Launcher to 1.3.0 or later to turn mods into addons.";
pub const NO_GAME: &str = "Mods turn into addons once Swofty Launcher finds F1 25.";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Made {
    pub id: String,
    pub name: String,
    #[serde(
        default,
        deserialize_with = "Category::read",
        skip_serializing_if = "Option::is_none"
    )]
    pub category: Option<Category>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    // It needs the SERPs Base Files, which the build hook loads while it is on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub base: bool,
}

pub type Report<'a> = &'a mut dyn FnMut(f32, &str);

pub trait Convert {
    fn convert(
        &self,
        cx: &Cx,
        library: &Library,
        p: &Package,
        id: &str,
        out: &Path,
        progress: Report,
    ) -> Result<Made>;
}

pub struct Converter;

impl Convert for Converter {
    fn convert(
        &self,
        cx: &Cx,
        library: &Library,
        p: &Package,
        id: &str,
        out: &Path,
        progress: Report,
    ) -> Result<Made> {
        let c = crate::convert::convert_to(library, p, id, cx.game, out, progress)?;
        Ok(Made {
            id: c.id,
            name: c.name,
            category: c.category,
            kinds: c.kinds.into_iter().map(|k| kind(k).into()).collect(),
            summary: c.summary,
            base: c.base,
            warnings: c
                .report
                .iter()
                .filter_map(|l| l.strip_prefix("warning:").map(|w| w.trim().to_owned()))
                .collect(),
            lines: c
                .report
                .into_iter()
                .filter(|l| !l.starts_with("warning:"))
                .collect(),
        })
    }
}

fn kind(k: Kind) -> &'static str {
    match k {
        Kind::Database => "Database",
        Kind::Erp => "Archives",
        Kind::Text => "Text",
        Kind::Ui => "UI",
        Kind::Files => "Files",
        Kind::Native => "Code",
        Kind::Simplified => "Simplified",
    }
}

// Raised when converting the same mod gives a different addon. 2: removals only
// apply to a mod made for the installed game version. 3: shader archive resources
// load from the mod's own boot overlay again.
pub const CONVERTER: u32 = 3;

// `last`, and each entry's `at` and `error`, are written to `LOCAL`; an older record
// holds them itself.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub mods: BTreeMap<String, Entry>,
    #[serde(default, skip_serializing)]
    pub last: Option<Last>,
}

// One library mod: the addon made from it, and the reason the last conversion
// failed. A failed conversion of a mod made before keeps the addon made before.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    #[serde(default, skip_serializing)]
    pub at: u64,
    #[serde(default)]
    pub converter: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made: Option<Made>,
    #[serde(default, skip_serializing)]
    pub error: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Local {
    #[serde(default)]
    mods: BTreeMap<String, LocalEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last: Option<Last>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LocalEntry {
    #[serde(default)]
    at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Last {
    pub at: u64,
    pub files: Vec<String>,
    #[serde(default)]
    pub mods: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variants>,
}

// Parts of one mod that write mostly the same things: alternatives, all installed
// and only `on` turned on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Variants {
    pub of: String,
    pub mods: Vec<String>,
    pub on: String,
}

impl Variants {
    pub fn line(&self, record: &Record) -> String {
        let on = record
            .mods
            .get(&self.on)
            .and_then(|e| e.made.as_ref())
            .map_or(self.on.as_str(), |m| m.name.as_str());
        let on = on.strip_suffix(&format!(" — {}", self.of)).unwrap_or(on);
        format!(
            "{}: {} variants installed as addons; {on} is on. Pick another on Addons.",
            self.of,
            self.mods.len()
        )
    }
}

impl Record {
    // A record that holds this machine's fields itself was written by an older version,
    // after `LOCAL` if there is one, so `LOCAL` is then left unread.
    pub fn load(data: &Path) -> Result<Self> {
        let path = data.join(RECORD);
        let mut record: Self = match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| err(format!("{}: {e}", path.display())))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(io_err(&path)(e)),
        };
        let older = record.last.is_some() || record.mods.values().any(|e| e.at != 0 || e.error.is_some());
        let local = std::fs::read(data.join(LOCAL))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Local>(&bytes).ok());
        if let Some(local) = local.filter(|_| !older) {
            for (id, l) in local.mods {
                if let Some(e) = record.mods.get_mut(&id) {
                    e.at = l.at;
                    e.error = l.error;
                }
            }
            record.last = local.last;
        }
        Ok(record)
    }

    // `LOCAL` first: a record left half saved still holds the fields `LOCAL` lacks.
    pub fn save(&self, data: &Path) -> Result<()> {
        std::fs::create_dir_all(data).map_err(io_err(data))?;
        let local = Local {
            mods: self
                .mods
                .iter()
                .map(|(id, e)| {
                    let l = LocalEntry {
                        at: e.at,
                        error: e.error.clone(),
                    };
                    (id.clone(), l)
                })
                .collect(),
            last: self.last.clone(),
        };
        write_json(data, LOCAL, &local)?;
        write_json(data, RECORD, self)
    }

    pub fn addon(&self, id: &str) -> Option<&Made> {
        self.mods.get(id).and_then(|e| e.made.as_ref())
    }
}

fn write_json(data: &Path, name: &str, value: &impl Serialize) -> Result<()> {
    let path = data.join(name);
    let tmp = data.join(format!("{name}.tmp"));
    let text = serde_json::to_vec_pretty(value).map_err(|e| err(format!("{}: {e}", path.display())))?;
    std::fs::write(&tmp, text).map_err(io_err(&tmp))?;
    std::fs::rename(&tmp, &path).map_err(io_err(&path))
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
    let convert = library.iter().filter(|p| due(cx, record, p)).count();
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

// A mod converts when it has no addon yet, or when an older converter made its addon:
// one before 3 merged shader archive resources into the game's shader archives, which
// any mod may carry.
fn due(cx: &Cx, record: &Record, p: &Package) -> bool {
    match record.mods.get(&p.id) {
        None => true,
        Some(e) => e.made.is_some() && p.track.is_none() && cx.game.is_some() && e.converter < CONVERTER,
    }
}

// A mod converts by comparing it with the game's own files; a track package needs none.
fn convertible(cx: &Cx, p: &Package) -> bool {
    cx.game.is_some() || p.track.is_some()
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
        let id = record
            .addon(&p.id)
            .map_or_else(|| crate::convert::addon_id(p), |m| m.id.clone());
        let mut made = conv.convert(cx, library, p, &id, &out, progress)?;
        made.id = addons.install(&out)?.id;
        Ok(made)
    })();
    std::fs::remove_dir_all(&out).ok();
    if made.is_ok() {
        library.remove(&p.id).ok();
    }
    let entry = record.mods.entry(p.id.clone()).or_default();
    entry.name = p.name.clone();
    entry.at = now();
    match &made {
        Ok(m) => {
            entry.made = Some(m.clone());
            entry.converter = CONVERTER;
            entry.error = None;
        }
        Err(e) => entry.error = Some(e.to_string()),
    }
    made
}

// Turns on, in the profile's mod order, the addons made from the mods the profile
// lists, as each mod was on or off, then moves them to where the mods loaded.
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
        settle(cx, addons, record, &state.mods, &placed);
        state.addons.extend(placed);
        cx.set_state(&state)?;
    }
    Ok(lines)
}

// The active profile's addon ids in load order, from each addon's position; a
// launcher before 1.3.0 gives none, and its profile file two folders above this
// addon's data folder is read instead.
fn profile_order(cx: &Cx, addons: &dyn AddonStore) -> Option<Vec<String>> {
    let placed: Vec<(usize, String)> = addons
        .list()
        .ok()?
        .into_iter()
        .filter_map(|a| Some((a.position?, a.id)))
        .collect();
    if let Some(len) = placed.iter().map(|(at, _)| at + 1).max() {
        let mut order = vec![String::new(); len];
        for (at, id) in placed {
            order[at] = id;
        }
        return Some(order);
    }
    let path = cx
        .data_dir
        .parent()?
        .parent()?
        .join("profiles")
        .join(format!("{}.json", cx.profile.name));
    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let ids = doc["addons"].as_array()?.iter();
    Some(ids.filter_map(|a| Some(a["id"].as_str()?.to_owned())).collect())
}

// Moves each placed addon to just after the importer, or after the addon of the
// mod before it in the profile's mod order: where the build hook loaded the mods.
// Set in order, the addons are already in mod order at the end of the list, which
// is where they stay on a launcher without move_addon.
fn settle(cx: &Cx, addons: &dyn AddonStore, record: &Record, mods: &[Slot], placed: &[String]) {
    let Some(mut order) = profile_order(cx, addons) else {
        return;
    };
    for id in placed {
        let (Some(made), Some(j)) = (record.addon(id), mods.iter().position(|s| &s.id == id)) else {
            continue;
        };
        let before: Vec<&str> = mods[..j]
            .iter()
            .filter_map(|s| record.addon(&s.id))
            .map(|m| m.id.as_str())
            .collect();
        let Some(from) = order.iter().position(|a| *a == made.id) else {
            continue;
        };
        let item = order.remove(from);
        let after = order
            .iter()
            .rposition(|a| a == crate::ID || before.contains(&a.as_str()));
        let to = after.map_or(from, |i| i + 1);
        order.insert(to, item);
        if to != from && addons.move_addon(&made.id, to as isize - from as isize).is_err() {
            return;
        }
    }
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
    Ok(migrated(cx, conv, progress, None)?.lines)
}

// The launcher's budget for an event is 300 seconds. A startup conversion starts no
// mod it expects to end past this, leaving the rest for the next start.
pub const BUDGET: Duration = Duration::from_secs(270);
const BYTES_PER_SEC: u64 = 40 << 20;

// Deletes the library copies of mods that are addons now and convert no more.
fn drop_converted(cx: &Cx, library: &Library, record: &Record) -> Result<()> {
    for p in library.list()? {
        if record.addon(&p.id).is_some() && !due(cx, record, &p) {
            library.remove(&p.id).ok();
        }
    }
    Ok(())
}

// Mods converted before 0.10.0 were flagged for the base files only as Studio or My Team
// mods: each is flagged from its import report. 0.10.0 gave some addons another
// origin.tool; each gets ID back, and loses the "Converted from …" prefix of earlier
// descriptions. Each gets its category from what its description says it replaces, as
// convert does from its shader packages: none for a mod that uses none. Each mod
// package's addon gets the types convert gives it now (convert::package_kinds).
fn relabel(cx: &Cx, mut record: Record) -> Result<Record> {
    let Some(addons) = cx.addons() else {
        return Ok(record);
    };
    let dirs: BTreeMap<String, PathBuf> = addons.list()?.into_iter().map(|a| (a.id, a.dir)).collect();
    let mut changed = false;
    for made in record.mods.values_mut().filter_map(|e| e.made.as_mut()) {
        let Some(dir) = dirs.get(&made.id) else { continue };
        if !made.base {
            let report = std::fs::read_to_string(dir.join(crate::convert::REPORT)).unwrap_or_default();
            made.base = report.contains(crate::convert::CARRIED);
            changed |= made.base;
        }
        let path = dir.join(MANIFEST_FILE);
        let Ok(text) = std::fs::read(&path) else { continue };
        let Ok(mut m) = serde_json::from_slice::<AddonManifest>(&text) else {
            continue;
        };
        let Some(origin) = m.origin.as_mut() else { continue };
        let mut dirty = origin.tool == crate::COPY_PASTE;
        origin.tool = crate::ID.into();
        // "Converted from <file> by the Simplified Importer: <counts>." keeps its counts.
        let described: Option<String> = m.description.as_deref().and_then(|d| {
            let d = d.strip_prefix("Converted from ")?;
            let rest = d
                .split_once(" by the Simplified Importer")
                .map_or(d, |(_, rest)| rest);
            Some(rest.trim_start_matches([':', '.']).trim().to_owned())
        });
        if let Some(d) = described {
            m.description = (!d.is_empty()).then_some(d);
            dirty = true;
        }
        let track = dir.join(crate::track::TRACK).exists();
        if !track {
            let kinds = crate::convert::package_kinds(
                m.kind.contains(&Kind::Database),
                m.kind.contains(&Kind::Text),
                made.base,
            );
            if m.kind != kinds {
                m.kind = kinds;
                dirty = true;
            }
            let labels: Vec<String> = m.kind.iter().map(|k| kind(*k).into()).collect();
            if made.kinds != labels {
                made.kinds = labels;
                changed = true;
            }
        }
        let category = match (track, made.base) {
            (true, _) => Some(Category::Tracks),
            (false, true) => m
                .description
                .as_deref()
                .and_then(crate::merge::category_described),
            (false, false) => None,
        };
        if m.category != category {
            m.category = category;
            dirty = true;
        }
        if made.category != category {
            made.category = category;
            changed = true;
        }
        if dirty {
            let json = serde_json::to_vec_pretty(&m).expect("serialisable");
            std::fs::write(&path, json).map_err(io_err(&path))?;
        }
    }
    if changed {
        record.save(cx.data_dir)?;
    }
    Ok(record)
}

// Turns the whole archives of addons made while the game lacked them into overlays
// (convert::rediff). An addon that fails keeps its archive and says why once.
fn rediff(cx: &Cx, mut record: Record) -> Result<Record> {
    let (Some(addons), Some(game)) = (cx.addons(), cx.game) else {
        return Ok(record);
    };
    let dirs: BTreeMap<String, PathBuf> = addons.list()?.into_iter().map(|a| (a.id, a.dir)).collect();
    let mut changed = false;
    for made in record.mods.values_mut().filter_map(|e| e.made.as_mut()) {
        let Some(dir) = dirs.get(&made.id) else { continue };
        match crate::convert::rediff(dir, game) {
            Ok(lines) if lines.is_empty() => {}
            Ok(lines) => {
                if let Ok(m) = AddonManifest::load(dir) {
                    made.kinds = m.kind.iter().map(|k| kind(*k).to_owned()).collect();
                }
                made.lines.extend(lines);
                changed = true;
            }
            Err(e) => {
                let warning = format!("its whole archives stay in the addon: {e}");
                if !made.warnings.contains(&warning) {
                    made.warnings.push(warning);
                    changed = true;
                }
            }
        }
    }
    if changed {
        record.save(cx.data_dir)?;
    }
    Ok(record)
}

// The startup event: converts what is pending within the budget, and says so.
pub fn startup(
    cx: &Cx,
    conv: &dyn Convert,
    progress: &dyn Progress,
    budget: Duration,
) -> Result<Option<String>> {
    if cx.addons().is_none() {
        return Ok(None);
    }
    let record = rediff(cx, relabel(cx, Record::load(cx.data_dir)?)?)?;
    drop_converted(cx, &Library::new(cx.data_dir), &record)?;
    if !pending(cx, &record)?.any() {
        return Ok(None);
    }
    let m = migrated(cx, conv, progress, Some(budget))?;
    let mut parts = Vec::new();
    match m.converted.len() {
        0 => {}
        1 => parts.push("Converted 1 mod into an addon".to_owned()),
        n => parts.push(format!("Converted {n} mods into addons")),
    }
    if !m.failed.is_empty() {
        parts.push(format!("not converted: {}", m.failed.join(", ")));
    }
    if m.left > 0 {
        parts.push(format!(
            "{} more convert at the next start, or with Convert now on the Simplified Importer",
            m.left
        ));
    }
    Ok((!parts.is_empty()).then(|| parts.join("; ")))
}

// The launcher's checklist of the mods a migration converts, sent whole each time
// one of them changes. A launcher without the dialog drops it.
struct Tasks {
    list: TaskList,
}

impl Tasks {
    fn new(todo: &[Package]) -> Self {
        Self {
            list: TaskList {
                title: "Migrating your mods to Swofty addons".into(),
                subtitle: "Swofty 1.3.0 turns every mod into an addon. Your library stays as it is.".into(),
                items: todo
                    .iter()
                    .map(|p| TaskItem {
                        id: p.id.clone(),
                        label: p.name.clone(),
                        state: TaskState::Waiting,
                        note: None,
                    })
                    .collect(),
            },
        }
    }

    fn send(&self, cx: &Cx) {
        if !self.list.items.is_empty() {
            cx.tasks(&self.list);
        }
    }

    fn set(&mut self, cx: &Cx, i: usize, state: TaskState, note: Option<String>) {
        self.list.items[i].state = state;
        self.list.items[i].note = note;
        self.send(cx);
    }
}

fn short(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    match line.char_indices().nth(120) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_owned(),
    }
}

#[derive(Default)]
struct Migrated {
    lines: Vec<String>,
    converted: Vec<Package>,
    failed: Vec<String>,
    left: usize,
}

fn migrated(
    cx: &Cx,
    conv: &dyn Convert,
    progress: &dyn Progress,
    budget: Option<Duration>,
) -> Result<Migrated> {
    let Some(addons) = cx.addons() else {
        return Ok(Migrated::default());
    };
    let started = Instant::now();
    let library = Library::new(cx.data_dir);
    let mut record = Record::load(cx.data_dir)?;
    let todo: Vec<Package> = Loaded::read(cx)?
        .order
        .into_iter()
        .map(|(p, _)| p)
        .filter(|p| due(cx, &record, p) && convertible(cx, p))
        .collect();
    let mut m = Migrated::default();
    let total = todo.len() as f32;
    let mut tried = Vec::new();
    let mut tasks = Tasks::new(&todo);
    tasks.send(cx);
    for (i, p) in todo.iter().enumerate() {
        let expected = Duration::from_secs(p.size() / BYTES_PER_SEC);
        if budget.is_some_and(|b| !tried.is_empty() && started.elapsed() + expected > b) {
            m.left = todo.len() - i;
            for item in &mut tasks.list.items[i..] {
                item.note = Some("converts on next start".into());
            }
            tasks.send(cx);
            break;
        }
        tried.push(p.id.clone());
        tasks.set(cx, i, TaskState::Running, None);
        let mut report = |f: f32, text: &str| {
            progress.report((i as f32 + f) / total, &format!("Converting {}: {text}", p.name))
        };
        match convert_one(cx, addons, conv, &library, p, &mut record, &mut report) {
            Ok(made) => {
                m.converted.push(p.clone());
                let note = made.category.map(|c| c.label().to_owned());
                tasks.set(cx, i, TaskState::Done, note);
            }
            Err(e) => {
                m.failed.push(p.name.clone());
                m.lines.push(format!("{} was not converted: {e}", p.name));
                tasks.set(cx, i, TaskState::Failed, Some(short(&e.to_string())));
            }
        }
        record.save(cx.data_dir)?;
    }
    if !m.converted.is_empty() {
        m.lines.insert(
            0,
            format!(
                "Converted {} from the library into addons",
                crate::plural(m.converted.len(), "mod", "mods")
            ),
        );
    }
    if !m.failed.is_empty() {
        m.lines.push(format!(
            "{} still {} through the Simplified Importer's build step",
            crate::plural(m.failed.len(), "mod", "mods"),
            if m.failed.len() == 1 { "loads" } else { "load" }
        ));
    }
    if m.left > 0 {
        m.lines.push(format!(
            "{} more convert at the next start",
            crate::plural(m.left, "mod", "mods")
        ));
    }
    drop_converted(cx, &library, &record)?;
    if !tried.is_empty() {
        record.last = Some(Last {
            at: now(),
            mods: tried,
            ..Last::default()
        });
        record.save(cx.data_dir)?;
    }
    m.lines.extend(place(cx, addons, &record)?);
    Ok(m)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Done {
    pub lines: Vec<String>,
    pub show: Option<String>,
    pub failed: bool,
    pub variants: bool,
}

// Unpacks each mod, converts it and installs the addon, then deletes the unpacked
// copy. A mod that can't convert is not kept either.
pub fn import(cx: &Cx, paths: &[PathBuf], progress: &dyn Progress, conv: &dyn Convert) -> Result<Done> {
    let Migrated {
        lines,
        converted: fresh,
        ..
    } = migrated(cx, conv, progress, None)?;
    let mut done = Done {
        lines,
        ..Done::default()
    };
    let library = Library::new(cx.data_dir);
    let mut record = Record::load(cx.data_dir)?;
    let mut last = Last {
        at: now(),
        ..Last::default()
    };
    let mut made_any = false;
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
            done.lines.push(format!(
                "{file}: {} left out",
                crate::plural(imported.refused, "unsafe path", "unsafe paths")
            ));
        }
        let groups = variants(cx, &library, &record, &imported.packages);
        for p in &imported.packages {
            progress.report((i as f32 + 0.5) / total, &format!("Converting {}", p.name));
            last.mods.push(p.id.clone());
            let Some(addons) = cx.addons().filter(|_| convertible(cx, p)) else {
                let why = if cx.addons().is_none() { UPDATE } else { NO_GAME };
                last.errors.push(format!("{} was not imported. {why}", p.name));
                library.remove(&p.id).ok();
                continue;
            };
            let mut report = |f: f32, text: &str| {
                progress.report((i as f32 + 0.5 + f / 2.) / total, &format!("Converting {text}"))
            };
            let same = |q: &Package| {
                q.id == p.id
                    && match (&q.sha256, &p.sha256) {
                        (Some(a), Some(b)) => a == b,
                        (None, None) => q.files == p.files,
                        _ => false,
                    }
            };
            let made = match (fresh.iter().any(same), record.addon(&p.id)) {
                (true, Some(made)) => Ok(made.clone()),
                _ => convert_one(cx, addons, conv, &library, p, &mut record, &mut report),
            };
            let group = groups.iter().find(|g| g.mods.contains(&p.id));
            let on = group.is_none_or(|g| g.on == p.id);
            match made {
                Ok(made) => {
                    made_any = true;
                    if group.is_none() {
                        done.lines
                            .push(format!("{} is now the addon {}", p.name, made.name));
                    }
                    done.lines
                        .extend(made.warnings.iter().map(|w| format!("{}: {w}", made.name)));
                    if !cx.profile.vanilla {
                        if let Err(e) = addons.set_enabled(&made.id, on) {
                            done.lines.push(format!("{}: not switched: {e}", made.name));
                        }
                        hand_over(cx, &p.id)?;
                    }
                    if on {
                        done.show.get_or_insert(made.id);
                    }
                }
                Err(e) => {
                    last.errors.push(format!("{} was not converted: {e}", p.name));
                    library.remove(&p.id).ok();
                }
            }
        }
        for g in groups {
            done.variants = true;
            done.lines.push(g.line(&record));
            last.variants.push(g);
        }
    }
    done.failed = !last.errors.is_empty() && !made_any;
    done.lines.splice(0..0, last.errors.iter().cloned());
    record.last = Some(last);
    record.save(cx.data_dir)?;
    Ok(done)
}

const OVERLAP: f64 = 0.5;

// Parts of one mod that write mostly the same records, resources, text and files
// are alternatives. The one on is the one on before a re-import, else the first
// named Default or Standard, else the first.
fn variants(cx: &Cx, library: &Library, record: &Record, packages: &[Package]) -> Vec<Variants> {
    let Some(game) = cx.game.filter(|_| packages.len() > 1) else {
        return Vec::new();
    };
    let targets: Vec<HashSet<String>> = packages
        .iter()
        .map(|p| match p.track {
            Some(_) => HashSet::new(),
            None => library
                .diff(p, game)
                .map(|d| crate::merge::targets(&d).into_iter().collect())
                .unwrap_or_default(),
        })
        .collect();
    let mut group: Vec<usize> = (0..packages.len()).collect();
    fn root(group: &mut [usize], mut i: usize) -> usize {
        while group[i] != i {
            group[i] = group[group[i]];
            i = group[i];
        }
        i
    }
    for a in 0..packages.len() {
        for b in a + 1..packages.len() {
            let (x, y) = (&targets[a], &targets[b]);
            let shared = x.intersection(y).count();
            let all = x.len() + y.len() - shared;
            if shared > 0 && shared as f64 >= OVERLAP * all as f64 {
                let (ra, rb) = (root(&mut group, a), root(&mut group, b));
                group[rb.max(ra)] = ra.min(rb);
            }
        }
    }
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..packages.len() {
        let r = root(&mut group, i);
        clusters.entry(r).or_default().push(i);
    }
    let enabled: HashSet<String> = cx
        .addons()
        .and_then(|a| a.list().ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.enabled)
        .map(|a| a.id)
        .collect();
    let standard = |p: &Package| {
        let name = p.part.as_ref().map_or(p.name.as_str(), |part| part.name.as_str());
        name.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w == "default" || w == "standard")
    };
    clusters
        .into_values()
        .filter(|c| c.len() > 1)
        .map(|c| {
            let was_on = c.iter().find(|&&i| {
                record
                    .addon(&packages[i].id)
                    .is_some_and(|m| enabled.contains(&m.id))
            });
            let on = was_on
                .or_else(|| c.iter().find(|&&i| standard(&packages[i])))
                .unwrap_or(&c[0]);
            let first = &packages[c[0]];
            Variants {
                of: first
                    .name
                    .rsplit_once(" — ")
                    .map_or(first.name.as_str(), |(_, of)| of)
                    .to_owned(),
                mods: c.iter().map(|&i| packages[i].id.clone()).collect(),
                on: packages[*on].id.clone(),
            }
        })
        .collect()
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
    // profile, and each profile's addon list in load order. With `positions` each
    // addon says its place in that list; without, the list is only in the profile
    // file, as core kept it before 1.3.0. Without `moves`, it can't reorder addons.
    struct Addons<'a> {
        root: &'a Path,
        profiles: &'a Path,
        order: &'a Mutex<BTreeMap<String, Vec<(String, bool)>>>,
        profile: String,
        calls: &'a Mutex<Vec<String>>,
        moves: bool,
        positions: bool,
    }

    impl Addons<'_> {
        fn write(&self, list: &[(String, bool)]) {
            if self.positions {
                return;
            }
            let addons: Vec<Value> = list
                .iter()
                .map(|(id, on)| json!({"id": id, "enabled": on}))
                .collect();
            std::fs::create_dir_all(self.profiles).unwrap();
            let doc = json!({"name": self.profile, "addons": addons});
            std::fs::write(
                self.profiles.join(format!("{}.json", self.profile)),
                doc.to_string(),
            )
            .unwrap();
        }
    }

    impl AddonStore for Addons<'_> {
        fn list(&self) -> Result<Vec<AddonInfo>> {
            let order = self.order.lock().unwrap();
            let on = order.get(&self.profile).cloned().unwrap_or_default();
            let mut ids: Vec<String> = std::fs::read_dir(self.root)
                .into_iter()
                .flatten()
                .flatten()
                .map(|d| d.file_name().to_string_lossy().into_owned())
                .collect();
            let listed: Vec<String> = on
                .iter()
                .map(|(a, _)| a.clone())
                .filter(|a| !ids.contains(a))
                .collect();
            ids.extend(listed);
            let mut out = Vec::new();
            for id in ids {
                out.push(AddonInfo {
                    enabled: on.iter().any(|(a, e)| *a == id && *e),
                    position: on.iter().position(|(a, _)| *a == id).filter(|_| self.positions),
                    name: id.clone(),
                    version: "1.0.0".into(),
                    author: "Team Simplified".into(),
                    dir: self.root.join(&id),
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
            self.write(list);
            Ok(())
        }

        fn move_addon(&self, id: &str, delta: isize) -> Result<()> {
            if !self.moves {
                return Err(err("this launcher can't reorder addons".into()));
            }
            self.calls.lock().unwrap().push(format!("move {id} {delta}"));
            let mut order = self.order.lock().unwrap();
            let list = order.get_mut(&self.profile).unwrap();
            let from = list.iter().position(|(a, _)| a == id).unwrap();
            let to = from.saturating_add_signed(delta).clamp(1, list.len() - 1);
            let item = list.remove(from);
            list.insert(to, item);
            self.write(list);
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
        fn convert(&self, _: &Cx, _: &Library, p: &Package, _: &str, out: &Path, _: Report) -> Result<Made> {
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
                category: Some(Category::Cars),
                kinds: vec!["archives".into()],
                summary: "1 archive resource".into(),
                lines: vec![format!("2025_asset_groups/{}.erp: 1 resource", p.id)],
                warnings: Vec::new(),
                base: p.id.contains("studio"),
            })
        }
    }

    struct Host {
        root: PathBuf,
        installed: PathBuf,
        profiles_dir: PathBuf,
        old: bool,
        positions: bool,
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
                data: root.join("addon-data").join(ID),
                installed: root.join("addons"),
                profiles_dir: root.join("profiles"),
                old: false,
                positions: true,
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
                profiles: &self.profiles_dir,
                order: &self.order,
                profile: profile.into(),
                calls: &self.calls,
                moves: !self.old,
                positions: self.positions && !self.old,
            }
        }

        fn profile(&self, name: &str, addons: &[&str]) {
            let list: Vec<(String, bool)> = addons.iter().map(|a| (a.to_string(), true)).collect();
            self.addons(name).write(&list);
            self.order.lock().unwrap().insert(name.into(), list);
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

        fn ids(&self, profile: &str) -> Vec<String> {
            self.order(profile).into_iter().map(|(id, _)| id).collect()
        }

        fn calls(&self) -> Vec<String> {
            std::mem::take(&mut *self.calls.lock().unwrap())
        }
    }

    fn with<T>(host: &Host, profile: &str, f: impl FnOnce(&Cx) -> T) -> T {
        with_game(host, profile, Some(&NoStock), f)
    }

    fn with_game<T>(
        host: &Host,
        profile: &str,
        game: Option<&dyn swofty_addon::launcher::GameFiles>,
        f: impl FnOnce(&Cx) -> T,
    ) -> T {
        let addons = host.addons(profile);
        let cx = Cx::new(
            ID,
            &host.profiles[profile],
            &host.data,
            &host.root,
            game,
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
        host.profile("one", &["swofty.core", ID, "dbedit.pit"]);
        host.profile("two", &["swofty.core", "dbedit.x", ID, "dbedit.y"]);
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
        assert_eq!(lines[0], "Converted 3 mods from the library into addons");
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
                "on imported.d",
                "move imported.c -1",
                "move imported.a -1",
                "move imported.d -1"
            ]
        );
        assert_eq!(
            host.ids("one"),
            [
                "swofty.core",
                ID,
                "imported.c",
                "imported.a",
                "imported.d",
                "dbedit.pit"
            ]
        );
        assert_eq!(host.order("one")[3], on("a", false));
        assert!(
            !host.profiles_dir.exists(),
            "the order comes from the addons' positions"
        );
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
        assert_eq!(
            host.ids("two"),
            [
                "swofty.core",
                "dbedit.x",
                ID,
                "imported.d",
                "imported.a",
                "dbedit.y"
            ]
        );
        assert_eq!(
            host.calls(),
            [
                "on imported.d",
                "on imported.a",
                "move imported.d -1",
                "move imported.a -1"
            ]
        );

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
            library.dir("b").is_dir() && !library.dir("a").is_dir(),
            "a converted mod leaves the library; one that failed stays to load through the build hook"
        );
        std::fs::remove_dir_all(&host.root).ok();
    }

    #[test]
    fn a_record_from_an_earlier_importer_reads_retired_categories_as_none() {
        for retired in Category::RETIRED {
            let made: Made = serde_json::from_value(json!({
                "id": "mod.old",
                "name": "Old",
                "category": retired,
            }))
            .unwrap();
            assert_eq!(made.category, None, "{retired}");
        }
        let made: Made = serde_json::from_value(json!({
            "id": "mod.cars",
            "name": "Cars",
            "category": "liveries",
        }))
        .unwrap();
        assert_eq!(made.category, Some(Category::Cars));
    }

    #[test]
    fn a_launcher_that_cannot_reorder_gets_the_addons_appended_in_mod_order() {
        let mut host = Host::new("append", &["one"]);
        host.old = true;
        let library = Library::new(&host.data);
        for name in ["a", "b"] {
            library.import(&host.mod_dir(name), None, &mut |_, _| {}).unwrap();
        }
        let mods = vec![slot("b", true), slot("a", true)];
        with(&host, "one", |cx| {
            cx.set_state(&State {
                mods,
                addons: Vec::new(),
            })
        })
        .unwrap();
        host.profile("one", &["swofty.core", ID, "dbedit.pit"]);
        with(&host, "one", |cx| migrate(cx, &Fake::default(), &NoProgress)).unwrap();
        assert_eq!(
            host.ids("one"),
            ["swofty.core", ID, "dbedit.pit", "imported.b", "imported.a"]
        );
        assert!(!host.calls().iter().any(|c| c.starts_with("move")));
        std::fs::remove_dir_all(&host.root).ok();
    }

    #[test]
    fn a_launcher_without_positions_is_read_from_its_profile_file() {
        let mut host = Host::new("file", &["one"]);
        host.positions = false;
        let library = Library::new(&host.data);
        for name in ["a", "b"] {
            library.import(&host.mod_dir(name), None, &mut |_, _| {}).unwrap();
        }
        let mods = vec![slot("b", true), slot("a", true)];
        with(&host, "one", |cx| {
            cx.set_state(&State {
                mods,
                addons: Vec::new(),
            })
        })
        .unwrap();
        host.profile("one", &["swofty.core", ID, "dbedit.pit"]);
        with(&host, "one", |cx| migrate(cx, &Fake::default(), &NoProgress)).unwrap();
        assert_eq!(
            host.ids("one"),
            ["swofty.core", ID, "imported.b", "imported.a", "dbedit.pit"]
        );
        assert!(host.profiles_dir.join("one.json").is_file());
        std::fs::remove_dir_all(&host.root).ok();
    }

    fn library_of(host: &Host, names: &[&str], mods: Vec<Slot>) -> Library {
        let library = Library::new(&host.data);
        for name in names {
            library.import(&host.mod_dir(name), None, &mut |_, _| {}).unwrap();
        }
        with(host, "one", |cx| {
            cx.set_state(&State {
                mods,
                addons: Vec::new(),
            })
        })
        .unwrap();
        host.profile("one", &["swofty.core", ID]);
        library
    }

    #[test]
    fn a_startup_converts_what_is_pending_once() {
        let host = Host::new("startup", &["one"]);
        library_of(
            &host,
            &["a", "b", "c"],
            vec![slot("a", true), slot("b", false), slot("c", true)],
        );
        let conv = Fake {
            refuse: vec!["c"],
            ..Fake::default()
        };
        let addon = Simplified::new();
        // The launcher sends startup to the addon's first section.
        let manifest = AddonManifest::parse(crate::MANIFEST).unwrap();
        let first = manifest.launcher.unwrap().sections[0].id.clone();
        let startup = || {
            with(&host, "one", |cx| {
                addon.event_with(cx, &first, Event::Startup {}, &conv)
            })
            .unwrap()
        };
        assert_eq!(
            startup(),
            Effect::Notice("Converted 2 mods into addons; not converted: c".into())
        );
        assert_eq!(
            host.order("one"),
            [
                ("swofty.core".to_owned(), true),
                (ID.to_owned(), true),
                on("a", true),
                on("b", false)
            ]
        );
        assert_eq!(startup(), Effect::None);
        assert_eq!(conv.made.lock().unwrap().len(), 3, "nothing converts twice");
        std::fs::remove_dir_all(&host.root).ok();
    }

    #[test]
    fn a_mod_an_older_converter_made_converts_again_once() {
        let host = Host::new("reconvert", &["one"]);
        library_of(&host, &["a"], vec![slot("a", true)]);
        let conv = Fake::default();
        let addon = Simplified::new();
        let startup = || {
            with(&host, "one", |cx| {
                addon.event_with(cx, SECTION, Event::Startup {}, &conv)
            })
            .unwrap()
        };
        startup();
        let library = Library::new(&host.data);
        assert!(library.list().unwrap().is_empty());
        library.import(&host.mod_dir("a"), None, &mut |_, _| {}).unwrap();
        let mut record = Record::load(&host.data).unwrap();
        assert!(record.mods.values().all(|e| e.converter == CONVERTER));
        for e in record.mods.values_mut() {
            e.converter = 0;
        }
        record.save(&host.data).unwrap();
        assert_eq!(startup(), Effect::Notice("Converted 1 mod into an addon".into()));
        assert_eq!(startup(), Effect::None);
        assert_eq!(conv.made.lock().unwrap().len(), 2);
        assert_eq!(host.order("one").last(), Some(&on("a", true)));
        assert!(library.list().unwrap().is_empty());
        std::fs::remove_dir_all(&host.root).ok();
    }

    #[derive(Default)]
    struct Shown(std::sync::Mutex<Vec<TaskList>>);

    impl swofty_addon::launcher::Tasks for Shown {
        fn show(&self, list: &TaskList) {
            self.0.lock().unwrap().push(list.clone());
        }
    }

    fn snapshots(shown: &Shown) -> Vec<Vec<(String, TaskState, Option<String>)>> {
        std::mem::take(&mut *shown.0.lock().unwrap())
            .into_iter()
            .map(|l| l.items.into_iter().map(|i| (i.id, i.state, i.note)).collect())
            .collect()
    }

    #[test]
    fn a_migration_shows_each_mod_as_it_converts() {
        use TaskState::{Done, Failed, Running, Waiting};
        let host = Host::new("tasks", &["one"]);
        library_of(
            &host,
            &["a", "b", "c"],
            vec![slot("a", true), slot("b", true), slot("c", true)],
        );
        let conv = Fake {
            refuse: vec!["b"],
            ..Fake::default()
        };
        let shown = Shown::default();
        let start = |budget| {
            let addons = host.addons("one");
            let cx = Cx::new(
                ID,
                &host.profiles["one"],
                &host.data,
                &host.root,
                Some(&NoStock),
                &host.states["one"],
            )
            .with_addons(&addons)
            .with_tasks(&shown);
            startup(&cx, &conv, &NoProgress, budget).unwrap()
        };
        start(Duration::ZERO);
        let item = |id: &str, state, note: Option<&str>| (id.to_owned(), state, note.map(str::to_owned));
        let next = Some("converts on next start");
        let cars = Some("Cars");
        assert_eq!(
            snapshots(&shown),
            [
                vec![
                    item("a", Waiting, None),
                    item("b", Waiting, None),
                    item("c", Waiting, None)
                ],
                vec![
                    item("a", Running, None),
                    item("b", Waiting, None),
                    item("c", Waiting, None)
                ],
                vec![
                    item("a", Done, cars),
                    item("b", Waiting, None),
                    item("c", Waiting, None)
                ],
                vec![
                    item("a", Done, cars),
                    item("b", Waiting, next),
                    item("c", Waiting, next)
                ],
            ]
        );
        start(BUDGET);
        let refused = Some("its archives could not be read");
        assert_eq!(
            snapshots(&shown),
            [
                vec![item("b", Waiting, None), item("c", Waiting, None)],
                vec![item("b", Running, None), item("c", Waiting, None)],
                vec![item("b", Failed, refused), item("c", Waiting, None)],
                vec![item("b", Failed, refused), item("c", Running, None)],
                vec![item("b", Failed, refused), item("c", Done, cars)],
            ]
        );
        start(BUDGET);
        assert!(snapshots(&shown).is_empty(), "nothing to migrate sends nothing");
        std::fs::remove_dir_all(&host.root).ok();
    }

    #[test]
    fn a_startup_leaves_what_does_not_fit_its_budget_for_the_next() {
        let host = Host::new("budget", &["one"]);
        library_of(&host, &["a", "b"], vec![slot("a", true), slot("b", true)]);
        let conv = Fake::default();
        let start = || with(&host, "one", |cx| startup(cx, &conv, &NoProgress, Duration::ZERO)).unwrap();
        assert_eq!(
            start().as_deref(),
            Some(
                "Converted 1 mod into an addon; 1 more convert at the next start, or with Convert now on the Simplified Importer"
            )
        );
        assert_eq!(start().as_deref(), Some("Converted 1 mod into an addon"));
        assert_eq!(start(), None);
        assert_eq!(host.ids("one")[2..], [on("a", true).0, on("b", true).0]);
        std::fs::remove_dir_all(&host.root).ok();
    }

    #[test]
    fn parts_that_write_the_same_things_are_variants_with_one_on() {
        let host = Host::new("variants", &["one"]);
        let src = host.root.join("src").join("Pack");
        for rel in [
            "Pack v1.0/Extra/F1 25/2025_asset_groups/extra.erp",
            "Pack v1.0/Bright Grip/Season A/F1 25/2025_asset_groups/tyres.erp",
            "Pack v1.0/Standard Grip/Season B/F1 25/2025_asset_groups/tyres.erp",
        ] {
            std::fs::create_dir_all(src.join(rel).parent().unwrap()).unwrap();
            std::fs::write(src.join(rel), "erp").unwrap();
        }
        std::fs::write(src.join(crate::serps::FILE), r#"{"mod_name":"Pack"}"#).unwrap();
        let (extra, a, b) = (
            "imported.pack.extra",
            "imported.pack.season-a-bright-grip",
            "imported.pack.season-b-standard-grip",
        );
        let addon = Simplified::new();
        let pick = || {
            with(&host, "one", |cx| {
                let pick = Event::Picked {
                    id: IMPORT.into(),
                    paths: vec![src.clone()],
                };
                addon.event_with(cx, SECTION, pick, &Fake::default())
            })
            .unwrap()
        };
        let line =
            |on: &str| format!("Pack: 2 variants installed as addons; {on} is on. Pick another on Addons.");
        let effect = pick();
        assert!(
            matches!(&effect, Effect::ShowAddon(s) if s.notice().is_some_and(|t| t.ends_with(&line("Season B · Standard Grip")))),
            "{effect:?}"
        );
        assert_eq!(
            host.calls(),
            [
                format!("install {extra}"),
                format!("on {extra}"),
                format!("install {a}"),
                format!("off {a}"),
                format!("install {b}"),
                format!("on {b}"),
            ]
        );
        let record = Record::load(&host.data).unwrap();
        let last = record.last.as_ref().unwrap();
        assert_eq!(
            last.variants,
            [Variants {
                of: "Pack".into(),
                mods: vec![
                    "pack.season-a-bright-grip".into(),
                    "pack.season-b-standard-grip".into()
                ],
                on: "pack.season-b-standard-grip".into(),
            }]
        );
        assert_eq!(
            record.addon("pack.season-a-bright-grip").unwrap().name,
            "Season A · Bright Grip — Pack"
        );

        let addons = host.addons("one");
        addons.set_enabled(a, true).unwrap();
        addons.set_enabled(b, false).unwrap();
        host.calls();
        let effect = pick();
        assert!(
            matches!(&effect, Effect::ShowAddon(s) if s.notice().is_some_and(|t| t.ends_with(&line("Season A · Bright Grip")))),
            "{effect:?}"
        );
        assert!(host.calls().contains(&format!("off {b}")), "the pick is kept");
        std::fs::remove_dir_all(&host.root).ok();
    }

    // The page's cards in reading order, through the panes' grids but not into a
    // card's own blocks.
    fn cards(view: &swofty_addon::view::View) -> Vec<String> {
        fn walk(blocks: &[Block], out: &mut Vec<String>) {
            for b in blocks {
                match b {
                    Block::Card(c) => out.extend(c.title.clone()),
                    Block::Hero(h) => out.push(h.title.clone()),
                    Block::DropZone(d) => out.push(d.title.clone()),
                    Block::Grid(g) => walk(&g.blocks, out),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        if let Body::Blocks(blocks) = &view.body {
            walk(blocks, &mut out);
        }
        out
    }

    fn texts(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Text { text, .. } | Block::Label(text) => out.push(text.clone()),
                Block::Names { label, items } => {
                    out.push(label.clone());
                    out.extend(items.iter().cloned());
                }
                Block::Buttons(buttons) => out.extend(buttons.iter().map(|b| format!("[{}]", b.label))),
                Block::Card(c) => {
                    out.extend(c.title.iter().chain(&c.subtitle).cloned());
                    out.extend(c.buttons.iter().map(|b| format!("[{}]", b.label)));
                    texts(&c.blocks, out);
                }
                Block::Grid(g) => texts(&g.blocks, out),
                _ => {}
            }
        }
    }

    fn all_text(view: &swofty_addon::view::View) -> Vec<String> {
        let mut out = Vec::new();
        if let Body::Blocks(blocks) = &view.body {
            texts(blocks, &mut out);
        }
        out
    }

    #[test]
    fn earlier_imports_are_flagged_from_their_report_and_typed_and_tagged_again() {
        let host = Host::new("relabel", &["one"]);
        let conv = Fake::default();
        let addon = Simplified::new();
        for name in ["plain", "shaded"] {
            let pick = Event::Picked {
                id: IMPORT.into(),
                paths: vec![host.mod_dir(name)],
            };
            with(&host, "one", |cx| addon.event_with(cx, SECTION, pick, &conv)).unwrap();
        }
        let dirs: BTreeMap<String, PathBuf> = host
            .addons("one")
            .list()
            .unwrap()
            .into_iter()
            .map(|a| (a.id, a.dir))
            .collect();
        for (id, report) in [
            ("imported.plain", "version 1.0.0"),
            ("imported.shaded", crate::convert::CARRIED),
        ] {
            let dir = &dirs[id];
            let manifest = json!({"schema_version": 1, "id": id, "name": id, "version": "1.0.0",
                "author": "Team Simplified", "compat": {"game": swofty_addon::GAME_ID},
                "description": match id {
                    "imported.plain" => format!("Converted from {id}.zip by the Simplified Importer: 3 car assets."),
                    _ => "Replaces the Ferrari team. 3 car assets.".to_owned(),
                },
                "category": "liveries",
                "kind": match id {
                    "imported.plain" => json!(["database", "erp", "files"]),
                    _ => json!(["erp"]),
                },
                "origin": {"tool": if id == "imported.plain" { crate::COPY_PASTE } else { crate::ID }}});
            std::fs::write(dir.join(MANIFEST_FILE), manifest.to_string()).unwrap();
            std::fs::write(
                dir.join(crate::convert::REPORT),
                format!(
                    "archives: 2 of them {report}
"
                ),
            )
            .unwrap();
        }
        let record = with(&host, "one", |cx| relabel(cx, Record::load(cx.data_dir).unwrap())).unwrap();
        let tool = |id: &str| {
            let m: AddonManifest =
                serde_json::from_slice(&std::fs::read(dirs[id].join(MANIFEST_FILE)).unwrap()).unwrap();
            m.origin.unwrap().tool
        };
        assert_eq!(tool("imported.plain"), crate::ID);
        assert_eq!(tool("imported.shaded"), crate::ID);
        let description = |id: &str| {
            let m: AddonManifest =
                serde_json::from_slice(&std::fs::read(dirs[id].join(MANIFEST_FILE)).unwrap()).unwrap();
            m.description
        };
        assert_eq!(description("imported.plain").as_deref(), Some("3 car assets."));
        let category = |id: &str| {
            let m: AddonManifest =
                serde_json::from_slice(&std::fs::read(dirs[id].join(MANIFEST_FILE)).unwrap()).unwrap();
            m.category
        };
        assert_eq!(category("imported.plain"), None);
        assert_eq!(category("imported.shaded"), Some(Category::Cars));
        let base = |id: &str| {
            record
                .mods
                .values()
                .filter_map(|e| e.made.as_ref())
                .any(|m| m.id == id && m.base)
        };
        assert!(!base("imported.plain"));
        assert!(base("imported.shaded"));
        let kinds = |id: &str| {
            let m: AddonManifest =
                serde_json::from_slice(&std::fs::read(dirs[id].join(MANIFEST_FILE)).unwrap()).unwrap();
            let made = record
                .mods
                .values()
                .filter_map(|e| e.made.as_ref())
                .find(|m| m.id == id);
            (m.kind, made.unwrap().kinds.clone())
        };
        assert_eq!(
            kinds("imported.plain"),
            (
                vec![Kind::Database, Kind::Files],
                vec!["Database".into(), "Files".into()]
            )
        );
        assert_eq!(
            kinds("imported.shaded"),
            (vec![Kind::Simplified], vec!["Simplified".into()])
        );
        assert_eq!(Record::load(&host.data).unwrap(), record);
    }

    #[test]
    fn the_base_files_load_while_a_mod_that_needs_them_is_on() {
        let host = Host::new("base", &["one"]);
        let conv = Fake::default();
        let addon = Simplified::new();
        let pick = |path: PathBuf| Event::Picked {
            id: IMPORT.into(),
            paths: vec![path],
        };
        with(&host, "one", |cx| {
            addon.event_with(cx, SECTION, pick(host.mod_dir("plain")), &conv)
        })
        .unwrap();
        assert_eq!(with(&host, "one", crate::needs_base).unwrap(), None);
        with(&host, "one", |cx| {
            addon.event_with(cx, SECTION, pick(host.mod_dir("studio")), &conv)
        })
        .unwrap();
        assert_eq!(
            with(&host, "one", crate::needs_base).unwrap().as_deref(),
            Some("studio")
        );
        host.addons("one").set_enabled("imported.studio", false).unwrap();
        assert_eq!(with(&host, "one", crate::needs_base).unwrap(), None);
        std::fs::remove_dir_all(&host.root).ok();
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
        assert!(
            Library::new(&host.data).list().unwrap().is_empty(),
            "the converted mod keeps no library copy"
        );

        let view = with(&host, "one", |cx| addon.view(cx, SECTION)).unwrap();
        assert_eq!(cards(&view), ["Import a mod"]);
        assert!(view.notices.is_empty(), "{:?}", view.notices);
        let text = all_text(&view);
        assert_eq!(view.footer, None);
        assert_eq!(text.last().map(String::as_str), Some(page::FOOTER));

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
            addon.event_with(
                cx,
                SECTION,
                click(format!("{}imported.livery", page::SHOW_ADDON)),
                &conv,
            )
        })
        .unwrap();
        assert_eq!(effect, Effect::ShowAddon("imported.livery".into()));
        std::fs::remove_dir_all(&host.root).ok();
    }

    #[test]
    fn vanilla_shows_no_import() {
        let mut host = Host::new("vanilla", &["one", "vanilla"]);
        host.profiles.get_mut("vanilla").unwrap().vanilla = true;
        let addon = Simplified::new();
        let pick = Event::Picked {
            id: IMPORT.into(),
            paths: vec![host.mod_dir("livery")],
        };
        with(&host, "one", |cx| {
            addon.event_with(cx, SECTION, pick, &Fake::default())
        })
        .unwrap();
        let view = with(&host, "vanilla", |cx| addon.view(cx, SECTION)).unwrap();
        assert!(cards(&view).is_empty());
        assert!(
            view.notices[0]
                .text
                .starts_with("The vanilla profile plays without")
        );
        std::fs::remove_dir_all(&host.root).ok();
    }

    #[test]
    fn a_mod_that_does_not_convert_is_not_kept() {
        let host = Host::new("refuse", &["one"]);
        let logos = host.mod_dir("logos");
        let conv = Fake {
            refuse: vec!["logos"],
            ..Fake::default()
        };
        let addon = Simplified::new();
        let failed = with(&host, "one", |cx| {
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
        .unwrap_err();
        assert!(failed.to_string().contains("logos was not converted"), "{failed}");
        assert!(host.calls().is_empty());
        assert!(Library::new(&host.data).list().unwrap().is_empty());
        let state: State = with(&host, "one", |cx| cx.state()).unwrap();
        assert!(state.mods.is_empty());
        assert!(with(&host, "one", |cx| addon.summary(cx)).is_none());
        std::fs::remove_dir_all(&host.root).ok();
    }

    struct NoStock;

    impl swofty_addon::launcher::GameFiles for NoStock {
        fn build_key(&self) -> String {
            "test".into()
        }

        fn exe_version(&self) -> Option<String> {
            None
        }

        fn is_stock(&self, _: &str) -> bool {
            false
        }

        fn original(&self, rel: &str) -> Result<PathBuf> {
            Err(err(format!("{rel} is not stock")))
        }

        fn original_lang(&self, code: &str) -> Result<PathBuf> {
            Err(err(format!("{code} is not stock")))
        }
    }

    #[test]
    fn the_real_conversion_installs_an_addon_the_launcher_can_read() {
        let host = Host::new("real", &["one"]);
        let dir = host.root.join("src").join("helmet");
        std::fs::create_dir_all(dir.join("F1 25/character_package/helmets")).unwrap();
        std::fs::write(dir.join("F1 25/character_package/helmets/custom.dds"), "dds").unwrap();
        std::fs::write(
            dir.join(crate::serps::FILE),
            r#"{"mod_name":"Helmet","author":"You"}"#,
        )
        .unwrap();
        let addon = Simplified::new();
        let pick = Event::Picked {
            id: IMPORT.into(),
            paths: vec![dir],
        };
        let refused = with_game(&host, "one", None, |cx| {
            addon.event_with(cx, SECTION, pick.clone(), &Converter)
        })
        .unwrap_err();
        assert!(refused.to_string().contains(NO_GAME), "{refused}");
        assert!(
            Library::new(&host.data).list().unwrap().is_empty(),
            "without a game the mod is not kept"
        );
        let effect = with(&host, "one", |cx| {
            addon.event_with(cx, SECTION, pick.clone(), &Converter)
        })
        .unwrap();
        let Effect::ShowAddon(shown) = &effect else {
            panic!("{effect:?}");
        };
        let id = shown.id().to_owned();
        assert_eq!(host.calls(), [format!("install {id}"), format!("on {id}")]);
        let installed = host.installed.join(&id).join("swofty.json");
        let manifest =
            swofty_addon::manifest::AddonManifest::parse(&std::fs::read_to_string(installed).unwrap());
        assert_eq!(manifest.unwrap().id, id);
        let effect = with(&host, "one", |cx| {
            addon.event_with(cx, SECTION, pick.clone(), &Converter)
        })
        .unwrap();
        assert_eq!(effect, Effect::ShowAddon(id.clone().into()));
        assert_eq!(host.calls(), [format!("install {id}"), format!("on {id}")]);
        let made = Record::load(&host.data)
            .unwrap()
            .addon("you.helmet")
            .cloned()
            .unwrap();
        assert!(!made.lines.is_empty(), "the report is kept: {made:?}");

        let mut record = Record::load(&host.data).unwrap();
        let old = "mod.you.helmet.from-an-older-importer";
        record
            .mods
            .get_mut("you.helmet")
            .unwrap()
            .made
            .as_mut()
            .unwrap()
            .id = old.into();
        record.save(&host.data).unwrap();
        let effect = with(&host, "one", |cx| addon.event_with(cx, SECTION, pick, &Converter)).unwrap();
        assert_eq!(effect, Effect::ShowAddon(old.into()));
        assert_eq!(
            host.calls(),
            [format!("install {old}"), format!("on {old}")],
            "a mod converted before keeps its addon's id"
        );
        std::fs::remove_dir_all(&host.root).ok();
    }

    fn record_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("swofty-importer-record-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_older_record_moves_this_machines_fields_beside_it_on_the_next_save() {
        let dir = record_dir("older");
        let older = serde_json::json!({
            "mods": {
                "one": { "name": "One", "at": 1792000000, "converter": 3,
                         "made": { "id": "mod.one", "name": "One", "kinds": ["Database"] } },
                "two": { "name": "Two", "at": 1792000500, "converter": 3, "error": "C:\\Mods\\two.zip: damaged" },
            },
            "last": { "at": 1792000600, "files": ["C:\\Mods\\one.zip"], "mods": ["one"] },
        });
        std::fs::write(dir.join(RECORD), older.to_string()).unwrap();
        let read = Record::load(&dir).unwrap();
        assert_eq!(read.mods["one"].at, 1792000000);
        assert_eq!(read.addon("one").unwrap().id, "mod.one");
        assert_eq!(
            read.mods["two"].error.as_deref(),
            Some("C:\\Mods\\two.zip: damaged")
        );
        assert_eq!(read.last.as_ref().unwrap().at, 1792000600);

        read.save(&dir).unwrap();
        let text = std::fs::read_to_string(dir.join(RECORD)).unwrap();
        for field in ["\"at\"", "\"error\"", "\"last\"", "Mods"] {
            assert!(!text.contains(field), "{field} in {text}");
        }
        assert!(dir.join(LOCAL).is_file());
        assert_eq!(Record::load(&dir).unwrap(), read);

        let mut rewritten = older.clone();
        rewritten["mods"]["one"]["at"] = serde_json::json!(1793000000);
        std::fs::write(dir.join(RECORD), rewritten.to_string()).unwrap();
        assert_eq!(Record::load(&dir).unwrap().mods["one"].at, 1793000000);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn records_that_differ_only_in_when_and_where_they_ran_save_the_same_record() {
        let record = |at: u64, path: &str| {
            let entry = |name: &str| Entry {
                name: name.into(),
                at,
                converter: CONVERTER,
                made: Some(Made {
                    id: format!("mod.{name}"),
                    name: name.into(),
                    ..Made::default()
                }),
                error: None,
            };
            let mut failed = entry("two");
            failed.made = None;
            failed.error = Some(format!("{path}: damaged"));
            Record {
                mods: BTreeMap::from([("one".into(), entry("one")), ("two".into(), failed)]),
                last: Some(Last {
                    at,
                    files: vec![path.into()],
                    mods: vec!["one".into()],
                    ..Last::default()
                }),
            }
        };
        let (a, b) = (record_dir("a"), record_dir("b"));
        record(1792000000, "C:\\Mods\\two.zip").save(&a).unwrap();
        record(1795000000, "/home/b/two.zip").save(&b).unwrap();
        let read = |dir: &Path, name: &str| std::fs::read(dir.join(name)).unwrap();
        assert_eq!(read(&a, RECORD), read(&b, RECORD));
        assert_ne!(read(&a, LOCAL), read(&b, LOCAL));
        assert_eq!(Record::load(&b).unwrap(), record(1795000000, "/home/b/two.zip"));
        for dir in [a, b] {
            std::fs::remove_dir_all(&dir).ok();
        }
    }
}
