pub mod diff;
pub mod library;
pub mod merge;
pub mod package;
pub mod serps;
pub mod texarray;
pub mod track;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use swofty_addon::asset::{AssetBatch, AssetOp, LangOp, Op, Origin};
use swofty_addon::launcher::{
    Addon, ArgSpec, BuildCx, Builtin, Claim, CommandSpec, Cx, Importer, Progress, Summary,
};
use swofty_addon::manifest::AddonManifest;
use swofty_addon::ops::{DbOp, ErpOp};
use swofty_addon::view::{
    Art, Block, Body, Button, ButtonKind, Control, Effect, Event, Fact, Icon, Notice, Row, SectionSpec,
    Style, Tag, Tone, View,
};
use swofty_addon::{Error, Result};

use crate::diff::Diff;
use crate::library::{Library, Package};
use crate::merge::Clash;
use crate::package::Class;

pub const ID: &str = "teamsimplified.mods";
pub const SECTION: &str = "mods";
const MANIFEST: &str = include_str!("../swofty.json");
const FOOTER: &str = "Works with any F1 25 mod: its files go where copying them into the game folder would put them. Made by Team Simplified. Based on Simplified Suite by Team Simplified.";
const IMPORT: &str = "import";
const ENABLE: &str = "enable";
const COMPARE: &str = "compare";
const SHOW: &str = "show:";
const REMOVE: &str = "remove:";

// Mods built from the same tools write many records and strings with the same
// values. A write that repeats the value already in place is left out, so it is
// not reported as one mod overriding another. A removal of something an earlier
// mod already removed is left out too, since the second one would find nothing.
fn without_repeats(ops: Vec<Op>) -> Vec<Op> {
    let mut last: HashMap<(char, String, String), Option<(u8, String)>> = HashMap::new();
    ops.into_iter()
        .filter(|op| {
            let (target, value) = match &op.op {
                AssetOp::Db(DbOp::PutRecord { key, tag, value }) => {
                    (('d', key.clone(), String::new()), Some((*tag, value.clone())))
                }
                AssetOp::Db(DbOp::RemoveRecord { key }) => (('d', key.clone(), String::new()), None),
                AssetOp::Lang(LangOp::Set { language, key, value }) => {
                    (('l', language.clone(), key.clone()), Some((0, value.clone())))
                }
                AssetOp::Erp(ErpOp::RemoveResource { erp, resource }) => {
                    (('e', erp.clone(), resource.clone()), None)
                }
                AssetOp::Erp(ErpOp::PutResource {
                    erp,
                    resource,
                    rename,
                    ..
                }) => {
                    let name = rename.as_deref().unwrap_or(resource);
                    last.remove(&('e', erp.clone(), name.to_owned()));
                    return true;
                }
                _ => return true,
            };
            last.insert(target, value.clone()) != Some(value)
        })
        .collect()
}

// A default goes in once for each key no mod renames: the mods that fill it in
// all give the game's own text, and a rename replaces it anyway.
fn only_defaults(defaults: Vec<Op>, ops: &[Op]) -> Vec<Op> {
    let key = |op: &Op| match &op.op {
        AssetOp::Lang(LangOp::Set { language, key, .. }) => Some((language.clone(), key.clone())),
        _ => None,
    };
    let mut seen: HashSet<(String, String)> = ops.iter().filter_map(key).collect();
    defaults
        .into_iter()
        .filter(|op| key(op).is_some_and(|k| seen.insert(k)))
        .collect()
}

pub fn builtin() -> Builtin {
    Builtin {
        addon: Arc::new(Simplified::new()),
        locked: false,
        default_on: true,
    }
}

pub(crate) fn err(text: String) -> Error {
    Error::Other(text)
}

pub(crate) fn io_err(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub mods: Vec<Slot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub id: String,
    pub enabled: bool,
}

pub struct Simplified {
    manifest: AddonManifest,
}

impl Default for Simplified {
    fn default() -> Self {
        Self::new()
    }
}

