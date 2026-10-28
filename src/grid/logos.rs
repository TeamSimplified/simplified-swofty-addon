// Team logos. A logo is a slice of a texture array, and the same team sits in
// several arrays, one per place the game draws it. Which slice is a team's is the
// team's position in the array's list in the UI database (overrides.uidb): the
// arrays name teams by their database label, and the timing tower arrays by the
// team's timing_tower_texture_path. Reading the list rather than keeping a table
// makes the teams addons add, such as the Classic Seasons ones, work the same.
//
// The arrays are read the way a build would leave them: the game's copy in each
// archive that carries it, with every other enabled addon's put-resource,
// merge-slices and overlay ops on it played in load order.
//
// Addons can leave an array differing between archives: Classic Seasons grows the
// arrays with its teams in the UI archives, while a team mod merges its logo into
// a game-sized copy it adds to vfx_package/common.erp, which the game then shows,
// so a Classic Seasons team reads past its end and gets the last slice, the 2026
// Williams logo. Where an addon grew an array, the editor joins every copy into
// one, matching teams by name since the grown lists move teams along, and writes
// it to every archive whose copy differs.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use swofty_addon::ops::ErpOp;
use swofty_formats::erp::{Erp, ErpFile, Resource};
use swofty_formats::hash::fnv1a64;
use swofty_formats::pkg::{Node, Pkg};
use swofty_formats::texarray::Array;
use swofty_formats::texture::{self, Codec};

use crate::grid::bc7;
use crate::grid::image::{self, Image};
use crate::grid::world::ErpChange;

const UIDB: &str = "eaid://ui_package/databases/overrides.uidb";
const DATABASES: &str = "2025_asset_groups/ui_package/databases.erp";
const UI_DIR: &str = "2025_asset_groups/ui_package";

pub fn resource(array: &str) -> String {
    format!("eaid://ui_package/art/texture_arrays/overrides.uidb/overrides.{array}.texarray.image")
}

/// The array's view resource, which goes with its picture resource.
pub fn descriptor(array: &str) -> String {
    format!("eaid://ui_package/art/texture_arrays/overrides.uidb/overrides.{array}.texarray")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyed {
    Label,
    Timing,
}

/// A place a logo shows, and the arrays that hold it there with the name each
/// lists the team by.
pub struct Kind {
    pub id: &'static str,
    pub label: &'static str,
    pub arrays: &'static [(&'static str, Keyed)],
}

/// The places a logo goes, in the editor's order, before the grid's own showroom
/// tiles. Standings, results and the timing tower share one 24 by 24 picture.
pub const KINDS: [Kind; 3] = [
    Kind {
        id: "standings",
        label: "Timing tower",
        arrays: &[
            ("teams.badges_broadcast", Keyed::Label),
            ("osd.timing_tower.team_badges_hud", Keyed::Timing),
            ("osd.timing_tower.team_badges_broadcast", Keyed::Timing),
        ],
    },
    Kind {
        id: "team_select",
        label: "Team selection",
        arrays: &[("team_logos.driver_career_2024_team_select", Keyed::Label)],
    },
    Kind {
        id: "starting_grid",
        label: "Starting grid",
        arrays: &[("broadcast_ui_generic.grid_team_logos", Keyed::Label)],
    },
];

pub const SHOWROOM: &str = "showroom";

/// The arrays of a place for a grid: its showroom tiles are an array per grid.
pub fn arrays_of(kind: &str, ui_path: &str) -> Vec<(String, Keyed)> {
    if kind == SHOWROOM {
        return [
            format!("team_logos.{ui_path}"),
            format!("team_logos.{ui_path}_showroom"),
        ]
        .into_iter()
        .map(|a| (a, Keyed::Label))
        .collect();
    }
    KINDS
        .iter()
        .find(|k| k.id == kind)
        .map(|k| {
            k.arrays
                .iter()
                .map(|(a, keyed)| (a.to_string(), *keyed))
                .collect()
        })
        .unwrap_or_default()
}

pub fn kind_label(kind: &str) -> &'static str {
    if kind == SHOWROOM {
        return "Showroom";
    }
    KINDS.iter().find(|k| k.id == kind).map_or("Logo", |k| k.label)
}

pub fn all_kinds() -> Vec<&'static str> {
    KINDS
        .iter()
        .map(|k| k.id)
        .chain(std::iter::once(SHOWROOM))
        .collect()
}

