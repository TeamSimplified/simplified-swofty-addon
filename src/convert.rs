use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};
use swofty_addon::asset::{AssetOp, FileOp, Op, Origin, Root, Source, TrackOp};
use swofty_addon::launcher::GameFiles;
use swofty_addon::manifest::{AddonManifest, Category, Kind, MANIFEST_FILE};
use swofty_addon::ops::{DbOp, ErpOp};
use swofty_formats::erp::{Erp, ErpFile};

use crate::diff::Diff;
use crate::library::{FILES, LIBRARY, Library, Package};
use crate::package::Class;
use crate::{ID, Result, err, io_err, merge, texarray, track};

pub const NAMESPACE: &str = "mod";
pub const LAUNCHER: &str = ">=1.3.0";
pub const REPORT: &str = "import-report.txt";
pub const DATABASE_OPS: &str = "ops/database.json";
pub const ERP_OPS: &str = "ops/erp.json";
pub const TEXT_OPS: &str = "ops/text.json";
pub const FILE_OPS: &str = "ops/files.json";
pub const TRACK_OPS: &str = "ops/track.json";
pub const ERP_DIR: &str = "erp";
pub const FILES_DIR: &str = "files";
pub const LOGOS: &str = "logos/ui_textures.erp";
const MIPMAPS: &str = ".mipmaps";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Converted {
    pub id: String,
    pub name: String,
    pub category: Category,
    pub kinds: Vec<Kind>,
    pub report: Vec<String>,
}

pub fn addon_id(p: &Package) -> String {
    let parts: Vec<String> = p.id.split('.').map(slug).filter(|s| !s.is_empty()).collect();
    match parts.is_empty() {
        true => format!("{NAMESPACE}.mod"),
        false => format!("{NAMESPACE}.{}", parts.join(".")),
    }
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

pub fn version(v: Option<&str>) -> String {
    let v = v.map(str::trim).unwrap_or_default();
    let bare = v.strip_prefix(['v', 'V']).unwrap_or(v);
    if semver_ok(bare) {
        return bare.to_owned();
    }
    let numbers: Vec<u64> = bare
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect();
    let [a, b, c] = std::array::from_fn(|i| numbers.get(i).copied().unwrap_or(0));
    match numbers.as_slice() {
        [] => "1.0.0".into(),
        [_, _, _, rest @ ..] if !rest.is_empty() => {
            let rest: Vec<String> = rest.iter().map(u64::to_string).collect();
            format!("{a}.{b}.{c}+{}", rest.join("."))
        }
        _ => format!("{a}.{b}.{c}"),
    }
}

fn semver_ok(v: &str) -> bool {
    semver::Version::parse(v).is_ok()
}

#[derive(Default)]
struct Writer {
    dir: PathBuf,
    db: Vec<DbOp>,
    erp: Vec<ErpOp>,
    text: Vec<AssetOp>,
    files: Vec<AssetOp>,
    track: Vec<AssetOp>,
    report: Vec<String>,
    contents: Contents,
    resources: usize,
}

#[derive(Serialize)]
struct OpFile<'a, T> {
    schema_version: u32,
    ops: &'a [T],
}

#[derive(Default)]
struct Tally {
    filled: usize,
    put: usize,
    sources: usize,
    mips: usize,
    logos: usize,
}

