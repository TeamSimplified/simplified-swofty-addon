use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};
use swofty_addon::asset::{AssetOp, FileOp, LangOp, Op, Origin, Root, Source, TrackOp};
use swofty_addon::launcher::GameFiles;
use swofty_addon::manifest::{AddonManifest, Category, Kind, MANIFEST_FILE};
use swofty_addon::ops::{DbOp, ErpOp};
use swofty_formats::erp::{Erp, ErpFile};

use crate::diff::Diff;
use crate::library::{FILES, LIBRARY, Library, Package};
use crate::log::Log;
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
const MAX_ID: usize = 64;
const SEASON_RECORDS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Converted {
    pub id: String,
    pub name: String,
    pub category: Category,
    pub kinds: Vec<Kind>,
    pub summary: String,
    pub report: Vec<String>,
}

pub fn addon_id(p: &Package) -> String {
    let parts: Vec<String> = p.id.split('.').map(slug).filter(|s| !s.is_empty()).collect();
    match parts.is_empty() {
        true => format!("{NAMESPACE}.mod"),
        false => format!(
            "{NAMESPACE}.{}",
            crate::library::capped(&parts.join("."), &p.id, MAX_ID - NAMESPACE.len() - 1)
        ),
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
    written: BTreeSet<(String, String)>,
    mips: usize,
    log: Log,
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
    views: usize,
}

#[derive(Default)]
struct Contents {
    records: usize,
    text: usize,
    team_text: usize,
    areas: BTreeMap<Area, usize>,
    vehicle_art: usize,
    tyres: usize,
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
    convert_to(library, p, &addon_id(p), game, out, progress)
}

pub fn convert_to(
    library: &Library,
    p: &Package,
    id: &str,
    game: Option<&dyn GameFiles>,
    out: &Path,
    progress: &mut dyn FnMut(f32, &str),
) -> Result<Converted> {
    let mut log = Log::default();
    log_source(&mut log, p, game, id);
    let mut ours = false;
    let result = (|| {
        if std::fs::read_dir(out).is_ok_and(|mut d| d.next().is_some()) {
            return Err(err(format!("{} is not empty", out.display())));
        }
        std::fs::create_dir_all(out).map_err(io_err(out))?;
        ours = true;
        write_addon(library, p, id, game, out, progress, &mut log)
    })();
    log.section("result");
    match &result {
        Ok(c) => {
            log.line(format!("addon: {}", c.id));
            log.line(format!("name: {}", c.name));
            log.line(format!("category: {}", c.category.label()));
            let kinds: Vec<String> = c.kinds.iter().map(|k| format!("{k:?}").to_lowercase()).collect();
            log.line(format!("kinds: {}", kinds.join(", ")));
            log.line(format!("summary: {}", c.summary));
            for line in c.report.iter().filter(|l| l.starts_with("warning:")) {
                log.line(line.clone());
            }
        }
        Err(e) => {
            log.line(format!("error: converting {} ({}) into {id}: {e}", p.name, p.id));
            if ours {
                std::fs::remove_dir_all(out).ok();
            }
        }
    }
    log.write(library.data_dir(), id).ok();
    result
}

fn log_source(log: &mut Log, p: &Package, game: Option<&dyn GameFiles>, id: &str) {
    log.section("source");
    log.line(format!("file: {}", p.source));
    log.line(format!("size: {} bytes in {} file(s)", p.size(), p.files.len()));
    log.line(format!("sha256: {}", p.sha256.as_deref().unwrap_or("none")));
    log.line(format!("library id: {}", p.id));
    log.line(format!("addon id: {id}"));
    log.line(format!("name: {}", p.name));
    log.line(format!("author: {}", p.author.as_deref().unwrap_or("none")));
    log.line(format!("version: {}", p.version.as_deref().unwrap_or("none")));
    log.line(format!(
        "game_version: {}",
        p.game_version.as_deref().unwrap_or("none")
    ));
    log.line(format!(
        "studio_version: {}",
        p.studio_version.as_deref().unwrap_or("none")
    ));
    if let Some(part) = &p.part {
        log.line(format!("part: {} of {}", part.name, part.of));
    }
    if let Some(code) = &p.track {
        log.line(format!("track: {code}"));
    }
    log.line(format!("importer: {ID} {}", env!("CARGO_PKG_VERSION")));
    log.section("game");
    match game {
        Some(g) => {
            log.line(format!("build key: {}", g.build_key()));
            log.line(format!(
                "exe version: {}",
                g.exe_version().as_deref().unwrap_or("unknown")
            ));
        }
        None => log.line("game: not found"),
    }
    log.section("files");
    let mut classes: BTreeMap<Class, (usize, u64)> = BTreeMap::new();
    for f in &p.files {
        let e = classes.entry(f.class).or_default();
        e.0 += 1;
        e.1 += f.size;
    }
    for (class, (n, size)) in classes {
        log.line(format!("class {}: {n} file(s), {size} bytes", class_name(class)));
    }
    for path in &p.skipped {
        log.line(format!(
            "skipped file {path}: programs and EA AntiCheat files are never copied"
        ));
    }
}

fn class_name(class: Class) -> &'static str {
    match class {
        Class::Database => "database",
        Class::Text => "text",
        Class::Archive => "archive",
        Class::Replacement => "replacement",
        Class::Addition => "addition",
    }
}