#[derive(Debug, Clone)]
pub struct State {
    pub resource: Resource,
    /// The array's view resource from the same source, when it has one.
    pub descriptor: Option<Resource>,
    pub array: Array,
    /// The addon whose picture each slice holds, where it isn't the game's.
    pub owners: HashMap<usize, String>,
}

/// The game's list of an array and the one the profile's build has, to find a
/// team's slice in either.
struct Names<'a> {
    stock: &'a [String],
    built: &'a [String],
}

impl Names<'_> {
    /// Where the slice at `i` of a copy with `count` slices sits in the built list:
    /// a game-sized copy follows the game's list.
    fn built_index(&self, i: usize, count: usize, stock_count: usize) -> Option<usize> {
        if count == stock_count && self.stock.len() == stock_count && self.built.len() != stock_count {
            let name = self.stock.get(i)?;
            self.built.iter().position(|n| n == name)
        } else {
            Some(i)
        }
    }

    /// Where the team at `i` of the built list sits in the game's array.
    fn stock_index(&self, i: usize, stock_count: usize) -> Option<usize> {
        if self.built.len() == stock_count || self.built.is_empty() {
            return Some(i).filter(|&i| i < stock_count);
        }
        let name = self.built.get(i)?;
        self.stock
            .iter()
            .position(|n| n == name)
            .filter(|&k| k < stock_count)
    }
}

impl State {
    fn stock(resource: &Resource, descriptor: Option<&Resource>, array: &Array) -> State {
        State {
            resource: resource.clone(),
            descriptor: descriptor.cloned(),
            array: array.clone(),
            owners: HashMap::new(),
        }
    }

    /// A whole array an addon put in: its slices that aren't the game's picture for
    /// that team are the addon's.
    fn whole(
        resource: Resource,
        descriptor: Option<Resource>,
        array: Array,
        stock: &Array,
        header: &texture::Header,
        addon: &str,
        names: &Names,
    ) -> State {
        let owners = (0..array.count)
            .filter(|&i| {
                if !array.fits(stock) {
                    return true;
                }
                let k = if array.count == stock.count {
                    Some(i)
                } else {
                    names.stock_index(i, stock.count)
                };
                k.is_none_or(|k| !looks_same(header, array.slice(i), stock.slice(k)))
            })
            .map(|i| (i, addon.to_owned()))
            .collect();
        State {
            resource,
            descriptor,
            array,
            owners,
        }
    }
}

/// Whether two slices show the same picture: the same bytes, or a copy encoded again
/// whose pixels are, on average, within a couple of levels of the game's. A different
/// logo is far past that.
fn looks_same(header: &texture::Header, a: &[u8], b: &[u8]) -> bool {
    if a == b {
        return true;
    }
    if header.codec() != Some(Codec::Bc7) {
        return false;
    }
    let level = Codec::Bc7.level_bytes(header.width, header.height);
    if a.len() < level || b.len() < level {
        return false;
    }
    let decode = |s: &[u8]| texture::decode_pixels(Codec::Bc7, header.width, header.height, &s[..level]).ok();
    let (Some(pa), Some(pb)) = (decode(a), decode(b)) else {
        return false;
    };
    if pa.is_empty() || pa.len() != pb.len() {
        return false;
    }
    let total: u64 = pa.iter().zip(&pb).map(|(x, y)| u64::from(x.abs_diff(*y))).sum();
    total as f64 / pa.len() as f64 <= 2.0
}

pub struct Stock {
    pub carriers: Vec<String>,
    pub resource: Resource,
    pub array: Array,
    pub header: texture::Header,
    pub template: PathBuf,
}

pub struct Shelf {
    pub stock: Stock,
    /// The array as each archive will hold it before this addon: every carrier,
    /// and every archive another addon writes it into.
    pub targets: BTreeMap<String, State>,
    /// The archive whose copy the game is likeliest to show: the one the last addon
    /// to change the array wrote, else the first carrier.
    pub shown: String,
    /// The array the editor shows and builds on: the shown copy, or, where an addon
    /// grew the array, every copy joined over the grown one.
    pub combined: State,
    /// An addon grew the array, so every archive gets the joined array whole.
    pub grown: bool,
}

impl Shelf {
    pub fn shown(&self) -> &State {
        &self.combined
    }
}

