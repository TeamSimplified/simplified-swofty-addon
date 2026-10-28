// What the game will load in the active profile, before this addon's own edits:
// the stock database and English text with every other enabled addon's operations
// played over them in load order, the way a build applies them. Rows that an
// addon only adds from its own build step, such as the drivers More Driver Models
// adds, are not in any op file; they come from the profile's last build.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;
use swofty_addon::asset::{AssetOp, FileOp, LangOp, OpFile, Root, Source};
use swofty_addon::manifest::AddonManifest;
use swofty_addon::ops::{DbOps, DbReport, ErpOp, apply_db_ops_lenient};
use swofty_core::profile::Profile;
use swofty_core::rowkeys::{Names, RowKeys};
use swofty_core::settings::render_ops;
use swofty_formats::gvdb::Gvdb;
use swofty_formats::hash::fnv1a64;
use swofty_formats::lng::Lng;

use crate::grid::db::Db;

pub const DB_REL: &str = "game_package/gamedatabases/gamedatabases.2025.bin";
pub const HEADS_DIR: &str = "2025_asset_groups/animation_package/scenes/cinematics/drivers/heads";

#[derive(Debug, Clone)]
pub struct AddonRef {
    pub id: String,
    pub name: String,
    pub dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Inputs {
    pub me: String,
    pub data_root: PathBuf,
    pub profile: String,
    pub game_dir: Option<PathBuf>,
    pub stock_db: PathBuf,
    pub stock_eng: Option<PathBuf>,
    /// The enabled addons in load order.
    pub addons: Vec<AddonRef>,
    /// Heads the launcher says the profile's addons put, with the addon's name.
    pub profile_heads: Vec<(String, Option<String>)>,
    /// Language keys this addon wrote at its last build, which the last build's
    /// text holds and which must not count as another addon's.
    pub own_lang: HashSet<String>,
}

/// More Driver Models: the drivers it adds and which are on are kept in its own files,
/// so the lists follow its page straight away rather than its last build.
pub const MORE_DRIVER_MODELS: &str = "mildtdesign.more-driver-models";
/// Its files the editor reads, in its data folder, whose changes it watches.
pub const MORE_DRIVER_MODELS_FILES: [&str; 3] = ["switches.json", "numbers.json", "edits.json"];

/// More Driver Models' drivers, by driver label: the ones it brings into the game,
/// not the game's own it only offers on the driver market.
pub struct Roster {
    pub addon: String,
    pub drivers: HashMap<String, RosterDriver>,
}

pub struct RosterDriver {
    pub name: String,
    /// Switched on on its page, by himself and by his category.
    pub on: bool,
    pub number: Option<i64>,
}

fn read_json(path: &Path) -> Value {
    std::fs::read(path)
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or(Value::Null)
}

fn roster(addon: &AddonRef, data_root: &Path) -> Option<Roster> {
    let doc = read_json(&addon.dir.join("roster.json"));
    let data = data_root.join("addon-data").join(&addon.id);
    let switches = read_json(&data.join("switches.json"));
    let numbers = read_json(&data.join("numbers.json"));
    let edits = read_json(&data.join("edits.json"));
    let listed = |k: &str, v: &str| {
        switches[k]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(v)))
    };
    let number = |v: &Value| v.as_i64().or_else(|| v.as_str()?.trim().parse().ok());
    let mut drivers = HashMap::new();
    for c in doc["categories"].as_array()? {
        let category = c["name"].as_str().unwrap_or_default();
        for d in c["drivers"].as_array().into_iter().flatten() {
            let Some(label) = d["file_name"].as_str() else {
                continue;
            };
            if d["in_game"].as_bool() == Some(true) {
                continue;
            }
            drivers.insert(
                label.to_owned(),
                RosterDriver {
                    name: d["name"].as_str().unwrap_or(label).to_owned(),
                    on: !listed("off", label) && !listed("off_categories", category),
                    number: number(&edits[label]["number"]).or_else(|| number(&numbers[label])),
                },
            );
        }
    }
    Some(Roster {
        addon: addon.name.clone(),
        drivers,
    })
}

/// One ERP op of another addon, with what it needs to read its source.
#[derive(Debug, Clone)]
pub struct ErpChange {
    pub addon: String,
    pub op: ErpOp,
    pub source: Option<PathBuf>,
}