#[derive(Default)]
struct Contents {
    records: usize,
    text: usize,
    team_text: usize,
    areas: BTreeMap<Area, usize>,
    vehicle_art: usize,
    track: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Area {
    Track,
    Vehicle,
    Ui,
    Audio,
    Other,
}

pub fn convert(
    library: &Library,
    p: &Package,
    game: Option<&dyn GameFiles>,
    out: &Path,
    progress: &mut dyn FnMut(f32, &str),
) -> Result<Converted> {
    if std::fs::read_dir(out).is_ok_and(|mut d| d.next().is_some()) {
        return Err(err(format!("{} is not empty", out.display())));
    }
    std::fs::create_dir_all(out).map_err(io_err(out))?;
    let result = write_addon(library, p, game, out, progress);
    if result.is_err() {
        std::fs::remove_dir_all(out).ok();
    }
    result
}

fn write_addon(
    library: &Library,
    p: &Package,
    game: Option<&dyn GameFiles>,
    out: &Path,
    progress: &mut dyn FnMut(f32, &str),
) -> Result<Converted> {
    let mut w = Writer {
        dir: out.to_path_buf(),
        ..Writer::default()
    };
    progress(0.0, &p.name);
    match (&p.track, game) {
        (Some(code), _) => w.track(library, p, code)?,
        (None, Some(game)) => w.package(library, p, game, progress)?,
        (None, None) => {
            return Err(err(format!(
                "{}: converting a mod needs the game's own files to compare it with",
                p.name
            )));
        }
    }
    let v = version(p.version.as_deref());
    if p.version.as_deref().map(str::trim) != Some(v.as_str()) {
        let from = p.version.as_deref().unwrap_or("none");
        w.report.push(format!("version {from} → {v}"));
    }
    for path in &p.skipped {
        w.report.push(format!(
            "skipped {path}: programs and EA AntiCheat files are never copied"
        ));
    }
    let id = addon_id(p);
    let kinds = w.kinds();
    let category = w.contents.category(&p.name);
    if w.db.is_empty() && w.erp.is_empty() && w.text.is_empty() && w.files.is_empty() && w.track.is_empty() {
        w.report
            .push("warning: the mod changes nothing in this game build".into());
    }
    let operations = w.write_ops()?;
    let manifest = manifest(p, game, &id, &kinds, category, operations, &w)?;
    let json = serde_json::to_vec_pretty(&manifest).expect("serialisable");
    let path = out.join(MANIFEST_FILE);
    std::fs::write(&path, json).map_err(io_err(&path))?;
    w.report.insert(
        0,
        format!("{} → {id} ({}), category {}", p.source, p.name, category.label()),
    );
    let path = out.join(REPORT);
    std::fs::write(&path, w.report.join("\n") + "\n").map_err(io_err(&path))?;
    progress(1.0, &p.name);
    Ok(Converted {
        id,
        name: p.name.clone(),
        category,
        kinds,
        report: w.report,
    })
}

fn manifest(
    p: &Package,
    game: Option<&dyn GameFiles>,
    id: &str,
    kinds: &[Kind],
    category: Category,
    operations: Value,
    w: &Writer,
) -> Result<AddonManifest> {
    let mut compat = json!({ "game": swofty_addon::GAME_ID, "launcher": LAUNCHER });
    let build = game.and_then(|g| g.build_key().split('-').next()?.parse::<u64>().ok());
    if let Some(build) = build {
        compat["game_builds"] = json!([format!(">={build}")]);
    }
    let v = version(p.version.as_deref());
    let doc = json!({
        "schema_version": swofty_addon::manifest::SCHEMA_VERSION,
        "id": id,
        "name": p.name,
        "version": v,
        "author": p.author.clone().unwrap_or_else(|| "Unknown".into()),
        "description": w.description(&p.source),
        "kind": kinds,
        "compat": compat,
        "operations": operations,
        "category": category,
        "origin": {
            "tool": ID,
            "tool_version": env!("CARGO_PKG_VERSION"),
            "file": p.source,
            "sha256": p.sha256,
        },
    });
    let m: AddonManifest =
        serde_json::from_value(doc).map_err(|e| err(format!("{}: the generated manifest: {e}", p.name)))?;
    m.validate()?;
    Ok(m)
}

impl Writer {
    fn kinds(&self) -> Vec<Kind> {
        let track = !self.track.is_empty();
        [
            (Kind::Database, track || !self.db.is_empty()),
            (Kind::Erp, track || !self.erp.is_empty()),
            (Kind::Ui, !self.text.is_empty()),
            (Kind::Files, !self.files.is_empty()),
        ]
        .into_iter()
        .filter_map(|(k, on)| on.then_some(k))
        .collect()
    }

    fn description(&self, source: &str) -> String {
        let mut parts = Vec::new();
        let mut count = |n: usize, one: &str, many: &str| {
            if n > 0 {
                parts.push(format!("{n} {}", if n == 1 { one } else { many }));
            }
        };
        count(self.db.len(), "database record", "database records");
        count(self.text.len(), "text entry", "text entries");
        count(self.resources, "archive resource", "archive resources");
        count(self.files.len(), "file", "files");
        if !self.track.is_empty() {
            parts.push("a track".into());
        }
        let what = match parts.is_empty() {
            true => String::new(),
            false => format!(": {}", parts.join(", ")),
        };
        format!("Converted from {source} by the Mod Importer{what}.")
    }