/// Every copy of an array joined into one: the grown copy the last addon wrote,
/// with the slices other addons put in the other copies on top, each at its team's
/// place in the grown list. None when no addon grew the array.
fn combine(targets: &BTreeMap<String, State>, shown: &str, stock: &Array, names: &Names) -> Option<State> {
    let longest = targets.values().map(|t| t.array.count).max()?;
    if longest <= stock.count {
        return None;
    }
    let base_key = if targets[shown].array.count == longest {
        shown.to_owned()
    } else {
        targets.iter().find(|(_, t)| t.array.count == longest)?.0.clone()
    };
    let mut out = targets[&base_key].clone();
    // The shown copy goes last, so its slices win.
    let mut order: Vec<String> = targets
        .keys()
        .filter(|k| **k != base_key && *k != shown)
        .cloned()
        .collect();
    order.push(shown.to_owned());
    let n = out.array.slice_len();
    for key in order {
        if key == base_key {
            continue;
        }
        let t = &targets[&key];
        if !t.array.fits(&out.array) {
            continue;
        }
        for (&i, owner) in &t.owners {
            let Some(j) = names.built_index(i, t.array.count, stock.count) else {
                continue;
            };
            if i < t.array.count && j < out.array.count {
                out.array.data[j * n..(j + 1) * n].copy_from_slice(t.array.slice(i));
                out.owners.insert(j, owner.clone());
            }
        }
    }
    Some(out)
}

pub struct Library {
    pub lists: HashMap<String, Vec<String>>,
    /// The game's own lists, before any addon added teams.
    pub stock_lists: HashMap<String, Vec<String>>,
    pub shelves: HashMap<String, Shelf>,
    pub notes: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Carriers {
    key: String,
    arrays: BTreeMap<String, Vec<String>>,
}

fn uidb_pkg(path: &Path) -> Option<Pkg> {
    let file = ErpFile::open(path).ok()?;
    let res = file.load(file.erp.find(UIDB).ok()?).ok()?;
    Pkg::parse(&res.fragments.first()?.data().ok()?).ok()
}

fn lists_of(pkg: &Pkg) -> Option<HashMap<String, Vec<String>>> {
    let top = pkg.root.as_obj()?.get("overrides")?.as_obj()?.get("Contents")?;
    let mut out = HashMap::new();
    walk(top, "", &mut out);
    Some(out)
}

fn uidb_lists(path: &Path) -> Option<HashMap<String, Vec<String>>> {
    lists_of(&uidb_pkg(path)?)
}

/// The lists as the profile's addons leave them: the game's, with every addon's
/// patch-pkg edits to the UI database played in load order, as a build plays them.
/// Classic Seasons adds its teams this way, so they have logos before the first build,
/// and a build made before an addon was turned on doesn't hide its teams.
fn patched_lists(
    game: &Path,
    changes: &[ErpChange],
    notes: &mut Vec<String>,
) -> Option<HashMap<String, Vec<String>>> {
    let mut pkg = uidb_pkg(&game.join(DATABASES))?;
    for ch in changes {
        let ErpOp::PatchPkg {
            erp,
            resource,
            fragment,
            edits,
        } = &ch.op
        else {
            continue;
        };
        if erp != DATABASES || resource != UIDB || *fragment != 0 {
            continue;
        }
        // An edit copying from another archive isn't one the lists need.
        if let Err(e) = swofty_core::pkgedit::apply_edits(&mut pkg.root, edits, &|_| None) {
            notes.push(format!(
                "{}: its UI database edits don't all apply here: {e}",
                ch.addon
            ));
            return None;
        }
    }
    lists_of(&pkg)
}

fn walk(node: &Node, path: &str, out: &mut HashMap<String, Vec<String>>) {
    let Some(obj) = node.as_obj() else { return };
    for (k, v) in &obj.items {
        let Some(k) = k.as_str() else { continue };
        let Some(contents) = v.as_obj().and_then(|o| o.get("Contents")) else {
            continue;
        };
        let here = if path.is_empty() {
            k.to_owned()
        } else {
            format!("{path}.{k}")
        };
        match contents.as_arr() {
            Some(arr) => {
                let names = arr
                    .items
                    .iter()
                    .filter_map(|e| e.as_obj()?.get("Name")?.as_val()?.as_str().map(str::to_owned))
                    .collect();
                out.insert(here, names);
            }
            None => walk(contents, &here, out),
        }
    }
}

fn ui_archives(game: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for sub in ["", "/screens"] {
        let dir = game.join(format!("{UI_DIR}{sub}"));
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".erp") {
                out.push(format!("{UI_DIR}{sub}/{name}"));
            }
        }
    }
    out.sort();
    out
}