pub struct World {
    pub db: Db,
    pub lang: HashMap<String, String>,
    pub stock_lists: HashSet<String>,
    /// The driver looks presets the game has before any addon, by label.
    pub stock_looks: HashSet<String>,
    /// Entrant lists an addon added, and that addon's name.
    pub list_origin: HashMap<String, String>,
    /// The addons that change each name, the code and each colour of a team, by
    /// team label and then by part: `full`, `short`, `super_short`, `code` or the
    /// colour field.
    pub touched: Changes,
    /// The addons that change a seat's driver or number, by the seat's row path and
    /// then `driver` or `number`.
    pub seats: Changes,
    pub erp: Vec<ErpChange>,
    pub heads: HashSet<String>,
    /// The addon that adds each driver row, by driver label.
    pub driver_origin: HashMap<String, String>,
    /// The addon that puts each head model, by head label.
    pub head_origin: HashMap<String, String>,
    /// The drivers the game has before any addon, by label.
    pub stock_drivers: HashSet<String>,
    /// The drivers an addon's build step adds or renames, such as More Driver
    /// Models' drivers: its build points the game's rows at its own names.
    pub build_drivers: HashSet<String>,
    /// More Driver Models' drivers, when the profile has it on.
    pub roster: Option<Roster>,
    pub built: Option<PathBuf>,
    pub notes: Vec<String>,
}

impl World {
    pub fn text(&self, key: &str) -> Option<String> {
        let t = self.lang.get(key)?.trim();
        (!t.is_empty()).then(|| clean(t))
    }

    /// Every addon that changes something of a team.
    pub fn writers(&self, team: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for list in self.touched.get(team).into_iter().flat_map(|m| m.values()) {
            for a in list {
                if !out.contains(a) {
                    out.push(a.clone());
                }
            }
        }
        out
    }

    /// The addons that change one part of a team or a seat.
    pub fn changed_by<'a>(changes: &'a Changes, key: &str, part: &str) -> &'a [String] {
        changes
            .get(key)
            .and_then(|m| m.get(part))
            .map_or(&[], Vec::as_slice)
    }
}

pub type Changes = HashMap<String, BTreeMap<String, Vec<String>>>;

/// What each team field is called as a part.
pub const TEAM_PARTS: [&str; 8] = [
    "full",
    "short",
    "super_short",
    "code",
    "ui_team_colour",
    "primary_colour",
    "secondary_colour",
    "tertiary_colour",
];

const TEAM_FIELDS: [&str; 8] = [
    "display_name",
    "display_name_short",
    "display_name_super_short",
    "display_name_code",
    "ui_team_colour",
    "primary_colour",
    "secondary_colour",
    "tertiary_colour",
];

fn team_values(db: &Db) -> HashMap<String, Vec<String>> {
    db.rows("entrants.teams", "team")
        .iter()
        .map(|r| {
            (
                db.field(r, "_metadata_"),
                TEAM_FIELDS.iter().map(|f| db.field(r, f)).collect(),
            )
        })
        .collect()
}

fn touch(changes: &mut Changes, key: &str, part: &str, addon: &str) {
    let list = changes
        .entry(key.to_owned())
        .or_default()
        .entry(part.to_owned())
        .or_default();
    if !list.iter().any(|a| a == addon) {
        list.push(addon.to_owned());
    }
}

const SEAT_FIELDS: [&str; 3] = ["entrant_reference", "driver", "car_number"];

/// Every seat of every grid: its row path and the fields that choose its driver
/// and number.
fn seat_values(db: &Db) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    for list in db.rows("entrants.entrant_lists", "entrant_list") {
        for r in db.rows(&list, "entrant") {
            let v = SEAT_FIELDS.iter().map(|f| db.field(&r, f)).collect();
            out.insert(r, v);
        }
    }
    out
}

/// A name without the game's inline markup.
pub fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut angle, mut brace) = (false, false);
    for c in text.chars() {
        match c {
            '<' => angle = true,
            '>' if angle => angle = false,
            '{' => brace = true,
            '}' if brace => brace = false,
            _ if !angle && !brace => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn latest_build(data_root: &Path, profile: &str) -> Option<PathBuf> {
    let dir = data_root.join("cache/overlay").join(profile);
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let plan = e.path().join("plan.json");
            Some((plan.metadata().ok()?.modified().ok()?, e.path()))
        })
        .max()
        .map(|(_, p)| p)
}