impl Simplified {
    pub fn new() -> Self {
        Self {
            manifest: AddonManifest::parse(MANIFEST).expect("the built-in manifest is valid"),
        }
    }
}

struct Loaded {
    order: Vec<(Package, bool)>,
}

impl Loaded {
    fn read(cx: &Cx) -> Result<Self> {
        let mut library = Library::new(cx.data_dir).list()?;
        let state: State = cx.state()?;
        let mut order = Vec::new();
        for slot in state.mods {
            if let Some(i) = library.iter().position(|p| p.id == slot.id) {
                order.push((library.remove(i), slot.enabled));
            }
        }
        order.extend(library.into_iter().map(|p| (p, false)));
        Ok(Self { order })
    }

    fn save(&self, cx: &Cx) -> Result<()> {
        let mods = self
            .order
            .iter()
            .map(|(p, on)| Slot {
                id: p.id.clone(),
                enabled: *on,
            })
            .collect();
        cx.set_state(&State { mods })
    }

    fn find(&self, id: &str) -> Result<usize> {
        self.order
            .iter()
            .position(|(p, _)| p.id == id)
            .ok_or_else(|| err(format!("no mod {id} in the library")))
    }

    fn set(&mut self, id: &str, on: bool) -> Result<&Package> {
        let i = self.find(id)?;
        self.order[i].1 = on;
        Ok(&self.order[i].0)
    }

    fn move_to(&mut self, id: &str, to: usize) -> Result<&Package> {
        let entry = self.order.remove(self.find(id)?);
        let to = to.min(self.order.len());
        self.order.insert(to, entry);
        Ok(&self.order[to].0)
    }

    fn enabled(&self) -> impl Iterator<Item = &Package> {
        self.order.iter().filter(|(_, on)| *on).map(|(p, _)| p)
    }
}