fn carriers(game: &Path, cache: &Path, key: &str, arrays: &[String]) -> BTreeMap<String, Vec<String>> {
    let path = cache.join("carriers.json");
    if let Some(c) = std::fs::read(&path)
        .ok()
        .and_then(|d| serde_json::from_slice::<Carriers>(&d).ok())
        .filter(|c| c.key == key && arrays.iter().all(|a| c.arrays.contains_key(a)))
    {
        return c.arrays;
    }
    let mut found: BTreeMap<String, Vec<String>> = arrays.iter().map(|a| (a.clone(), Vec::new())).collect();
    let by_name: HashMap<String, String> = arrays.iter().map(|a| (resource(a), a.clone())).collect();
    for rel in ui_archives(game) {
        let Ok(file) = ErpFile::open(&game.join(&rel)) else {
            continue;
        };
        for r in &file.erp.resources {
            if let Some(a) = by_name.get(&r.name) {
                found.get_mut(a).expect("listed").push(rel.clone());
            }
        }
    }
    let doc = Carriers {
        key: key.to_owned(),
        arrays: found.clone(),
    };
    if let Ok(text) = serde_json::to_vec(&doc) {
        std::fs::create_dir_all(cache).ok();
        std::fs::write(path, text).ok();
    }
    found
}

fn read_resource(path: &Path, name: &str) -> Option<Resource> {
    let file = ErpFile::open(path).ok()?;
    file.load(file.erp.find(name).ok()?).ok()
}

/// Every array the editor shows, for the grids' showroom paths.
pub fn wanted(ui_paths: &[String]) -> Vec<String> {
    let mut out: Vec<String> = KINDS
        .iter()
        .flat_map(|k| k.arrays.iter().map(|(a, _)| a.to_string()))
        .collect();
    for p in ui_paths.iter().filter(|p| !p.is_empty()) {
        out.push(format!("team_logos.{p}"));
        out.push(format!("team_logos.{p}_showroom"));
    }
    out.sort();
    out.dedup();
    out
}