fn read_lng(path: &Path) -> Option<Lng> {
    Lng::parse(&std::fs::read(path).ok()?).ok()
}

fn resolve(source: &Source, addon: &AddonRef, data_root: &Path) -> Option<PathBuf> {
    match source.root {
        Root::Addon => source.resolve(&addon.dir).ok(),
        Root::Data => Some(data_root.join("addon-data").join(&addon.id).join(&source.path)),
        Root::Cache => Some(data_root.join("cache/addons").join(&addon.id).join(&source.path)),
    }
}

fn english(language: &str) -> bool {
    language == "*" || language == "eng"
}

struct Loaded {
    db: DbOps,
    lang: Vec<LangOp>,
    erp: Vec<ErpOp>,
    files: Vec<String>,
}

fn load_ops(addon: &AddonRef, profile: &Profile, notes: &mut Vec<String>) -> Option<Loaded> {
    let raw = std::fs::read_to_string(addon.dir.join("swofty.json")).ok()?;
    let manifest = match AddonManifest::parse(&raw) {
        Ok(m) => m,
        Err(e) => {
            notes.push(format!("{}: swofty.json doesn't read ({e})", addon.name));
            return None;
        }
    };
    let mut out = Loaded {
        db: DbOps {
            schema_version: 1,
            fields: BTreeMap::new(),
            strict: false,
            side_data: Default::default(),
            ops: Vec::new(),
        },
        lang: Vec::new(),
        erp: Vec::new(),
        files: Vec::new(),
    };
    for (kind, rel) in manifest.operations.op_files() {
        let Some(mut doc) = std::fs::read(addon.dir.join(rel))
            .ok()
            .and_then(|d| serde_json::from_slice::<Value>(&d).ok())
        else {
            continue;
        };
        if render_ops(&mut doc, &manifest, profile).is_err() {
            notes.push(format!(
                "{}: {rel} doesn't render with this profile's settings",
                addon.name
            ));
            continue;
        }
        let file = match OpFile::from_value(doc, kind) {
            Ok(f) => f,
            Err(e) => {
                notes.push(format!("{}: {rel} doesn't read ({e})", addon.name));
                continue;
            }
        };
        for (k, v) in file.fields {
            out.db.fields.entry(k).or_default().extend(v);
        }
        for op in file.ops {
            match op {
                AssetOp::Db(op) => out.db.ops.push(op),
                AssetOp::Lang(op) => out.lang.push(op),
                AssetOp::Erp(op) => out.erp.push(op),
                AssetOp::File(FileOp::Put { target, .. }) => out.files.push(target),
                _ => {}
            }
        }
    }
    for f in manifest.operations.files {
        out.files.push(f.target);
    }
    Some(out)
}

const GROWN_TABLES: [(&str, &str, &str); 4] = [
    ("entrants.drivers", "driver", "entrants.drivers.driver_N"),
    (
        "entrants.entrant_lists.entrants",
        "entrant",
        "entrants.entrant_lists.entrants.entrant_N",
    ),
    (
        "entrants.animation.driver_heads",
        "driver_head",
        "entrants.animation.driver_heads.driver_head_N",
    ),
    ("entrants.teams", "team", "entrants.teams.team_N"),
];

type BuildEdits = HashMap<u64, (u8, Vec<u8>)>;

/// The database writes of the last build, by record key.
fn build_edits(built: &Path) -> BuildEdits {
    let Some(doc) = std::fs::read(built.join("swofty/assets.json"))
        .ok()
        .and_then(|d| serde_json::from_slice::<Value>(&d).ok())
    else {
        return HashMap::new();
    };
    let mut edits: BuildEdits = HashMap::new();
    for e in doc["db_edits"].as_array().into_iter().flatten() {
        let (Some(hash), Some(value)) = (e["hash"].as_str(), e["value"].as_str()) else {
            continue;
        };
        let Ok(key) = u64::from_str_radix(hash, 16) else {
            continue;
        };
        let tag = e["type"].as_u64().unwrap_or(2) as u8;
        edits.insert(key, (tag, value.as_bytes().to_vec()));
    }
    edits
}