    fn track(&mut self, library: &Library, p: &Package, code: &str) -> Result<()> {
        let from = library.dir(&p.id).join(track::TRACK);
        if !from.join(track::MANIFEST).is_file() {
            return Err(err(format!("{}: no track package in the library", p.name)));
        }
        let n = copy_tree(&from, &self.dir.join(track::TRACK))?;
        self.track.push(AssetOp::Track(TrackOp::Add {
            source: Source::addon(track::TRACK),
        }));
        self.contents.track = true;
        self.report.push(format!(
            "track {code}: the track package ({n} file(s)) → track.add from track/"
        ));
        Ok(())
    }

    fn package(
        &mut self,
        library: &Library,
        p: &Package,
        game: &dyn GameFiles,
        progress: &mut dyn FnMut(f32, &str),
    ) -> Result<()> {
        let diff = library.diff(p, game)?;
        progress(0.3, &p.name);
        let records = library.records(&p.id, &diff.build_key)?;
        let mut merged = merge::ops(&p.id, &diff, &records);
        let origin = Origin {
            addon: ID.into(),
            item: Some(p.id.clone()),
        };
        let wrap = |op| Op {
            op,
            origin: origin.clone(),
        };
        let mut ops: Vec<Op> = std::mem::take(&mut merged.ops).into_iter().map(wrap).collect();
        let defaults = crate::only_defaults(
            std::mem::take(&mut merged.defaults)
                .into_iter()
                .map(wrap)
                .collect(),
            &ops,
        );
        let mut tally = Tally {
            filled: defaults.len(),
            ..Tally::default()
        };
        ops.splice(0..0, defaults);
        let ops = crate::without_repeats(ops);
        let lib_files = library.dir(&p.id).join(FILES);
        let prefix = format!("{LIBRARY}/{}/{FILES}/", p.id);
        let rel = |s: &Source| match s.root {
            Root::Data => s.path.strip_prefix(&prefix).map(str::to_owned),
            _ => None,
        };
        let mut overlays: Vec<(String, Vec<(PathBuf, String)>)> = Vec::new();
        for op in ops {
            match op.op {
                AssetOp::Db(op) => self.db.push(op),
                AssetOp::Lang(op) => self.text.push(AssetOp::Lang(op)),
                AssetOp::Erp(ErpOp::PutResource {
                    erp,
                    resource,
                    source,
                    rename: None,
                }) => {
                    let r = rel(&source).ok_or_else(|| err(format!("{resource}: no source in the mod")))?;
                    let at = match overlays.iter().position(|(target, _)| *target == erp) {
                        Some(at) => at,
                        None => {
                            let n = overlays.len();
                            self.erp.push(ErpOp::Overlay {
                                erps: vec![erp.clone()],
                                source: Source::addon(overlay_name(n)),
                            });
                            overlays.push((erp, Vec::new()));
                            n
                        }
                    };
                    overlays[at].1.push((lib_files.join(r), resource));
                }
                AssetOp::Erp(op @ ErpOp::RemoveResource { .. }) => {
                    self.resources += 1;
                    self.erp.push(op);
                }
                AssetOp::File(FileOp::Put { source, target }) => {
                    let r = rel(&source).ok_or_else(|| err(format!("{target}: no source in the mod")))?;
                    let dest = format!("{FILES_DIR}/{r}");
                    copy_file(&lib_files.join(&r), &self.dir.join(&dest))?;
                    let source = Source::addon(dest);
                    match target.to_ascii_lowercase().ends_with(MIPMAPS) {
                        true => {
                            tally.mips += 1;
                            self.erp.push(ErpOp::AddMipmaps { source, target });
                        }
                        false => self.files.push(AssetOp::File(FileOp::Put { source, target })),
                    }
                }
                other => {
                    return Err(err(format!(
                        "{}: the merge gave an op the converter does not know: {other:?}",
                        p.name
                    )));
                }
            }
        }
        progress(0.5, &p.name);
        let mut sources = BTreeSet::new();
        for (n, (_, resources)) in overlays.iter().enumerate() {
            sources.extend(resources.iter().map(|(path, _)| path.clone()));
            tally.put += write_overlay(resources, &self.dir.join(overlay_name(n)))?;
        }
        tally.sources = sources.len();
        progress(0.8, &p.name);
        tally.logos = self.logos(library, p, &diff, game)?;
        self.resources += tally.put + tally.logos;
        self.contents.read(&diff);
        self.report_package(p, &diff, &merged, &tally);
        Ok(())
    }