pub fn load(
    game: &Path,
    built: Option<&Path>,
    cache: &Path,
    build_key: &str,
    arrays: &[String],
    changes: &[ErpChange],
) -> Library {
    let mut notes = Vec::new();
    let stock_lists = uidb_lists(&game.join(DATABASES)).unwrap_or_default();
    // Addons that add teams also add their names to these lists. Played here, else as
    // the last build left them.
    let lists = patched_lists(game, changes, &mut notes)
        .or_else(|| {
            built
                .map(|b| b.join(DATABASES))
                .filter(|p| p.is_file())
                .and_then(|p| uidb_lists(&p))
        })
        .or_else(|| uidb_lists(&game.join(DATABASES)))
        .unwrap_or_else(|| {
            notes.push("the game's UI database doesn't read, so logos aren't shown".into());
            HashMap::new()
        });
    let carried = carriers(game, cache, build_key, arrays);
    let mut sources: HashMap<(PathBuf, String), Option<Resource>> = HashMap::new();
    let mut source = |path: &Path, name: &str| {
        sources
            .entry((path.to_path_buf(), name.to_owned()))
            .or_insert_with(|| read_resource(path, name))
            .clone()
    };
    let mut shelves = HashMap::new();
    let none = Vec::new();
    for a in arrays {
        let name = resource(a);
        let view = descriptor(a);
        let names = Names {
            stock: stock_lists.get(a).unwrap_or(&none),
            built: lists.get(a).unwrap_or(&none),
        };
        let Some(carriers) = carried.get(a).filter(|c| !c.is_empty()) else {
            continue;
        };
        let template = game.join(&carriers[0]);
        let Some(res) = read_resource(&template, &name) else {
            continue;
        };
        let stock_view = read_resource(&template, &view);
        let (Some(array), Ok(header)) = (Array::read(&res), texture::header(&res)) else {
            continue;
        };
        let stock = Stock {
            carriers: carriers.clone(),
            resource: res.clone(),
            array: array.clone(),
            header,
            template,
        };
        let mut targets: BTreeMap<String, State> = carriers
            .iter()
            .map(|c| (c.clone(), State::stock(&res, stock_view.as_ref(), &array)))
            .collect();
        let mut shown = carriers[0].clone();
        for ch in changes {
            let Some(from) = &ch.source else { continue };
            match &ch.op {
                ErpOp::PutResource { erp, resource: r, .. } if *r == name => {
                    if let Some(theirs) = source(from, &name)
                        && let Some(arr) = Array::read(&theirs)
                    {
                        let their_view = source(from, &view);
                        targets.insert(
                            erp.clone(),
                            State::whole(theirs, their_view, arr, &array, &header, &ch.addon, &names),
                        );
                        shown = erp.clone();
                    }
                }
                ErpOp::Overlay { erps, .. } => {
                    if let Some(theirs) = source(from, &name)
                        && let Some(arr) = Array::read(&theirs)
                    {
                        let their_view = source(from, &view);
                        for erp in erps {
                            targets.insert(
                                erp.clone(),
                                State::whole(
                                    theirs.clone(),
                                    their_view.clone(),
                                    arr.clone(),
                                    &array,
                                    &header,
                                    &ch.addon,
                                    &names,
                                ),
                            );
                            shown = erp.clone();
                        }
                    }
                }
                ErpOp::MergeSlices { erp, resource: r, .. } if *r == name => {
                    let Some(theirs) = source(from, &name).and_then(|t| Array::read(&t)) else {
                        continue;
                    };
                    if !theirs.fits(&array) {
                        continue;
                    }
                    let mut cur = targets
                        .get(erp)
                        .filter(|s| s.array.same_layout(&array))
                        .cloned()
                        .unwrap_or_else(|| State::stock(&res, stock_view.as_ref(), &array));
                    let changed = cur.array.merge(&array, &theirs);
                    if !changed.is_empty() {
                        // A slice the addon only encoded again, such as another team's
                        // logo a team mod carries along, still merges but stays the
                        // game's picture.
                        for i in changed {
                            if !looks_same(&header, theirs.slice(i), array.slice(i)) {
                                cur.owners.insert(i, ch.addon.clone());
                            }
                        }
                        targets.insert(erp.clone(), cur);
                        shown = erp.clone();
                    }
                }
                _ => {}
            }
        }
        let (combined, grown) = match combine(&targets, &shown, &array, &names) {
            Some(c) => (c, true),
            None => (targets[&shown].clone(), false),
        };
        shelves.insert(
            a.clone(),
            Shelf {
                stock,
                targets,
                shown,
                combined,
                grown,
            },
        );
    }
    Library {
        lists,
        stock_lists,
        shelves,
        notes,
    }
}

impl Library {
    pub fn slice(&self, array: &str, key: &str) -> Option<usize> {
        let shelf = self.shelves.get(array)?;
        let i = self.lists.get(array)?.iter().position(|n| n == key)?;
        (i < shelf.shown().array.count).then_some(i)
    }

    /// The team's slice of each array of a place, with the key it goes by.
    pub fn slots(&self, kind: &str, ui_path: &str, label: &str, timing: &str) -> Vec<(String, usize)> {
        arrays_of(kind, ui_path)
            .into_iter()
            .filter_map(|(a, keyed)| {
                let key = if keyed == Keyed::Timing { timing } else { label };
                self.slice(&a, key).map(|s| (a, s))
            })
            .collect()
    }

    /// A team an addon added: in the profile's logo lists but none of the game's.
    /// Its logos are its own, not a change to the game's.
    pub fn is_added(&self, label: &str) -> bool {
        let has = |lists: &HashMap<String, Vec<String>>| lists.values().any(|l| l.iter().any(|n| n == label));
        has(&self.lists) && !has(&self.stock_lists)
    }

    pub fn size(&self, array: &str) -> Option<(u32, u32)> {
        let h = self.shelves.get(array)?.stock.header;
        Some((h.width, h.height))
    }

    /// The addon whose picture a slice shows, when not the game's.
    pub fn changed_by(&self, array: &str, slice: usize) -> Option<&str> {
        self.shelves
            .get(array)?
            .shown()
            .owners
            .get(&slice)
            .map(String::as_str)
    }

