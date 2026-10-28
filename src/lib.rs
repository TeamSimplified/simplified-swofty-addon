pub mod about;
pub mod convert;
pub mod diff;
pub mod grid;
pub mod importer;
pub mod library;
pub mod log;
pub mod merge;
pub mod package;
pub mod page;
pub mod serps;
pub mod texarray;
pub mod track;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use swofty_addon::asset::{AssetBatch, AssetOp, LangOp, Op, Origin};
use swofty_addon::launcher::{Addon, BuildCx, Builtin, Claim, CommandSpec, Cx, Importer, Progress, Summary};
use swofty_addon::manifest::{AddonManifest, Launcher};
use swofty_addon::ops::{DbOp, ErpOp};
use swofty_addon::view::{Button, Effect, Event, Notice, SectionSpec, Tone, View};
use swofty_addon::{Error, Result};

use crate::importer::{Converter, Record};
use crate::library::{Library, Package};

pub const ID: &str = "teamsimplified.mods";
// The origin.tool 0.10.0 gave an addon made from a mod that leaves the shader packages
// alone. Every addon carries ID, which the launcher tags Simplified; startup switches
// these back.
pub const COPY_PASTE: &str = "teamsimplified.mods/copy-paste";
pub const SECTION: &str = "mods";
pub(crate) const MANIFEST: &str = include_str!("../swofty.json");
// Team Simplified's SERPs Base Files for F1 25 v4.1, shipped with the addon: each
// archive in place of the game's, as the Simplified Launcher installs them.
const BASE_FILES: [(&str, &str); 3] = [
    (
        "2025_asset_groups/vfx_package/common.erp",
        "base/vfx_package/common.erp",
    ),
    (
        "2025_asset_groups/f1_2025_vehicle_package/teams/common.erp",
        "base/f1_2025_vehicle_package/teams/common.erp",
    ),
    (
        "2025_asset_groups/f1_2026_vehicle_package/teams/common.erp",
        "base/f1_2026_vehicle_package/teams/common.erp",
    ),
];
const IMPORT: &str = "import";
const ENABLE: &str = "enable";

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
                AssetOp::Db(DbOp::RemoveRecord { key, .. }) => (('d', key.clone(), String::new()), None),
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

// Only the logo texture arrays of a mod's ui_texture archive go into the merged UI
// archive, slice by slice. Everything else it adds loads with the mod's own archive
// (merge::ops), and what Simplified Studio adds to every shader package with neither.
fn ui_uris(added: &[String]) -> Vec<String> {
    added
        .iter()
        .filter(|uri| texarray::merged_by_slice(uri) && !merge::studio_only(uri))
        .cloned()
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
    // The mods this profile loads as their addons, which the build hook leaves out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub id: String,
    pub enabled: bool,
}

pub struct Simplified {
    manifest: AddonManifest,
    grid: grid::Grid,
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
            grid: grid::Grid::default(),
        }
    }

    fn launcher(&self) -> &Launcher {
        self.manifest
            .launcher
            .as_ref()
            .expect("swofty.json declares the launcher block")
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
        let addons = cx.state::<State>()?.addons;
        cx.set_state(&State { mods, addons })
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

fn notices(cx: &Cx) -> Vec<Notice> {
    if cx.profile.vanilla {
        return vec![Notice {
            text: "The vanilla profile plays without addons or mods. Switch to another profile to import mods; the addons made before are listed here.".into(),
            tone: Tone::Muted,
            action: None,
        }];
    }
    if !cx.profile.enabled {
        return vec![Notice {
            text: format!("Simplified for Swofty is off in {}.", cx.profile.name),
            tone: Tone::Warn,
            action: Some(Button::new(ENABLE, "Turn on")),
        }];
    }
    Vec::new()
}

fn subtitle(p: &Package) -> String {
    [p.author.as_deref(), p.version.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Addon for Simplified {
    fn manifest(&self) -> &AddonManifest {
        &self.manifest
    }

    fn icon(&self) -> Option<&'static [u8]> {
        Some(include_bytes!("../icon.svg"))
    }

    // The Simplified Importer and About pages and the mods command are declared once, in swofty.json,
    // which the launcher also reads when it runs this addon as a helper program.
    fn sections(&self) -> Vec<SectionSpec> {
        self.launcher().sections.clone()
    }

    fn view(&self, cx: &Cx, section: &str) -> Result<View> {
        match section {
            SECTION => page::view(cx),
            about::SECTION => about::view(cx),
            grid::SECTION => self.grid.view(cx),
            _ => Err(Error::NoSection(section.into())),
        }
    }

    fn tick(&self, cx: &Cx, section: &str) -> Result<bool> {
        Ok(section == grid::SECTION && self.grid.tick(cx))
    }

    fn event(&self, cx: &Cx, section: &str, event: Event) -> Result<Effect> {
        self.event_with(cx, section, event, &Converter)
    }

    fn importer(&self) -> Option<&dyn Importer> {
        Some(self)
    }

    fn build(&self, bcx: &BuildCx) -> Result<AssetBatch> {
        let cx = &bcx.cx;
        let mut batch = AssetBatch::default();
        let Some(game) = cx.game.filter(|_| !cx.profile.vanilla) else {
            return Ok(batch);
        };
        let library = Library::new(cx.data_dir);
        let loaded = Loaded::read(cx)?;
        let addons = importer::handed_over(cx)?;
        let mut carried = Vec::new();
        let mut defaults = Vec::new();
        for p in loaded.enabled().filter(|p| !addons.contains(&p.id)) {
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
                    uris: ui_uris(&a.added),
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
        if let Some(by) = needs_base(cx)? {
            batch.lines.push(format!(
                "SERPs Base Files: {} archive(s) load with {by}, which needs them",
                BASE_FILES.len()
            ));
            batch.ops.extend(BASE_FILES.iter().map(|(erp, source)| Op {
                op: AssetOp::Erp(ErpOp::Overlay {
                    erps: vec![(*erp).to_owned()],
                    source: swofty_addon::asset::Source::addon(*source),
                }),
                origin: Origin {
                    addon: ID.into(),
                    item: Some("serps-base-files".into()),
                },
            }));
        }
        // The Grid Editor's edits last, over the mods above.
        self.grid.build(bcx, &mut batch)?;
        Ok(batch)
    }

    fn commands(&self) -> Vec<CommandSpec> {
        self.launcher().commands.clone()
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
                let record = Record::load(cx.data_dir)?;
                let addons = importer::handed_over(cx)?;
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
                        let state = match (record.mods.get(&p.id), addons.contains(&p.id)) {
                            (Some(e), true) => {
                                format!("-> {}", e.made.as_ref().map_or("", |m| m.id.as_str()))
                            }
                            (Some(e), false) if e.made.is_some() => "addon".into(),
                            (Some(e), false) => format!(
                                "[{}] not converted: {}",
                                if *on { "on " } else { "off" },
                                e.error.as_deref().unwrap_or("")
                            ),
                            (None, _) => format!("[{}]", if *on { "on " } else { "off" }),
                        };
                        format!("{:>2}. {state} {}  {}{by}", i + 1, p.id, p.name)
                    })
                    .collect();
                Ok(lines.join("\n"))
            }
            "import" => {
                let paths: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
                let done = importer::import(cx, &paths, &NoProgress, &Converter)?;
                match done.failed {
                    true => Err(err(done.lines.join("\n"))),
                    false => Ok(done.lines.join("\n")),
                }
            }
            "remove" => Ok(format!("removed {} from the library", remove(cx, arg(0)?)?.name)),
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
        let mods = Loaded::read(cx)
            .ok()
            .zip(importer::handed_over(cx).ok())
            .and_then(|(loaded, addons)| {
                let items: Vec<String> = loaded
                    .enabled()
                    .filter(|p| !addons.contains(&p.id))
                    .map(|p| p.name.clone())
                    .collect();
                (!items.is_empty()).then(|| Summary {
                    label: "MODS".into(),
                    items,
                })
            });
        // One summary per addon: with both, the mods and the grid edits under one name.
        match (mods, self.grid.summary(cx)) {
            (Some(m), Some(g)) => Some(Summary {
                label: "SIMPLIFIED".into(),
                items: m.items.into_iter().chain(g.items).collect(),
            }),
            (m, g) => m.or(g),
        }
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
        let done = importer::import(cx, paths, progress, &Converter)?;
        match done.failed {
            true => Err(err(done.lines.join("\n"))),
            false => Ok(done.lines),
        }
    }
}