fn write_addon(
    library: &Library,
    p: &Package,
    id: &str,
    game: Option<&dyn GameFiles>,
    out: &Path,
    progress: &mut dyn FnMut(f32, &str),
    log: &mut Log,
) -> Result<Converted> {
    let mut w = Writer {
        dir: out.to_path_buf(),
        log: std::mem::take(log),
        ..Writer::default()
    };
    let result = (|| -> Result<Converted> {
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
        w.log_ops();
        let kinds = w.kinds();
        let category = w.contents.category(&p.name);
        if w.db.is_empty()
            && w.erp.is_empty()
            && w.text.is_empty()
            && w.files.is_empty()
            && w.track.is_empty()
        {
            w.report
                .push("warning: the mod changes nothing in this game build".into());
        }
        let operations = w.write_ops()?;
        w.log.time("write");
        let manifest = manifest(p, game, id, &kinds, category, operations, &w)?;
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
            id: id.to_owned(),
            name: p.name.clone(),
            category,
            kinds,
            summary: w.counts().join(", "),
            report: std::mem::take(&mut w.report),
        })
    })();
    *log = std::mem::take(&mut w.log);
    result
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
            (Kind::Text, !self.text.is_empty()),
            (Kind::Files, !self.files.is_empty()),
        ]
        .into_iter()
        .filter_map(|(k, on)| on.then_some(k))
        .collect()
    }

    fn description(&self, source: &str) -> String {
        let parts = self.counts();
        let what = match parts.is_empty() {
            true => String::new(),
            false => format!(": {}", parts.join(", ")),
        };
        format!("Converted from {source} by the Mod Importer{what}.")
    }

    fn counts(&self) -> Vec<String> {
        let mut parts = Vec::new();
        let mut count = |n: usize, one: &str, many: &str| {
            if n > 0 {
                parts.push(format!("{n} {}", if n == 1 { one } else { many }));
            }
        };
        count(self.db.len(), "database record", "database records");
        count(self.text.len(), "text entry", "text entries");
        let mut assets: BTreeMap<Area, usize> = BTreeMap::new();
        for (_, uri) in &self.written {
            *assets.entry(area(&uri.to_ascii_lowercase())).or_default() += 1;
        }
        for (a, n) in assets {
            let (one, many) = a.assets();
            count(n, one, many);
        }
        count(self.files.len() + self.mips, "file", "files");
        if !self.track.is_empty() {
            parts.push("a track".into());
        }
        parts
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
        self.log.time("diff");
        self.log_diff(p, &diff);
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
        let defaults: Vec<Op> = crate::only_defaults(
            std::mem::take(&mut merged.defaults)
                .into_iter()
                .map(wrap)
                .collect(),
            &ops,
        )
        .into_iter()
        .map(|op| Op {
            op: fill_in(op.op),
            origin: op.origin,
        })
        .collect();
        let mut tally = Tally {
            filled: defaults.len(),
            ..Tally::default()
        };
        ops.splice(0..0, defaults);
        let before = ops.len();
        let ops = crate::without_repeats(ops);
        if ops.len() < before {
            self.log.line(format!(
                "left out {} op(s) that repeat the value an earlier op of the mod writes",
                before - ops.len()
            ));
        }
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
                AssetOp::Erp(ErpOp::RemoveResource { erp, resource }) => {
                    self.written.insert((erp.clone(), resource.clone()));
                    self.erp.push(ErpOp::RemoveResource { erp, resource });
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
        for (n, (erp, resources)) in overlays.iter().enumerate() {
            sources.extend(resources.iter().map(|(path, _)| path.clone()));
            self.written
                .extend(resources.iter().map(|(_, uri)| (erp.clone(), uri.clone())));
            tally.put += write_overlay(resources, &self.dir.join(overlay_name(n)))?;
        }
        tally.sources = sources.len();
        self.log.time("overlay archives");
        progress(0.8, &p.name);
        (tally.logos, tally.views) = self.logos(library, p, &diff)?;
        self.log.time("logo arrays");
        self.mips = tally.mips;
        self.contents.read(&diff);
        self.report_package(p, &diff, &merged, &tally);
        Ok(())
    }

    // Team logo arrays ride in the mod's ui_texture shader archives. Each goes into
    // the boot archive with merge-slices, which writes only the slices that differ
    // from the game's over what earlier addons wrote; their views go in whole.
    fn logos(&mut self, library: &Library, p: &Package, diff: &Diff) -> Result<(usize, usize)> {
        let mut resources = Vec::new();
        for a in diff.archives.iter().filter(|a| texarray::carrier(&a.path)) {
            let erp = library.dir(&p.id).join(FILES).join(&a.path);
            resources.extend(
                a.added
                    .iter()
                    .filter(|uri| texarray::merged_by_slice(uri) && !merge::studio_only(uri))
                    .map(|uri| (erp.clone(), uri.clone())),
            );
        }
        if resources.is_empty() {
            return Ok((0, 0));
        }
        write_overlay(&resources, &self.dir.join(LOGOS))?;
        let mut seen = BTreeSet::new();
        let (mut arrays, mut views) = (0, 0);
        for (_, uri) in resources {
            if !seen.insert(uri.clone()) {
                continue;
            }
            let (erp, source) = (merge::BOOT_ERP.to_owned(), Source::addon(LOGOS));
            self.written.insert((erp.clone(), uri.clone()));
            self.erp.push(match texarray::is_array(&uri) {
                true => {
                    arrays += 1;
                    ErpOp::MergeSlices {
                        erp,
                        resource: uri,
                        source,
                    }
                }
                false => {
                    views += 1;
                    ErpOp::PutResource {
                        erp,
                        resource: uri,
                        source,
                        rename: None,
                    }
                }
            });
        }
        Ok((arrays, views))
    }

    fn log_diff(&mut self, p: &Package, diff: &Diff) {
        let log = &mut self.log;
        log.section("diff");
        let r = &diff.records;
        log.line(format!(
            "database: {} changed, {} added, {} removed record(s)",
            r.changed.len(),
            r.added.len(),
            r.removed.len()
        ));
        if diff.dropped > 0 {
            log.line(format!(
                "approximated: {} removal(s) left out, the mod was made for game version {}",
                diff.dropped,
                p.game_version.as_deref().unwrap_or("unknown")
            ));
        }
        for a in &diff.archives {
            let left: Vec<&String> = a
                .changed
                .iter()
                .chain(&a.added)
                .filter(|uri| !merge::loaded(a, uri) && !texarray::carrier(&a.path))
                .collect();
            log.line(format!(
                "archive {}: {} changed, {} added, {} removed resource(s)",
                a.path,
                a.changed.len(),
                a.added.len(),
                a.removed.len()
            ));
            for uri in left {
                log.line(format!(
                    "skipped resource {uri} in {}: Simplified Studio adds it to every shader archive",
                    a.path
                ));
            }
            if texarray::carrier(&a.path) {
                for uri in a.changed.iter().chain(&a.added).filter(|u| merge::studio_only(u)) {
                    log.line(format!(
                        "skipped resource {uri} in {}: Simplified Studio adds it to every shader archive",
                        a.path
                    ));
                }
            }
        }
        for t in &diff.text {
            log.line(format!(
                "text {}: {} changed, {} added, {} filled in{}",
                t.language,
                t.changed.len(),
                t.added.len(),
                t.defaults.len(),
                t.from
                    .as_ref()
                    .map_or_else(String::new, |f| format!(", from the mod's {f} file"))
            ));
        }
        for f in &diff.files {
            log.line(format!("file {} ({})", f.path, class_name(f.class)));
        }
    }

    fn log_ops(&mut self) {
        let log = &mut self.log;
        log.section("ops");
        let puts = self
            .db
            .iter()
            .filter(|o| matches!(o, DbOp::PutRecord { .. }))
            .count();
        if !self.db.is_empty() {
            log.line(format!(
                "{DATABASE_OPS}: {puts} put-record, {} remove-record",
                self.db.len() - puts
            ));
        }
        let mut removals: BTreeMap<&str, usize> = BTreeMap::new();
        let mut mips = 0;
        for op in &self.erp {
            match op {
                ErpOp::Overlay { erps, source } => {
                    let n = self.written.iter().filter(|(e, _)| erps.contains(e)).count();
                    log.line(format!(
                        "{ERP_OPS}: erp.overlay {} into {} ({n} resource(s) written there in all)",
                        source.path,
                        erps.join(", ")
                    ));
                }
                ErpOp::RemoveResource { erp, .. } => *removals.entry(erp).or_default() += 1,
                ErpOp::MergeSlices {
                    erp,
                    resource,
                    source,
                } => log.line(format!(
                    "{ERP_OPS}: merge-slices {resource} into {erp} from {}",
                    source.path
                )),
                ErpOp::PutResource {
                    erp,
                    resource,
                    source,
                    ..
                } => log.line(format!(
                    "{ERP_OPS}: put-resource {resource} into {erp} from {}",
                    source.path
                )),
                ErpOp::AddMipmaps { .. } => mips += 1,
                other => log.line(format!("{ERP_OPS}: {other:?}")),
            }
        }
        for (erp, n) in removals {
            log.line(format!("{ERP_OPS}: {n} remove-resource on {erp}"));
        }
        if mips > 0 {
            log.line(format!("{ERP_OPS}: {mips} add-mipmaps"));
        }
        let mut langs: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for op in &self.text {
            if let AssetOp::Lang(l) = op {
                let (kind, language) = match l {
                    LangOp::Set { language, .. } => ("lang.set", language.as_str()),
                    LangOp::Default { language, .. } => ("lang.default", language.as_str()),
                    LangOp::Table { language, .. } => ("lang.table", language.as_str()),
                };
                *langs.entry((language, kind)).or_default() += 1;
            }
        }
        for ((language, kind), n) in langs {
            log.line(format!("{TEXT_OPS}: {n} {kind} for {language}"));
        }
        for op in &self.files {
            if let AssetOp::File(FileOp::Put { target, .. }) = op {
                log.line(format!("{FILE_OPS}: file.put {target}"));
            }
        }
        for op in &self.track {
            log.line(format!("{TRACK_OPS}: {op:?}"));
        }
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
                "warning: {} left out: the mod was made for {}, not this game version",
                crate::plural(diff.dropped, "removal", "removals"),
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
                "text: {} entr(ies) the mod fills in with the game's own text → lang.default in {TEXT_OPS}",
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
                "logos: {} team logo texture array(s) → erp.merge-slices into {} from {LOGOS}",
                t.logos,
                merge::BOOT_ERP
            ));
        }
        if t.views > 0 {
            self.report.push(format!(
                "logos: {} texture array view(s) → put-resource into {} from {LOGOS}",
                t.views,
                merge::BOOT_ERP
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
        if area == Area::Vehicle && ["tyre", "tire", "/wheel"].iter().any(|w| lower.contains(w)) {
            self.tyres += 1;
        }
        *self.areas.entry(area).or_default() += 1;
    }

    // What the mod changes decides; its name only settles between two categories
    // its contents fit equally.
    fn category(&self, name: &str) -> Category {
        let n = |a| self.areas.get(&a).copied().unwrap_or(0);
        let (track, vehicle, ui, audio, other) = (
            n(Area::Track),
            n(Area::Vehicle),
            n(Area::Ui),
            n(Area::Audio),
            n(Area::Other),
        );
        let name = name.to_lowercase();
        let named = |words: &[&str]| words.iter().any(|w| name.contains(w));
        if self.track || track > 0 && track >= vehicle + ui {
            Category::Tracks
        } else if self.records >= SEASON_RECORDS && (vehicle > 0 || self.team_text > 0) {
            Category::Seasons
        } else if vehicle > 0 && vehicle >= ui.max(audio).max(other) {
            let liveries = self.vehicle_art == vehicle
                || named(&["livery", "liveries"]) && self.vehicle_art * 2 >= vehicle;
            match self.tyres * 2 < vehicle && liveries {
                true => Category::Liveries,
                false => Category::Cars,
            }
        } else if audio > 0 && audio >= ui + other {
            Category::Audio
        } else if self.records > 0 && self.team_text > 0 {
            Category::DriversTeams
        } else if self.records > 0 && named(&["season", "calendar"]) {
            Category::Seasons
        } else if self.records > 0 {
            Category::Database
        } else if self.text > 0 || ui > 0 {
            Category::Ui
        } else if vehicle > 0 {
            Category::Cars
        } else if audio > 0 {
            Category::Audio
        } else {
            Category::Other
        }
    }
}

impl Area {
    fn assets(self) -> (&'static str, &'static str) {
        match self {
            Self::Track => ("track asset", "track assets"),
            Self::Vehicle => ("car asset", "car assets"),
            Self::Ui => ("UI asset", "UI assets"),
            Self::Audio => ("sound asset", "sound assets"),
            Self::Other => ("game asset", "game assets"),
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

fn fill_in(op: AssetOp) -> AssetOp {
    match op {
        AssetOp::Lang(LangOp::Set { language, key, value }) => {
            AssetOp::Lang(LangOp::Default { language, key, value })
        }
        other => other,
    }
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