    fn logos(&mut self, library: &Library, p: &Package, diff: &Diff, game: &dyn GameFiles) -> Result<usize> {
        let carried: Vec<texarray::Carried> = diff
            .archives
            .iter()
            .filter(|a| texarray::carrier(&a.path) && !a.added.is_empty())
            .map(|a| texarray::Carried {
                item: p.name.clone(),
                erp: library.dir(&p.id).join(FILES).join(&a.path),
                uris: a
                    .added
                    .iter()
                    .filter(|uri| texarray::merged_by_slice(uri) && !merge::studio_only(uri))
                    .cloned()
                    .collect(),
            })
            .filter(|c| !c.uris.is_empty())
            .collect();
        if carried.is_empty() {
            return Ok(0);
        }
        let built = texarray::build(game, &carried, &self.dir.join(LOGOS))?;
        let n = built.ops.len();
        for op in built.ops {
            if let AssetOp::Erp(ErpOp::PutResource {
                erp,
                resource,
                rename,
                ..
            }) = op
            {
                self.erp.push(ErpOp::PutResource {
                    erp,
                    resource,
                    source: Source::addon(LOGOS),
                    rename,
                });
            }
        }
        self.report.extend(built.lines);
        self.report
            .extend(built.warnings.into_iter().map(|w| format!("warning: {w}")));
        if n > 0 {
            self.report.push(format!(
                "warning: {n} team logo texture array resource(s) ship whole from {LOGOS}, merged slice by slice \
                 with this game build's arrays; another addon that changes the same array later in the load \
                 order replaces it whole instead of merging slices"
            ));
        }
        Ok(n)
    }

    fn report_package(&mut self, p: &Package, diff: &Diff, merged: &merge::Merged, t: &Tally) {
        let r = &diff.records;
        if !r.is_empty() {
            self.report.push(format!(
                "database: {} changed, {} added, {} removed record(s) → {} put-record and {} remove-record op(s) in {DATABASE_OPS}",
                r.changed.len(),
                r.added.len(),
                r.removed.len(),
                r.changed.len() + r.added.len(),
                r.removed.len()
            ));
        }
        if diff.dropped > 0 {
            self.report.push(format!(
                "warning: {} removal(s) left out: the mod was made for {}, not this game version",
                diff.dropped,
                p.game_version.as_deref().unwrap_or("an unknown version")
            ));
        }
        let languages = diff.text.iter().filter(|t| t.entries().next().is_some()).count();
        if merged.strings > 0 {
            self.report.push(format!(
                "text: {} entr(ies) in {languages} language(s) → lang.set in {TEXT_OPS}",
                merged.strings
            ));
        }
        let borrowed: BTreeMap<&str, Vec<&str>> = diff.text.iter().fold(BTreeMap::new(), |mut m, t| {
            if let Some(from) = &t.from {
                m.entry(from.as_str()).or_default().push(t.language.as_str());
            }
            m
        });
        for (from, langs) in borrowed {
            self.report.push(format!(
                "text: {} take the mod's {from} text, as the mod has no file for them",
                langs.join(", ")
            ));
        }
        if t.filled > 0 {
            self.report.push(format!(
                "warning: {} text entr(ies) the mod fills in with the game's own text → lang.set before \
                 its other text; loaded after another addon that renames the same key, they undo that rename",
                t.filled
            ));
        }
        if t.put > 0 {
            self.report.push(format!(
                "archives: {} resource(s) → erp.overlay into {} in {ERP_OPS}, from {} that holds only \
                 those resources, copied out of {} of the mod's archive(s)",
                t.put,
                merge::BOOT_ERP,
                overlay_name(0),
                t.sources
            ));
        }
        if merged.carried > 0 {
            self.report.push(format!(
                "archives: {} of them carried in Simplified Studio shader archives",
                merged.carried
            ));
        }
        if merged.removals > 0 {
            self.report.push(format!(
                "archives: {} resource(s) removed → remove-resource on their own archives",
                merged.removals
            ));
        }
        let studio: usize = diff
            .archives
            .iter()
            .map(|a| {
                a.changed
                    .iter()
                    .chain(&a.added)
                    .filter(|uri| {
                        uri.starts_with(merge::MARKER) || merge::carrier(&a.path) && merge::studio_only(uri)
                    })
                    .count()
            })
            .sum();
        if studio > 0 {
            self.report.push(format!(
                "archives: {studio} Simplified Studio marker or vehicle shadow resource(s) left out"
            ));
        }
        if t.logos > 0 {
            self.report.push(format!(
                "archives: {} team logo texture array resource(s) → put-resource from {LOGOS}",
                t.logos
            ));
        }
        let count = |class| diff.files.iter().filter(|f| f.class == class).count();
        let (replaced, added) = (count(Class::Replacement), count(Class::Addition));
        if replaced + added > 0 {
            self.report.push(format!(
                "files: {replaced} replaced and {added} added → {} file.put op(s) in {FILE_OPS} and {} \
                 add-mipmaps op(s) in {ERP_OPS}, copied under {FILES_DIR}/",
                replaced + added - t.mips,
                t.mips
            ));
        }
    }