fn import(cx: &Cx, paths: &[PathBuf], progress: &dyn Progress) -> Result<Vec<String>> {
    let library = Library::new(cx.data_dir);
    let known: Vec<String> = library.list()?.into_iter().map(|p| p.id).collect();
    let mut done = Vec::new();
    let mut added = Vec::new();
    let mut failed = Vec::new();
    let total = paths.len().max(1) as f32;
    for (i, path) in paths.iter().enumerate() {
        let mut report =
            |f: f32, text: &str| progress.report((i as f32 + f) / total, &format!("Importing {text}"));
        match library.import(path, cx.game, &mut report) {
            Ok(imported) => {
                let p = &imported.package;
                let mut line = format!("Imported {}", p.name);
                if imported.refused > 0 {
                    line.push_str(&format!(", {} unsafe path(s) left out", imported.refused));
                }
                if !p.skipped.is_empty() {
                    line.push_str(&format!(", skipped {}", p.skipped.join(", ")));
                }
                if let Some(Err(e)) = cx.game.map(|g| library.diff(p, g)) {
                    line.push_str(&format!(", not compared with the game: {e}"));
                }
                done.push(line);
                if !known.contains(&p.id) {
                    added.push(p.id.clone());
                }
            }
            Err(e) => failed.push(format!("{}: {e}", package::file_name(path))),
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

fn import_button(primary: bool) -> Button {
    Button {
        icon: Some(Icon::Upload),
        style: if primary { Style::Primary } else { Style::Outline },
        kind: ButtonKind::Pick {
            title: "Import mods".into(),
            extensions: vec!["zip".into(), "7z".into(), "rar".into(), "serps".into()],
            folders: true,
            multiple: true,
        },
        ..Button::new(IMPORT, "Import…")
    }
}

fn subtitle(p: &Package) -> String {
    [p.author.as_deref(), p.version.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

fn monogram(name: &str) -> String {
    let letters: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .collect();
    letters.to_uppercase()
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.0} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / (1u64 << 10) as f64),
        b => format!("{b} B"),
    }
}

fn date(secs: u64) -> String {
    let days = (secs / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year}-{month:02}-{day:02}")
}

fn tags(p: &Package, exe: Option<&str>) -> Vec<Tag> {
    let tag = |text: String, tone: Tone| Tag { text, tone };
    let mut out = Vec::new();
    if p.track.is_some() {
        out.push(tag("Track".into(), Tone::Accent));
        return out;
    }
    if p.count(Class::Database) > 0 {
        out.push(tag("Database".into(), Tone::Muted));
    }
    if p.count(Class::Text) > 0 {
        out.push(tag("Text".into(), Tone::Muted));
    }
    match p.count(Class::Archive) {
        0 => {}
        1 => out.push(tag("1 archive".into(), Tone::Muted)),
        n => out.push(tag(format!("{n} archives"), Tone::Muted)),
    }
    match p.count(Class::Replacement) + p.count(Class::Addition) {
        0 => {}
        1 => out.push(tag("1 file".into(), Tone::Muted)),
        n => out.push(tag(format!("{n} files"), Tone::Muted)),
    }
    if let (Some(made), Some(exe)) = (p.game_version.as_deref(), exe)
        && made != exe
    {
        out.push(tag(format!("Made for {made}"), Tone::Warn));
    }
    out
}

fn changes(p: &Package, d: &Diff, exe: Option<&str>) -> Vec<Block> {
    let mut out = vec![Block::Label("WHAT IT CHANGES".into())];
    if d.is_empty() {
        out.push(Block::Text {
            text: "Nothing differs from the game.".into(),
            tone: Tone::Muted,
        });
    }
    if !d.records.is_empty() {
        out.push(Block::Facts(vec![
            Fact::new("RECORDS CHANGED", d.records.changed.len().to_string()),
            Fact::new("RECORDS ADDED", d.records.added.len().to_string()),
            Fact::new("RECORDS REMOVED", d.records.removed.len().to_string()),
        ]));
    }
    if !d.text.is_empty() {
        out.push(Block::Table {
            columns: vec!["Language".into(), "Changed".into(), "Added".into()],
            rows: d
                .text
                .iter()
                .map(|t| {
                    let language = match &t.from {
                        Some(from) => format!("{} (from {from})", t.language),
                        None => t.language.clone(),
                    };
                    vec![language, t.changed.len().to_string(), t.added.len().to_string()]
                })
                .collect(),
        });
    }
    if !d.archives.is_empty() {
        out.push(Block::Table {
            columns: vec![
                "Archive".into(),
                "Changed".into(),
                "Added".into(),
                "Removed".into(),
            ],
            rows: d
                .archives
                .iter()
                .map(|a| {
                    vec![
                        a.path.clone(),
                        a.changed.len().to_string(),
                        a.added.len().to_string(),
                        a.removed.len().to_string(),
                    ]
                })
                .collect(),
        });
    }
    for (label, class) in [
        ("REPLACED FILES", Class::Replacement),
        ("ADDED FILES", Class::Addition),
    ] {
        let items: Vec<String> = d
            .files
            .iter()
            .filter(|f| f.class == class)
            .map(|f| f.path.clone())
            .collect();
        if !items.is_empty() {
            out.push(Block::Names {
                label: label.into(),
                items,
            });
        }
    }
    if d.dropped > 0 {
        let made = p.game_version.as_deref().unwrap_or("an unknown version");
        let game = exe.unwrap_or("unknown");
        out.push(Block::Text {
            text: format!(
                "{} removal(s) left out: the package was made for {made} and the game is {game}.",
                d.dropped
            ),
            tone: Tone::Warn,
        });
    }
    out
}

fn files(p: &Package) -> Vec<Block> {
    let groups = [
        ("DATABASE", Class::Database),
        ("LANGUAGES", Class::Text),
        ("ARCHIVES", Class::Archive),
        ("REPLACED FILES", Class::Replacement),
        ("ADDED FILES", Class::Addition),
    ];
    let mut out = Vec::new();
    for (label, class) in groups {
        let items: Vec<String> = p
            .files
            .iter()
            .filter(|f| f.class == class)
            .map(|f| match class {
                Class::Text => diff::language(&f.path).unwrap_or(&f.path).to_owned(),
                _ => f.path.clone(),
            })
            .collect();
        if !items.is_empty() {
            out.push(Block::Names {
                label: label.into(),
                items,
            });
        }
    }
    out
}

struct Shown<'a> {
    on: bool,
    switchable: bool,
    exe: Option<&'a str>,
    diff: Option<&'a Diff>,
    wins: Vec<(&'a str, &'a Clash)>,
    losses: Vec<(&'a str, &'a Clash)>,
}

fn conflicts(shown: &Shown) -> (Vec<Tag>, Vec<Block>) {
    let mut tags = Vec::new();
    let mut blocks = Vec::new();
    for (name, c) in &shown.wins {
        tags.push(Tag {
            text: format!("Overrides {} in {name}", c.targets.len()),
            tone: Tone::Accent,
        });
        blocks.push(Block::Names {
            label: format!("OVERRIDES {}", name.to_uppercase()),
            items: c.targets.clone(),
        });
    }
    for (name, c) in &shown.losses {
        tags.push(Tag {
            text: format!("{} overridden by {name}", c.targets.len()),
            tone: Tone::Warn,
        });
        blocks.push(Block::Names {
            label: format!("OVERRIDDEN BY {}", name.to_uppercase()),
            items: c.targets.clone(),
        });
    }
    if !blocks.is_empty() {
        blocks.insert(0, Block::Label("CONFLICTS".into()));
    }
    (tags, blocks)
}

fn row(p: &Package, shown: Shown) -> Row {
    let (on, switchable, exe, diff) = (shown.on, shown.switchable, shown.exe, shown.diff);
    let (clash_tags, clash_blocks) = conflicts(&shown);
    let dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "—".into());
    let mut detail = vec![Block::Stats(vec![
        Fact::new("STATUS", if on { "On" } else { "Off" }),
        Fact::new("VERSION", dash(&p.version)),
        Fact::new("AUTHOR", dash(&p.author)),
        Fact::new("MADE FOR", dash(&p.game_version)),
        Fact::new("STUDIO", dash(&p.studio_version)),
        Fact::new("SIZE", size(p.size())),
        Fact::new("IMPORTED", date(p.imported)),
    ])];
    match diff {
        Some(d) => detail.extend(changes(p, d, exe)),
        None => detail.extend(files(p)),
    }
    detail.extend(clash_blocks);
    if !p.skipped.is_empty() {
        detail.push(Block::Names {
            label: "SKIPPED".into(),
            items: p.skipped.clone(),
        });
    }
    detail.push(Block::Buttons(vec![
        Button {
            icon: Some(Icon::FolderOpen),
            ..Button::new(format!("{SHOW}{}", p.id), "Show files")
        },
        Button {
            icon: Some(Icon::Trash),
            style: Style::Danger,
            kind: ButtonKind::Confirm {
                title: format!("Remove {}?", p.name),
                text: "It is deleted from the library and turned off in every profile.".into(),
            },
            ..Button::new(format!("{REMOVE}{}", p.id), "Remove from library")
        },
    ]));
    Row {
        id: p.id.clone(),
        title: p.name.clone(),
        subtitle: subtitle(p),
        art: Art::Monogram(monogram(&p.name)),
        tags: tags(p, exe).into_iter().chain(clash_tags).collect(),
        trailing: Some(Control::Switch {
            id: p.id.clone(),
            on,
            enabled: switchable,
        }),
        detail,
    }
}