impl Simplified {
    fn event_with(
        &self,
        cx: &Cx,
        section: &str,
        event: Event,
        conv: &dyn importer::Convert,
    ) -> Result<Effect> {
        // The launcher sends startup to the first section, About; the importer runs it.
        let section = match event {
            Event::Startup {} => SECTION,
            _ => section,
        };
        match section {
            SECTION => Ok(page::event(cx, event, conv)?.unwrap_or(Effect::None)),
            about::SECTION => about::event(event),
            grid::SECTION => self.grid.event(cx, event),
            _ => Err(Error::NoSection(section.into())),
        }
    }
}

// The first enabled addon made from a mod that needs the SERPs Base Files.
pub(crate) fn needs_base(cx: &Cx) -> Result<Option<String>> {
    let record = Record::load(cx.data_dir)?;
    let enabled: Option<HashSet<String>> = cx
        .addons()
        .map(|a| {
            a.list()
                .map(|l| l.into_iter().filter(|a| a.enabled).map(|a| a.id).collect())
        })
        .transpose()?;
    Ok(record
        .mods
        .values()
        .filter_map(|e| e.made.as_ref())
        .filter(|m| m.base && enabled.as_ref().is_none_or(|on| on.contains(&m.id)))
        .map(|m| m.name.clone())
        .next())
}

fn remove(cx: &Cx, id: &str) -> Result<Package> {
    let p = Library::new(cx.data_dir).remove(id)?;
    let mut record = Record::load(cx.data_dir)?;
    if record.mods.remove(id).is_some() {
        record.save(cx.data_dir)?;
    }
    if !cx.profile.vanilla {
        Loaded::read(cx)?.save(cx)?;
    }
    Ok(p)
}

pub(crate) struct NoProgress;

impl Progress for NoProgress {
    fn report(&self, _: f32, _: &str) {}
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    #[test]
    fn counts_read_as_words() {
        assert_eq!(plural(1, "mod", "mods"), "1 mod");
        assert_eq!(plural(16, "mod", "mods"), "16 mods");
    }

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
    fn only_logo_arrays_go_into_the_ui_archive() {
        let logos =
            "eaid://ui_package/art/texture_arrays/overrides.uidb/overrides.team_logos.f1_2026.texarray.image";
        let added: Vec<String> = [
            logos,
            "eaid://ui_package/art/textures/broadcast_ui_generic/bc_starting_grid/bc_starting_grid_grad_1.tif.image",
            "eaid://tools_package/default/serps",
            "eaid://f1_2025_vehicle_package/shared/shadows/materials/f1_vehicle_shadow.material",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(ui_uris(&added), [logos]);
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
        let drop_record = |item: &str| {
            op(
                item,
                AssetOp::Db(DbOp::RemoveRecord {
                    key: "0x1".into(),
                    allow_stock_removal: false,
                }),
            )
        };
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