    fn write_ops(&self) -> Result<Value> {
        let mut operations = json!({});
        if !self.db.is_empty() {
            self.write(DATABASE_OPS, &self.db)?;
            operations["database"] = json!([DATABASE_OPS]);
        }
        if !self.erp.is_empty() {
            self.write(ERP_OPS, &self.erp)?;
            operations["erp"] = json!([ERP_OPS]);
        }
        let mut assets = Vec::new();
        for (path, ops) in [
            (TRACK_OPS, &self.track),
            (TEXT_OPS, &self.text),
            (FILE_OPS, &self.files),
        ] {
            if !ops.is_empty() {
                self.write(path, ops)?;
                assets.push(path);
            }
        }
        if !assets.is_empty() {
            operations["assets"] = json!(assets);
        }
        Ok(operations)
    }

    fn write<T: Serialize>(&self, rel: &str, ops: &[T]) -> Result<()> {
        let path = self.dir.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        let doc = OpFile {
            schema_version: 1,
            ops,
        };
        std::fs::write(&path, serde_json::to_vec_pretty(&doc).expect("serialisable")).map_err(io_err(&path))
    }
}

impl Contents {
    fn read(&mut self, diff: &Diff) {
        self.records = diff.records.len();
        for t in &diff.text {
            self.text += t.len();
            self.team_text += t
                .entries()
                .filter(|(k, _)| ["team", "driver"].iter().any(|w| k.contains(w)))
                .count();
        }
        for a in &diff.archives {
            for uri in a.changed.iter().chain(&a.added).chain(&a.removed) {
                if uri.starts_with(merge::MARKER) || merge::studio_only(uri) {
                    continue;
                }
                self.count(uri);
            }
        }
        for f in &diff.files {
            self.count(&f.path);
        }
    }

    fn count(&mut self, path: &str) {
        let lower = path.to_ascii_lowercase();
        let area = area(&lower);
        if area == Area::Vehicle && art(&lower) {
            self.vehicle_art += 1;
        }
        *self.areas.entry(area).or_default() += 1;
    }