impl Addon for Simplified {
    fn manifest(&self) -> &AddonManifest {
        &self.manifest
    }

    fn icon(&self) -> Option<&'static [u8]> {
        Some(include_bytes!("../icon.svg"))
    }

    fn sections(&self) -> Vec<SectionSpec> {
        vec![SectionSpec {
            id: SECTION.into(),
            title: "Mods".into(),
            icon: Icon::Package,
        }]
    }

    fn view(&self, cx: &Cx, section: &str) -> Result<View> {
        if section != SECTION {
            return Err(Error::NoSection(section.into()));
        }
        let loaded = Loaded::read(cx)?;
        let on = loaded.enabled().count();
        let mut notices = Vec::new();
        if cx.profile.vanilla {
            notices.push(Notice {
                text: "The vanilla profile plays without addons or mods.".into(),
                tone: Tone::Muted,
                action: None,
            });
        } else if !cx.profile.enabled {
            notices.push(Notice {
                text: format!("Simplified for Swofty is off in {}.", cx.profile.name),
                tone: Tone::Warn,
                action: Some(Button::new(ENABLE, "Turn on")),
            });
        }
        let exe = cx.game.and_then(|g| g.exe_version());
        let key = cx.game.map(|g| g.build_key());
        let library = Library::new(cx.data_dir);
        let diffs: Vec<Option<Diff>> = loaded
            .order
            .iter()
            .map(|(p, _)| key.as_deref().and_then(|k| library.cached(&p.id, k)))
            .collect();
        let missing = diffs.iter().filter(|d| d.is_none()).count();
        if key.is_some() && missing > 0 {
            notices.push(Notice {
                text: format!("{missing} mod(s) are not compared with this game build yet."),
                tone: Tone::Muted,
                action: Some(Button::new(COMPARE, "Compare")),
            });
        }
        let on_diffs: Vec<Option<&Diff>> = loaded
            .order
            .iter()
            .zip(&diffs)
            .map(|((_, on), d)| d.as_ref().filter(|_| *on))
            .collect();
        let clashes = merge::clashes(&on_diffs);
        let name = |i: usize| loaded.order[i].0.name.as_str();
        let body = if loaded.order.is_empty() {
            Body::Empty {
                title: "No mods yet".into(),
                text: "Drop a mod on the window, or import its zip, 7z, rar or folder. Mods made with Simplified Studio import too."
                    .into(),
                action: Some(import_button(true)),
            }
        } else {
            Body::List {
                rows: loaded
                    .order
                    .iter()
                    .zip(&diffs)
                    .enumerate()
                    .map(|(i, ((p, on), d))| {
                        let shown = Shown {
                            on: *on,
                            switchable: !cx.profile.vanilla,
                            exe: exe.as_deref(),
                            diff: d.as_ref(),
                            wins: clashes
                                .iter()
                                .filter(|c| c.winner == i)
                                .map(|c| (name(c.loser), c))
                                .collect(),
                            losses: clashes
                                .iter()
                                .filter(|c| c.loser == i)
                                .map(|c| (name(c.winner), c))
                                .collect(),
                        };
                        row(p, shown)
                    })
                    .collect(),
                reorder: !cx.profile.vanilla,
            }
        };
        Ok(View {
            subtitle: format!(
                "{} in library · {on} on in {} · lower in the list wins",
                loaded.order.len(),
                cx.profile.name
            ),
            actions: vec![import_button(false)],
            notices,
            body,
            footer: Some(FOOTER.into()),
        })
    }

    fn event(&self, cx: &Cx, section: &str, event: Event) -> Result<Effect> {
        if section != SECTION {
            return Err(Error::NoSection(section.into()));
        }
        match event {
            Event::Toggle { id, on } => {
                let mut loaded = Loaded::read(cx)?;
                loaded.set(&id, on)?;
                loaded.save(cx)?;
                Ok(Effect::Reload)
            }
            Event::Move { row, to } => {
                let mut loaded = Loaded::read(cx)?;
                loaded.move_to(&row, to)?;
                loaded.save(cx)?;
                Ok(Effect::Reload)
            }
            Event::Picked { id, paths } if id == IMPORT => {
                Ok(Effect::Notice(import(cx, &paths, &NoProgress)?.join("\n")))
            }
            Event::Click { id } if id == ENABLE => Ok(Effect::SetAddonEnabled(true)),
            Event::Click { id } if id == COMPARE => {
                let game = cx
                    .game
                    .ok_or_else(|| err("no game was found to compare with".into()))?;
                let library = Library::new(cx.data_dir);
                for p in library.list()? {
                    library.diff(&p, game)?;
                }
                Ok(Effect::Reload)
            }
            Event::Click { id } if id.starts_with(SHOW) => {
                let dir = Library::new(cx.data_dir)
                    .dir(&id[SHOW.len()..])
                    .join(library::FILES);
                Ok(Effect::Open(dir))
            }
            Event::Click { id } if id.starts_with(REMOVE) => {
                let p = Library::new(cx.data_dir).remove(&id[REMOVE.len()..])?;
                if !cx.profile.vanilla {
                    Loaded::read(cx)?.save(cx)?;
                }
                Ok(Effect::Notice(format!("Removed {} from the library", p.name)))
            }
            _ => Ok(Effect::None),
        }
    }

    fn importer(&self) -> Option<&dyn Importer> {
        Some(self)
    }

    fn build(&self, cx: &BuildCx) -> Result<AssetBatch> {
        let cx = &cx.cx;
        let mut batch = AssetBatch::default();
        let Some(game) = cx.game.filter(|_| !cx.profile.vanilla) else {
            return Ok(batch);
        };
        let library = Library::new(cx.data_dir);
        let loaded = Loaded::read(cx)?;
        let mut carried = Vec::new();
        let mut defaults = Vec::new();
        for p in loaded.enabled() {
            if p.track.is_some() {
                batch.lines.push(format!("{}: track {}", p.name, p.id));
                batch.ops.push(Op {
                    op: track::op(p),
                    origin: Origin {
                        addon: ID.into(),
                        item: Some(p.id.clone()),
                    },
                });
                continue;
            }
            let diff = library.diff(p, game)?;
            for a in diff
                .archives
                .iter()
                .filter(|a| texarray::carrier(&a.path) && !a.added.is_empty())
            {
                carried.push(texarray::Carried {
                    item: p.name.clone(),
                    erp: library.dir(&p.id).join(library::FILES).join(&a.path),
                    uris: a
                        .added
                        .iter()
                        .filter(|uri| !uri.starts_with(merge::MARKER))
                        .cloned()
                        .collect(),
                });
            }
            let records = library.records(&p.id, &diff.build_key)?;
            let merged = merge::ops(&p.id, &diff, &records);
            batch.lines.push(format!(
                "{}: {} record(s), {} resource(s), {} text entr(ies), {} file(s)",
                p.name, merged.records, merged.resources, merged.strings, merged.files
            ));
            if merged.carried > 0 {
                batch.lines.push(format!(
                    "{}: {} resource(s) carried in shader archives load with {}",
                    p.name,
                    merged.carried,
                    merge::BOOT_ERP
                ));
            }
            if diff.dropped > 0 {
                batch.warnings.push(format!(
                    "{}: {} removal(s) left out; made for {}",
                    p.name,
                    diff.dropped,
                    p.game_version.as_deref().unwrap_or("an unknown version")
                ));
            }
            if merged.removals > 0 {
                batch.lines.push(format!(
                    "{}: {} resource(s) removed from the game's archives",
                    p.name, merged.removals
                ));
            }
            let origin = Origin {
                addon: ID.into(),
                item: Some(p.id.clone()),
            };
            defaults.extend(merged.defaults.into_iter().map(|op| Op {
                op,
                origin: origin.clone(),
            }));
            batch.ops.extend(merged.ops.into_iter().map(|op| Op {
                op,
                origin: origin.clone(),
            }));
        }
        batch.ops.splice(0..0, only_defaults(defaults, &batch.ops));
        batch.ops = without_repeats(std::mem::take(&mut batch.ops));
        let ui = texarray::build(game, &carried, &cx.cache_dir.join(texarray::MERGED))?;
        if !ui.ops.is_empty() {
            batch.lines.push(format!(
                "ui textures: {} resource(s) load with {}",
                ui.ops.len(),
                texarray::UI_ERP
            ));
        }
        batch.lines.extend(ui.lines);
        batch.warnings.extend(ui.warnings);
        batch.ops.extend(ui.ops.into_iter().map(|op| Op {
            op,
            origin: Origin {
                addon: ID.into(),
                item: None,
            },
        }));
        Ok(batch)
    }

    fn commands(&self) -> Vec<CommandSpec> {
        let id = |help: &str| ArgSpec {
            name: "id".into(),
            help: help.into(),
            required: true,
            many: false,
        };
        let cmd = |name: &str, about: &str, args: Vec<ArgSpec>| CommandSpec {
            name: name.into(),
            about: about.into(),
            args,
            subcommands: Vec::new(),
        };
        vec![CommandSpec {
            name: "mods".into(),
            about: "Mods in the library and their order in the profile".into(),
            args: Vec::new(),
            subcommands: vec![
                cmd("list", "List the library in load order", Vec::new()),
                cmd(
                    "import",
                    "Import mods into the library",
                    vec![ArgSpec {
                        name: "path".into(),
                        help: "A mod's zip, 7z, rar, .serps or folder, laid out like the game folder".into(),
                        required: true,
                        many: true,
                    }],
                ),
                cmd(
                    "remove",
                    "Delete a mod from the library",
                    vec![id("The mod's id")],
                ),
                cmd("enable", "Turn a mod on in the profile", vec![id("The mod's id")]),
                cmd(
                    "disable",
                    "Turn a mod off in the profile",
                    vec![id("The mod's id")],
                ),
                cmd(
                    "move",
                    "Move a mod to a place in the load order (1 loads first)",
                    vec![
                        id("The mod's id"),
                        ArgSpec {
                            name: "position".into(),
                            help: "The new place, from 1".into(),
                            required: true,
                            many: false,
                        },
                    ],
                ),
            ],
        }]
    }

    fn command(&self, cx: &Cx, path: &[String], args: &[String]) -> Result<String> {
        let verb = match path {
            [top, verb] if top == "mods" => verb.as_str(),
            _ => return Err(Error::NoCommand(path.join(" "))),
        };
        let arg = |i: usize| {
            args.get(i)
                .map(String::as_str)
                .ok_or_else(|| err(format!("mods {verb}: missing argument")))
        };
        let refuse_vanilla = || match cx.profile.vanilla {
            true => Err(err("the vanilla profile plays without mods".into())),
            false => Ok(()),
        };
        match verb {
            "list" => {
                let loaded = Loaded::read(cx)?;
                if loaded.order.is_empty() {
                    return Ok("no mods in the library".into());
                }
                let lines: Vec<String> = loaded
                    .order
                    .iter()
                    .enumerate()
                    .map(|(i, (p, on))| {
                        let by = subtitle(p);
                        let by = if by.is_empty() {
                            String::new()
                        } else {
                            format!(" ({by})")
                        };
                        format!(
                            "{:>2}. [{}] {}  {}{by}",
                            i + 1,
                            if *on { "on " } else { "off" },
                            p.id,
                            p.name
                        )
                    })
                    .collect();
                Ok(lines.join("\n"))
            }
            "import" => {
                let paths: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
                Ok(import(cx, &paths, &NoProgress)?.join("\n"))
            }
            "remove" => {
                let p = Library::new(cx.data_dir).remove(arg(0)?)?;
                if !cx.profile.vanilla {
                    Loaded::read(cx)?.save(cx)?;
                }
                Ok(format!("removed {} from the library", p.name))
            }
            "enable" | "disable" => {
                refuse_vanilla()?;
                let on = verb == "enable";
                let mut loaded = Loaded::read(cx)?;
                let name = loaded.set(arg(0)?, on)?.name.clone();
                loaded.save(cx)?;
                Ok(format!(
                    "{name} is {} in {}",
                    if on { "on" } else { "off" },
                    cx.profile.name
                ))
            }
            "move" => {
                refuse_vanilla()?;
                let to: usize = arg(1)?
                    .parse()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or_else(|| err(format!("{} is not a position from 1", arg(1).unwrap_or(""))))?;
                let mut loaded = Loaded::read(cx)?;
                let name = loaded.move_to(arg(0)?, to - 1)?.name.clone();
                loaded.save(cx)?;
                let at = loaded.find(arg(0)?)? + 1;
                Ok(format!("{name} is number {at} in {}", cx.profile.name))
            }
            _ => Err(Error::NoCommand(path.join(" "))),
        }
    }

    fn summary(&self, cx: &Cx) -> Option<Summary> {
        if cx.profile.vanilla {
            return None;
        }
        let loaded = Loaded::read(cx).ok()?;
        Some(Summary {
            label: "MODS".into(),
            items: loaded.enabled().map(|p| p.name.clone()).collect(),
        })
    }

    fn migrate(&self, data: &Path, version: u32) -> Result<Vec<String>> {
        if version >= 3 {
            return Ok(Vec::new());
        }
        track::migrate(data)
    }
}