    /// A PNG of the slice as the game will show it, cached by its bytes.
    pub fn preview(&self, array: &str, slice: usize, cache: &Path) -> Option<PathBuf> {
        let shelf = self.shelves.get(array)?;
        let state = shelf.shown();
        let h = shelf.stock.header;
        if h.codec()? != Codec::Bc7 || slice >= state.array.count {
            return None;
        }
        let bytes = state.array.slice(slice);
        let level = Codec::Bc7.level_bytes(h.width, h.height).min(bytes.len());
        let path = cache.join("previews").join(format!(
            "{}-{slice}-{:016x}.png",
            array.replace('.', "_"),
            fnv1a64(bytes)
        ));
        if !path.is_file() {
            let rgba = texture::decode_pixels(Codec::Bc7, h.width, h.height, &bytes[..level]).ok()?;
            image::save_png(
                &Image {
                    width: h.width,
                    height: h.height,
                    rgba,
                },
                &path,
            )
            .ok()?;
        }
        Some(path)
    }
}

/// A picture in a slice's format: fitted to the array's size, with the mip chain the
/// array has.
pub fn encode_slice(picture: &Image, header: &texture::Header, slice_len: usize) -> Result<Vec<u8>, String> {
    if header.codec() != Some(Codec::Bc7) {
        return Err(format!("the array's format {:#x} isn't BC7", header.format));
    }
    if !header.width.is_multiple_of(4) || !header.height.is_multiple_of(4) {
        return Err(format!(
            "the array's slices are {}x{}",
            header.width, header.height
        ));
    }
    let mut level = image::fit(picture, header.width, header.height);
    let mut out = Vec::with_capacity(slice_len);
    for _ in 0..header.levels {
        let (w, h) = (
            level.width.max(4).div_ceil(4) * 4,
            level.height.max(4).div_ceil(4) * 4,
        );
        let padded = if (w, h) == (level.width, level.height) {
            level.clone()
        } else {
            image::fit(&level, w, h)
        };
        out.extend(bc7::encode(&padded.rgba, w, h));
        if out.len() >= slice_len {
            break;
        }
        level = image::half(&level);
    }
    if out.len() != slice_len {
        return Err(format!(
            "an encoded slice is {} bytes, the array's {slice_len}",
            out.len()
        ));
    }
    Ok(out)
}

/// One picture for one slice of one array.
pub struct Placement {
    pub array: String,
    pub slice: usize,
    pub picture: PathBuf,
    /// How much of the slot the picture fills, in percent.
    pub size: u32,
}

pub struct Baked {
    pub ops: Vec<Value>,
    pub lines: Vec<String>,
    pub warnings: Vec<String>,
}

fn write_archive(template: &Path, resources: Vec<Resource>, out: &Path) -> Result<(), String> {
    let file = ErpFile::open(template).map_err(|e| format!("{}: {e}", template.display()))?;
    let mut erp = Erp {
        header: file.erp.header,
        version: file.erp.version,
        resources,
    };
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(out, erp.to_bytes()).map_err(|e| format!("{}: {e}", out.display()))
}