/// The fields of a driver row an addon's build step points at names and a head of
/// its own.
const BUILD_DRIVER_FIELDS: [&str; 5] = [
    "first_name",
    "last_name",
    "last_name_standard",
    "full_name",
    "head",
];

/// Driver rows the last build gave other names or a head: an addon such as More
/// Driver Models does that from its build step, for the drivers it has on. The
/// editor's own names, under its lng_sge_ keys, are left out. Returns the labels.
fn drivers_from_build(db: &mut Db, edits: &BuildEdits) -> HashSet<String> {
    let mut out = HashSet::new();
    for r in db.rows("entrants.drivers", "driver") {
        for f in BUILD_DRIVER_FIELDS {
            let key = fnv1a64(format!("{r}.{f}"));
            let Some((tag, value)) = edits.get(&key) else {
                continue;
            };
            let text = String::from_utf8_lossy(value);
            if text.starts_with("lng_sge_") || db.field(&r, f) == text {
                continue;
            }
            db.put_key(key, *tag, value.clone());
            out.insert(db.field(&r, "_metadata_"));
        }
    }
    out
}

// Rows past the live row count that the last build has: the rows addons add from
// their build step. Their keys come from the launcher's key catalogue.
fn grow_from_build(db: &mut Db, edits: &BuildEdits, notes: &mut Vec<String>) -> usize {
    let keys = RowKeys::builtin();
    let mut added = 0;
    for (container, row, shape) in GROWN_TABLES {
        let count_path = format!("{container}._{row}_instancecount_");
        let live = db.count(container, row);
        let Some(built_count) = edits
            .get(&fnv1a64(&count_path))
            .and_then(|(_, v)| String::from_utf8_lossy(v).trim().parse::<usize>().ok())
        else {
            continue;
        };
        if built_count <= live {
            continue;
        }
        let Some(fields) = keys.get(shape) else {
            notes.push(format!("the key catalogue has no {shape}"));
            continue;
        };
        for i in live..built_count {
            for f in fields {
                let path = format!("{container}.{row}_{i}.{f}");
                if let Some((tag, value)) = edits.get(&fnv1a64(&path)) {
                    db.put_key(fnv1a64(&path), *tag, value.clone());
                }
            }
            added += 1;
        }
        let tag = db.gvdb.get(&count_path).map_or(0, |r| r.tag);
        db.put_key(fnv1a64(&count_path), tag, built_count.to_string().into_bytes());
    }
    added
}

pub fn heads_in_game(game_dir: &Path) -> HashSet<String> {
    std::fs::read_dir(game_dir.join(HEADS_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".erp").map(str::to_owned)
        })
        .collect()
}