impl Importer for Simplified {
    fn claims(&self, path: &Path) -> Claim {
        if track::claims(path) == Claim::Yes {
            return Claim::Yes;
        }
        package::claims(path)
    }

    fn import(&self, cx: &Cx, paths: &[PathBuf], progress: &dyn Progress) -> Result<Vec<String>> {
        import(cx, paths, progress)
    }
}

struct NoProgress;

impl Progress for NoProgress {
    fn report(&self, _: f32, _: &str) {}
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn set(item: &str, key: &str, value: &str) -> Op {
        Op {
            op: AssetOp::Lang(LangOp::Set {
                language: "eng".into(),
                key: key.into(),
                value: value.into(),
            }),
            origin: Origin {
                addon: ID.into(),
                item: Some(item.into()),
            },
        }
    }

    fn record(item: &str, value: &str) -> Op {
        Op {
            op: AssetOp::Db(DbOp::PutRecord {
                key: "0x1".into(),
                tag: 2,
                value: value.into(),
            }),
            origin: Origin {
                addon: ID.into(),
                item: Some(item.into()),
            },
        }
    }

    fn items(ops: &[Op]) -> Vec<&str> {
        ops.iter().map(|op| op.origin.item.as_deref().unwrap()).collect()
    }

    #[test]
    fn a_write_that_repeats_the_value_in_place_is_left_out() {
        let ops = vec![
            record("a", "6c"),
            record("b", "6c"),
            record("c", "6d"),
            record("d", "6c"),
        ];
        assert_eq!(items(&without_repeats(ops)), ["a", "c", "d"]);
    }