/// Writes the archives the placements need under `root` and returns the ops that
/// put them in, with sources under `rel` (relative to the cache dir or the addon
/// folder the ops are read from). Where no addon grew an array, each archive gets
/// a merge-slices, so later addons' slices still merge. Where one did, such as
/// Classic Seasons, every archive whose copy differs gets the joined array whole
/// with these slices in it, its view too, since a merge would fall back to the
/// game's shorter array; with `join`, that happens even without placements, to
/// mend copies other addons left short.
pub fn bake(
    lib: &Library,
    placements: &[Placement],
    root: &Path,
    rel: &str,
    cached: bool,
    join: bool,
) -> Baked {
    let mut baked = Baked {
        ops: Vec::new(),
        lines: Vec::new(),
        warnings: Vec::new(),
    };
    let mut by_array: BTreeMap<&str, Vec<&Placement>> = BTreeMap::new();
    for p in placements {
        by_array.entry(&p.array).or_default().push(p);
    }
    if join {
        for (a, shelf) in &lib.shelves {
            if shelf.grown {
                by_array.entry(a.as_str()).or_default();
            }
        }
    }
    let mut pictures: HashMap<PathBuf, Option<Image>> = HashMap::new();
    let source = |path: &str| {
        if cached {
            json!({ "root": "cache", "path": path })
        } else {
            json!(path)
        }
    };
    for (array, list) in by_array {
        let Some(shelf) = lib.shelves.get(array) else {
            continue;
        };
        let stock = &shelf.stock;
        let name = resource(array);
        let mut slices: Vec<(usize, Vec<u8>)> = Vec::new();
        for p in list {
            let picture =
                pictures
                    .entry(p.picture.clone())
                    .or_insert_with(|| match image::load_png(&p.picture) {
                        Ok(img) => Some(img),
                        Err(e) => {
                            baked.warnings.push(e);
                            None
                        }
                    });
            let Some(picture) = picture else { continue };
            let placed = image::place(picture, stock.header.width, stock.header.height, p.size, None);
            match encode_slice(&placed, &stock.header, stock.array.slice_len()) {
                Ok(data) => slices.push((p.slice, data)),
                Err(e) => baked.warnings.push(format!("{array}: {e}")),
            }
        }
        let file_stem = array.replace('.', "_");
        if shelf.grown {
            let combined = &shelf.combined;
            let mut whole = combined.array.clone();
            let n_slice = whole.slice_len();
            for (i, data) in &slices {
                if *i < whole.count && data.len() == n_slice {
                    whole.data[i * n_slice..(i + 1) * n_slice].copy_from_slice(data);
                }
            }
            let stale: Vec<&String> = shelf
                .targets
                .iter()
                .filter(|(_, t)| t.array.count != whole.count || t.array.data != whole.data)
                .map(|(erp, _)| erp)
                .collect();
            if stale.is_empty() {
                continue;
            }
            let mut res = combined.resource.clone();
            if let Err(e) = res.set_fragment(1, &whole.data) {
                baked.warnings.push(format!("{array}: {e}"));
                continue;
            }
            let mut resources = vec![res];
            resources.extend(combined.descriptor.clone());
            let path = format!("{rel}/{file_stem}.erp");
            if let Err(e) = write_archive(&stock.template, resources, &root.join(&path)) {
                baked.warnings.push(e);
                continue;
            }
            for erp in &stale {
                baked.ops.push(json!({ "op": "erp.put-resource", "erp": erp, "resource": name, "source": source(&path) }));
                if combined.descriptor.is_some() {
                    baked.ops.push(json!({ "op": "erp.put-resource", "erp": erp, "resource": descriptor(array), "source": source(&path) }));
                }
            }
            baked.lines.push(if slices.is_empty() {
                format!(
                    "{array}: {} copies made whole again ({} teams)",
                    stale.len(),
                    whole.count
                )
            } else {
                format!(
                    "{} logo slice(s) in {array}, into {} archive(s)",
                    slices.len(),
                    stale.len()
                )
            });
            continue;
        }
        if slices.is_empty() {
            continue;
        }
        let mut merged = false;
        for (n, (erp, state)) in shelf.targets.iter().enumerate() {
            let grown = !state.array.same_layout(&stock.array);
            let (base_res, base) = if grown {
                (&state.resource, &state.array)
            } else {
                (&stock.resource, &stock.array)
            };
            let mut array_data = base.clone();
            let n_slice = array_data.slice_len();
            for (i, data) in &slices {
                if *i < array_data.count && data.len() == n_slice {
                    array_data.data[i * n_slice..(i + 1) * n_slice].copy_from_slice(data);
                }
            }
            let mut res = base_res.clone();
            if let Err(e) = res.set_fragment(1, &array_data.data) {
                baked.warnings.push(format!("{array}: {e}"));
                continue;
            }
            if grown {
                let path = format!("{rel}/{file_stem}.{n}.erp");
                if let Err(e) = write_archive(&stock.template, vec![res], &root.join(&path)) {
                    baked.warnings.push(e);
                    continue;
                }
                baked.ops.push(json!({ "op": "erp.put-resource", "erp": erp, "resource": name, "source": source(&path) }));
            } else {
                let path = format!("{rel}/{file_stem}.erp");
                if !merged {
                    if let Err(e) = write_archive(&stock.template, vec![res], &root.join(&path)) {
                        baked.warnings.push(e);
                        continue;
                    }
                    merged = true;
                }
                baked.ops.push(json!({ "op": "erp.merge-slices", "erp": erp, "resource": name, "source": source(&path) }));
            }
        }
        baked.lines.push(format!(
            "{} logo slice(s) in {array}, into {} archive(s)",
            slices.len(),
            shelf.targets.len()
        ));
    }
    baked
}