pub fn load(inputs: &Inputs) -> Result<World, String> {
    let bytes = std::fs::read(&inputs.stock_db).map_err(|e| format!("{}: {e}", inputs.stock_db.display()))?;
    let gvdb = Gvdb::parse(&bytes).map_err(|e| format!("the game database is unreadable: {e}"))?;
    let mut db = Db::new(gvdb);
    let mut notes = Vec::new();
    let stock_lists: HashSet<String> = db
        .rows("entrants.entrant_lists", "entrant_list")
        .iter()
        .map(|r| db.field(r, "_metadata_"))
        .collect();
    let stock_looks: HashSet<String> = db
        .rows("entrants.driver_customisations", "folder")
        .iter()
        .flat_map(|f| db.rows(f, "preloaded_driver_customisation_properties"))
        .map(|r| db.field(&r, "_metadata_"))
        .collect();

    let mut lang: HashMap<String, String> = HashMap::new();
    if let Some(eng) = inputs.stock_eng.as_deref().and_then(read_lng) {
        lang.extend(eng.entries.into_iter().map(|e| (e.key, e.value)));
    }
    let stock_keys: HashSet<String> = lang.keys().cloned().collect();

    let built = latest_build(&inputs.data_root, &inputs.profile);
    if let Some(dir) = &built {
        for name in ["all.lng", "eng.lng"] {
            let Some(table) = read_lng(&dir.join("swofty/assets/lang").join(name)) else {
                continue;
            };
            for e in table.entries {
                if !stock_keys.contains(&e.key) && !inputs.own_lang.contains(&e.key) {
                    lang.insert(e.key, e.value);
                }
            }
        }
    }

    let profile: Profile = std::fs::read(
        inputs
            .data_root
            .join("profiles")
            .join(format!("{}.json", inputs.profile)),
    )
    .ok()
    .and_then(|d| serde_json::from_slice(&d).ok())
    .unwrap_or_else(|| {
        serde_json::from_value(serde_json::json!({ "schema_version": 2, "name": inputs.profile }))
            .expect("a bare profile")
    });

    let empty = BTreeMap::new();
    let names = Names {
        fields: &empty,
        keys: RowKeys::builtin(),
        stock: None,
        tracks: None,
    };
    // The rows the launcher's core adds before any addon, such as the swofty_f1
    // modes Classic Seasons appends its grids to.
    let core = swofty_core::builtin::core_db_ops();
    let core = DbOps {
        schema_version: core.schema_version,
        fields: core.fields,
        strict: false,
        side_data: Default::default(),
        ops: core
            .ops
            .into_iter()
            .filter_map(|op| match op {
                AssetOp::Db(op) => Some(op),
                _ => None,
            })
            .collect(),
    };
    apply_db_ops_lenient(&mut db.gvdb, &core, &names, &mut DbReport::default());
    db.reindex();
    let mut heads = inputs.game_dir.as_deref().map(heads_in_game).unwrap_or_default();
    let mut head_origin: HashMap<String, String> = HashMap::new();
    for (head, addon) in &inputs.profile_heads {
        heads.insert(head.clone());
        if let Some(a) = addon {
            head_origin.insert(head.clone(), a.clone());
        }
    }
    let driver_labels = |db: &Db| -> HashSet<String> {
        db.rows("entrants.drivers", "driver")
            .iter()
            .map(|r| db.field(r, "_metadata_"))
            .collect()
    };
    let stock_drivers = driver_labels(&db);
    let mut drivers_before = stock_drivers.clone();
    let mut driver_origin: HashMap<String, String> = HashMap::new();
    let mut list_origin = HashMap::new();
    let mut touched: Changes = HashMap::new();
    let mut seats: Changes = HashMap::new();
    let stock_teams = team_values(&db);
    let mut teams_before = stock_teams.clone();
    let mut seats_before = seat_values(&db);
    let mut repointed: Changes = HashMap::new();
    let mut lang_writes: Vec<(String, String, String)> = Vec::new();
    let mut erp = Vec::new();
    let mut defaults: Vec<(String, String)> = Vec::new();
    let mut sets: Vec<(String, String)> = Vec::new();
    for addon in &inputs.addons {
        if addon.id == inputs.me {
            continue;
        }
        let Some(ops) = load_ops(addon, &profile, &mut notes) else {
            continue;
        };
        let before: HashSet<String> = db
            .rows("entrants.entrant_lists", "entrant_list")
            .iter()
            .map(|r| db.field(r, "_metadata_"))
            .collect();
        if !ops.db.ops.is_empty() {
            let mut report = DbReport::default();
            let (_, failed) = apply_db_ops_lenient(&mut db.gvdb, &ops.db, &names, &mut report);
            if !failed.is_empty() {
                notes.push(format!(
                    "{}: {} database op(s) don't apply here, such as: {}",
                    addon.name,
                    failed.len(),
                    failed[0].1
                ));
            }
            db.reindex();
            // Only rows that were there before count: the teams and seats an addon
            // adds, such as a Classic Seasons year, are its own, not changes.
            let now = team_values(&db);
            for (team, values) in &now {
                let Some(was) = teams_before.get(team) else {
                    continue;
                };
                for (i, part) in TEAM_PARTS.iter().enumerate() {
                    if was[i] == values[i] {
                        continue;
                    }
                    // A name field holds a text key; whether pointing it elsewhere
                    // shows a new name is known once the text is in.
                    let into = if i < 3 { &mut repointed } else { &mut touched };
                    touch(into, team, part, &addon.name);
                }
            }
            teams_before = now;
            let now = seat_values(&db);
            for (row, values) in &now {
                let Some(was) = seats_before.get(row) else {
                    continue;
                };
                if was[..2] != values[..2] {
                    touch(&mut seats, row, "driver", &addon.name);
                }
                if was[2] != values[2] {
                    touch(&mut seats, row, "number", &addon.name);
                }
            }
            seats_before = now;
            let now = driver_labels(&db);
            for d in now.difference(&drivers_before) {
                driver_origin.insert(d.clone(), addon.name.clone());
            }
            drivers_before = now;
        }
        for r in db.rows("entrants.entrant_lists", "entrant_list") {
            let label = db.field(&r, "_metadata_");
            if !before.contains(&label) {
                list_origin.insert(label, addon.name.clone());
            }
        }
        for op in ops.lang {
            match op {
                LangOp::Set { language, key, value } if english(&language) => {
                    lang_writes.push((addon.name.clone(), key.clone(), value.clone()));
                    sets.push((key, value));
                }
                LangOp::Default { language, key, value } if english(&language) => {
                    defaults.push((key, value));
                }
                LangOp::Table { language, source } if english(&language) => {
                    if let Some(table) = resolve(&source, addon, &inputs.data_root)
                        .as_deref()
                        .and_then(read_lng)
                    {
                        for e in table.entries {
                            lang_writes.push((addon.name.clone(), e.key.clone(), e.value.clone()));
                            sets.push((e.key, e.value));
                        }
                    }
                }
                _ => {}
            }
        }
        for target in &ops.files {
            let t = target.replace('\\', "/").to_lowercase();
            if let Some(name) = t
                .strip_prefix(&format!("{HEADS_DIR}/"))
                .and_then(|n| n.strip_suffix(".erp"))
            {
                heads.insert(name.to_owned());
                head_origin.insert(name.to_owned(), addon.name.clone());
            }
        }
        for op in ops.erp {
            let source = match &op {
                ErpOp::PutResource { source, .. }
                | ErpOp::MergeSlices { source, .. }
                | ErpOp::Overlay { source, .. } => resolve(source, addon, &inputs.data_root),
                _ => None,
            };
            erp.push(ErpChange {
                addon: addon.name.clone(),
                op,
                source,
            });
        }
    }
    let stock_text = lang.clone();
    for (k, v) in defaults {
        lang.insert(k, v);
    }
    for (k, v) in sets {
        lang.insert(k, v);
    }
    // A team's names count as changed by an addon when they read differently from
    // the game's: the addons that pointed its name fields elsewhere or wrote the
    // text those fields now read.
    let shown =
        |text: &HashMap<String, String>, key: &str| text.get(key).map(|t| clean(t)).unwrap_or_default();
    for (team, values) in team_values(&db) {
        let Some(stock) = stock_teams.get(&team) else {
            continue;
        };
        for (i, part) in TEAM_PARTS[..3].iter().enumerate() {
            if shown(&lang, &values[i]) == shown(&stock_text, &stock[i]) {
                continue;
            }
            // Credit the addon whose text shows: the last to write the key the field
            // reads. Only when the new name is text the game already had is it the
            // last addon to point the field at that key.
            let writer = lang_writes
                .iter()
                .rev()
                .find(|(_, key, value)| values[i] == *key && clean(value) != shown(&stock_text, &stock[i]))
                .map(|(addon, _, _)| addon.clone());
            if let Some(addon) = writer.or_else(|| World::changed_by(&repointed, &team, part).last().cloned())
            {
                touch(&mut touched, &team, part, &addon);
            }
        }
    }
    let mut build_drivers = HashSet::new();
    if let Some(dir) = &built {
        let edits = build_edits(dir);
        let before = driver_labels(&db);
        let n = grow_from_build(&mut db, &edits, &mut notes);
        if n > 0 {
            db.reindex();
        }
        build_drivers = driver_labels(&db).difference(&before).cloned().collect();
        build_drivers.extend(drivers_from_build(&mut db, &edits));
    }
    Ok(World {
        db,
        lang,
        stock_lists,
        stock_looks,
        list_origin,
        touched,
        seats,
        erp,
        heads,
        driver_origin,
        head_origin,
        stock_drivers,
        build_drivers,
        roster: inputs
            .addons
            .iter()
            .find(|a| a.id == MORE_DRIVER_MODELS)
            .and_then(|a| roster(a, &inputs.data_root)),
        built,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_comes_out_of_names() {
        assert_eq!(clean("{o:caps}McLaren{/o}  F1 <b>Team</b>"), "McLaren F1 Team");
    }
}