    fn category(&self, name: &str) -> Category {
        let n = |a| self.areas.get(&a).copied().unwrap_or(0);
        let (track, vehicle, ui, audio, other) = (
            n(Area::Track),
            n(Area::Vehicle),
            n(Area::Ui),
            n(Area::Audio),
            n(Area::Other),
        );
        let season = name.to_lowercase().contains("season");
        if self.track || track > 0 && track >= vehicle {
            Category::Tracks
        } else if season && self.records + vehicle > 0 || vehicle > 0 && self.records >= 1000 {
            Category::Seasons
        } else if vehicle > 0 && self.vehicle_art == vehicle {
            Category::Liveries
        } else if vehicle > 0 {
            Category::Cars
        } else if audio > 0 && audio >= ui + other {
            Category::Audio
        } else if self.records > 0 && self.team_text > 0 {
            Category::DriversTeams
        } else if self.records > 0 {
            Category::Database
        } else if self.text > 0 || ui > 0 {
            Category::Ui
        } else if audio > 0 {
            Category::Audio
        } else {
            Category::Other
        }
    }
}

fn area(lower: &str) -> Area {
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    if lower.contains("environment_package/tracks/") {
        Area::Track
    } else if lower.contains("vehicle_package") {
        Area::Vehicle
    } else if lower.contains("ui_package") || texarray::merged_by_slice(lower) {
        Area::Ui
    } else if lower.starts_with("audio/") || ["bnk", "wem", "pck", "fsb", "acb", "awb"].contains(&ext) {
        Area::Audio
    } else {
        Area::Other
    }
}

fn art(lower: &str) -> bool {
    [
        ".image",
        ".tif",
        ".dds",
        ".png",
        ".material",
        ".texarray",
        MIPMAPS,
    ]
    .iter()
    .any(|e| lower.ends_with(e))
        || ["livery", "/textures/", "decal"]
            .iter()
            .any(|w| lower.contains(w))
}

fn overlay_name(n: usize) -> String {
    match n {
        0 => format!("{ERP_DIR}/overlay.erp"),
        n => format!("{ERP_DIR}/overlay-{}.erp", n + 1),
    }
}

// Copies each resource's fragments straight from the mod's archives, so a large mod
// is never held in memory; a URI given twice keeps its last copy, as later puts win.
fn write_overlay(resources: &[(PathBuf, String)], to: &Path) -> Result<usize> {
    let mut seen = BTreeSet::new();
    let mut wanted: Vec<&(PathBuf, String)> = resources
        .iter()
        .rev()
        .filter(|(_, uri)| seen.insert(uri))
        .collect();
    wanted.reverse();
    let mut files: BTreeMap<&Path, ErpFile> = BTreeMap::new();
    for (path, _) in &wanted {
        if !files.contains_key(path.as_path()) {
            let file = ErpFile::open(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
            files.insert(path, file);
        }
    }
    let first = &files[wanted[0].0.as_path()];
    let mut erp = Erp {
        header: first.erp.header,
        version: first.erp.version,
        resources: Vec::with_capacity(wanted.len()),
    };
    let mut spans = Vec::new();
    for (path, uri) in &wanted {
        let file = &files[path.as_path()];
        if file.erp.version != erp.version {
            return Err(err(format!(
                "{}: archive version {} differs from {} in the same mod",
                path.display(),
                file.erp.version,
                erp.version
            )));
        }
        let r = file
            .erp
            .find(uri)
            .map_err(|e| err(format!("{}: {e}", path.display())))?;
        spans.extend(
            r.fragments
                .iter()
                .map(|f| (path.as_path(), file.data_off + f.offset, f.packed)),
        );
        erp.resources.push(r.clone());
    }
    let head = erp.to_bytes();
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    let tmp = to.with_extension("erp.tmp");
    let mut out =
        std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp).map_err(io_err(&tmp))?);
    out.write_all(&head).map_err(io_err(&tmp))?;
    let mut open: Option<(&Path, std::fs::File)> = None;
    for (path, at, len) in spans {
        if open.as_ref().is_none_or(|(p, _)| *p != path) {
            open = Some((path, std::fs::File::open(path).map_err(io_err(path))?));
        }
        let (_, src) = open.as_mut().expect("opened above");
        src.seek(SeekFrom::Start(at)).map_err(io_err(path))?;
        let copied = std::io::copy(&mut Read::take(&mut *src, len), &mut out).map_err(io_err(path))?;
        if copied != len {
            return Err(err(format!("{}: a fragment ends early", path.display())));
        }
    }
    out.flush().map_err(io_err(&tmp))?;
    drop(out);
    std::fs::rename(&tmp, to).map_err(io_err(to))?;
    Ok(erp.resources.len())
}

fn copy_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    std::fs::copy(from, to).map_err(io_err(from))?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<usize> {
    let mut n = 0;
    for path in crate::package::files(from)? {
        let rel = path.strip_prefix(from).expect("under the folder");
        copy_file(&path, &to.join(rel))?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests;