    #[test]
    fn a_removal_of_something_already_removed_is_left_out() {
        let op = |item: &str, op: AssetOp| Op {
            op,
            origin: Origin {
                addon: ID.into(),
                item: Some(item.into()),
            },
        };
        let remove = |item: &str| {
            op(
                item,
                AssetOp::Erp(ErpOp::RemoveResource {
                    erp: "a.erp".into(),
                    resource: "eaid://r".into(),
                }),
            )
        };
        let put = |item: &str| {
            op(
                item,
                AssetOp::Erp(ErpOp::PutResource {
                    erp: "a.erp".into(),
                    resource: "eaid://r".into(),
                    source: swofty_addon::asset::Source::addon("x.erp"),
                    rename: None,
                }),
            )
        };
        let drop_record = |item: &str| op(item, AssetOp::Db(DbOp::RemoveRecord { key: "0x1".into() }));
        let ops = vec![
            remove("a"),
            remove("b"),
            put("c"),
            remove("d"),
            drop_record("e"),
            record("f", "6c"),
            drop_record("g"),
            drop_record("h"),
        ];
        assert_eq!(items(&without_repeats(ops)), ["a", "c", "d", "e", "f", "g"]);
    }

    #[test]
    fn defaults_go_in_once_and_only_for_keys_nobody_renames() {
        let renames = vec![set("b", "ast", "Genesis")];
        let defaults = vec![
            set("a", "ast", "Aston"),
            set("a", "alp", "Alpine"),
            set("b", "alp", "Alpine"),
        ];
        let kept = only_defaults(defaults, &renames);
        assert_eq!(items(&kept), ["a"]);
        assert!(matches!(&kept[0].op, AssetOp::Lang(LangOp::Set { key, .. }) if key == "alp"));
    }
}
