// Parity harness: converts each mod with the Mod Importer, applies the addon's ops
// with the launcher's crates to clean copies of the game's files, and compares the
// game state with what the Simplified Launcher's install gives for the same mod.
//
// cargo run --release --example parity -- <mod>...
// PARITY_GAME (the F1 25 folder, read only), PARITY_DATA (Swofty's data folder, for
// its stock baseline), PARITY_ORIGINAL (the Simplified Launcher's app/original) and
// PARITY_SCRATCH (a scratch folder) override the defaults.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use swofty_addon::asset::{AssetOp, FileOp, LangOp, OpFile, Root, Source, TrackOp};
use swofty_addon::launcher::{Claim, GameFiles};
use swofty_addon::manifest::AddonManifest;
use swofty_addon::ops::{DbOp, DbOps, ErpOp, apply_db_ops};
use swofty_addon_simplified::library::{FILES, Library, Package};
use swofty_addon_simplified::package::{self, Class, DATABASE};
use swofty_addon_simplified::{convert, merge, texarray, track};
use swofty_core::erpbuild::ErpBuild;
use swofty_formats::erp::{ErpFile, Resource};
use swofty_formats::gvdb::Gvdb;
use swofty_formats::lng::Lng;

type R<T> = Result<T, String>;

const LANGUAGES: [&str; 11] = [
    "bra", "dut", "eng", "fre", "ger", "ita", "jap", "mex", "pol", "spa", "zhs",
];
const EXAMPLES: usize = 6;

fn env_path(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name).map_or_else(|| PathBuf::from(default), PathBuf::from)
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

struct Stock {
    game: PathBuf,
    objects: PathBuf,
    baseline: HashMap<String, String>,
    lang: PathBuf,
    key: String,
    depots: Option<swofty_core::stock::Depots>,
    verified: Mutex<HashMap<String, bool>>,
    homes: Mutex<Option<HashMap<String, Vec<String>>>>,
}

impl Stock {
    fn open(game: PathBuf, data: &Path) -> R<Self> {
        let base = data.join("baseline");
        let key = std::fs::read_dir(&base)
            .map_err(|e| format!("{}: {e}", base.display()))?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n != "objects" && base.join(n).join("baseline.json").is_file())
            .ok_or("no baseline")?;
        let doc: serde_json::Value = serde_json::from_slice(
            &std::fs::read(base.join(&key).join("baseline.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let baseline = doc["files"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v["sha256"].as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let library = game
            .ancestors()
            .find(|p| p.join("steamapps").is_dir())
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let install = swofty_core::steam::GameInstall {
            steam_root: home().join(".local/share/Steam"),
            library,
            dir: game.clone(),
            build_id: None,
            compatdata: None,
        };
        let depots = match swofty_core::stock::Depots::load(&install) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("warning: game files are not checked against Steam's manifests: {e}");
                None
            }
        };
        Ok(Self {
            objects: base.join("objects"),
            lang: base.join(&key).join("lang"),
            game,
            baseline,
            key,
            depots,
            verified: Mutex::new(HashMap::new()),
            homes: Mutex::new(None),
        })
    }

    fn stock_file(&self, rel: &str, path: &Path) -> bool {
        let Some(depots) = &self.depots else {
            return true;
        };
        let mut seen = self.verified.lock().unwrap();
        if let Some(ok) = seen.get(rel) {
            return *ok;
        }
        let ok = match depots.get(rel) {
            Some(e) => {
                std::fs::metadata(path).is_ok_and(|m| m.len() == e.size)
                    && swofty_core::stock::sha1_file(path).is_ok_and(|s| s == e.sha1)
            }
            None => true,
        };
        seen.insert(rel.to_owned(), ok);
        ok
    }
}

impl Stock {
    // The game's own copy of a resource, from the first archive under
    // 2025_asset_groups that holds it.
    fn holders(&self, uri: &str) -> R<Vec<String>> {
        let mut homes = self.homes.lock().unwrap();
        if homes.is_none() {
            let root = self.game.join("2025_asset_groups");
            let mut map = HashMap::new();
            for path in package::files(&root).map_err(|e| e.to_string())? {
                if path.extension().is_none_or(|e| e != "erp") {
                    continue;
                }
                let rel = path
                    .strip_prefix(&self.game)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Ok(f) = ErpFile::open(&path) {
                    for r in &f.erp.resources {
                        map.entry(r.name.clone())
                            .or_insert_with(Vec::new)
                            .push(rel.clone());
                    }
                }
            }
            *homes = Some(map);
        }
        Ok(homes.as_ref().unwrap().get(uri).cloned().unwrap_or_default())
    }

    fn copy_in(&self, rel: &str, uri: &str) -> R<Resource> {
        let path = self.original(rel).map_err(|e| e.to_string())?;
        let f = ErpFile::open(&path).map_err(|e| e.to_string())?;
        f.load(f.erp.find(uri).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }

    fn home_copy(&self, uri: &str) -> R<Option<(String, Resource)>> {
        let Some(rel) = self.holders(uri)?.into_iter().next() else {
            return Ok(None);
        };
        let path = self.original(&rel).map_err(|e| e.to_string())?;
        let f = ErpFile::open(&path).map_err(|e| e.to_string())?;
        let r = f
            .load(f.erp.find(uri).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Ok(Some((rel, r)))
    }
}

impl GameFiles for Stock {
    fn build_key(&self) -> String {
        self.key.clone()
    }

    fn exe_version(&self) -> Option<String> {
        swofty_core::exe_version::read(&self.game.join("F1_25.exe"))
    }

    fn is_stock(&self, rel: &str) -> bool {
        self.baseline.contains_key(rel) || self.game.join(rel).is_file()
    }

    fn original(&self, rel: &str) -> swofty_addon::Result<PathBuf> {
        let path = match self.baseline.get(rel) {
            Some(sha) => self.objects.join(&sha[..2]).join(sha),
            None => self.game.join(rel),
        };
        if !path.is_file() {
            return Err(swofty_addon::Error::Other(format!("{rel} is not a game file")));
        }
        if !self.baseline.contains_key(rel) && !self.stock_file(rel, &path) {
            return Err(swofty_addon::Error::Other(format!(
                "{rel} in the game folder is not the stock file"
            )));
        }
        Ok(path)
    }

    fn original_lang(&self, code: &str) -> swofty_addon::Result<PathBuf> {
        let path = self.lang.join(format!("language_{code}.lng"));
        match path.is_file() {
            true => Ok(path),
            false => Err(swofty_addon::Error::Other(format!("no {code}"))),
        }
    }
}

#[derive(Default)]
struct Findings {
    lines: Vec<String>,
    differences: usize,
}

impl Findings {
    fn same(&mut self, what: String) {
        self.lines.push(format!("  identical: {what}"));
    }

    fn differ(&mut self, what: String, examples: &[String]) {
        self.differences += 1;
        self.lines.push(format!("  DIFFERENT: {what}"));
        for e in examples.iter().take(EXAMPLES) {
            self.lines.push(format!("    {e}"));
        }
        if examples.len() > EXAMPLES {
            self.lines
                .push(format!("    … and {} more", examples.len() - EXAMPLES));
        }
    }

    fn note(&mut self, what: String) {
        self.lines.push(format!("  note: {what}"));
    }
}

// The Simplified Launcher's install of one mod on a clean game.
#[derive(Default)]
struct Expected {
    db: Option<Gvdb>,
    text: BTreeMap<String, BTreeMap<String, String>>,
    archives: Vec<(String, PathBuf)>,
    files: BTreeMap<String, PathBuf>,
}

// The converted addon's ops applied by the launcher's crates.
struct Got<'a> {
    db: Option<Gvdb>,
    text: BTreeMap<String, BTreeMap<String, String>>,
    erp: ErpBuild<'a>,
    removed: BTreeSet<(String, String)>,
    files: BTreeMap<String, PathBuf>,
    tracks: Vec<PathBuf>,
}

fn lng(path: &Path) -> R<Lng> {
    Lng::parse(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn entries(l: &Lng) -> HashMap<String, String> {
    l.entries
        .iter()
        .map(|e| (e.key.clone(), e.value.clone()))
        .collect()
}

// Merged.cs Languages: per folder, each game language takes the mod's own file for
// it, else its English, else its first by name; entries changed or added count.
fn expected_text(files: &[(String, PathBuf)], game: &Stock) -> R<BTreeMap<String, BTreeMap<String, String>>> {
    let mut folders: BTreeMap<String, BTreeMap<String, PathBuf>> = BTreeMap::new();
    for (rel, path) in files {
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        folders
            .entry(dir.to_owned())
            .or_default()
            .insert(name.to_owned(), path.clone());
    }
    let mut cleans: HashMap<&str, HashMap<String, String>> = HashMap::new();
    for code in LANGUAGES {
        let path = game.original_lang(code).map_err(|e| e.to_string())?;
        cleans.insert(code, entries(&lng(&path)?));
    }
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for set in folders.values() {
        for code in LANGUAGES {
            let own = format!("language_{code}.lng");
            let source = set
                .get(&own)
                .or_else(|| set.get("language_eng.lng"))
                .or_else(|| set.values().next());
            let Some(source) = source else { continue };
            let file = source.file_name().unwrap().to_string_lossy().to_lowercase();
            let clean_code = file
                .strip_prefix("language_")
                .and_then(|f| f.strip_suffix(".lng"))
                .unwrap_or("eng");
            let clean = &cleans[clean_code];
            let edits = out.entry(code.to_owned()).or_default();
            for e in lng(source)?.entries {
                if clean.get(&e.key) != Some(&e.value) {
                    edits.insert(e.key, e.value);
                }
            }
        }
    }
    Ok(out)
}

fn expected(p: &Package, files: &Path, game: &Stock) -> R<Expected> {
    let mut out = Expected::default();
    let mut languages = Vec::new();
    for f in &p.files {
        let path = files.join(&f.path);
        let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_ascii_lowercase();
        let game_language = LANGUAGES.iter().any(|c| name == format!("language_{c}.lng"));
        if f.class == Class::Database {
            let clean = Gvdb::parse(
                &std::fs::read(game.original(DATABASE).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let layer =
                Gvdb::parse(&std::fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            out.db = Some(layers(&clean, &[layer]));
        } else if game_language {
            languages.push((f.path.clone(), path));
        } else if f.class == Class::Archive {
            out.archives.push((f.path.clone(), path));
        } else {
            out.files.insert(f.path.clone(), path);
        }
    }
    out.text = expected_text(&languages, game)?;
    Ok(out)
}

// DatabaseLayers.cs Apply.
fn layers(clean: &Gvdb, layers: &[Gvdb]) -> Gvdb {
    let baseline: HashMap<u64, (u8, &Vec<u8>)> =
        clean.records.iter().map(|r| (r.key, (r.tag, &r.value))).collect();
    let mut result: BTreeMap<u64, Option<swofty_formats::gvdb::Record>> =
        clean.records.iter().map(|r| (r.key, Some(r.clone()))).collect();
    for layer in layers {
        let mut seen = BTreeSet::new();
        for r in &layer.records {
            seen.insert(r.key);
            if baseline
                .get(&r.key)
                .is_some_and(|(t, v)| *t == r.tag && **v == r.value)
            {
                continue;
            }
            result.insert(r.key, Some(r.clone()));
        }
        for k in baseline.keys() {
            if !seen.contains(k) {
                result.insert(*k, None);
            }
        }
    }
    Gvdb {
        records: result.into_values().flatten().collect(),
    }
}

fn addon_ops(dir: &Path) -> R<Vec<AssetOp>> {
    let m = AddonManifest::load(dir).map_err(|e| e.to_string())?;
    let mut ops = Vec::new();
    for (kind, rel) in m.operations.op_files() {
        let doc = serde_json::from_slice(&std::fs::read(dir.join(rel)).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        ops.extend(OpFile::from_value(doc, kind).map_err(|e| e.to_string())?.ops);
    }
    ops.extend(m.operations.file_ops());
    Ok(ops)
}

fn got<'a>(addon: &Path, game: &Stock, original: &'a dyn Fn(&str) -> PathBuf) -> R<Got<'a>> {
    let ops = addon_ops(addon)?;
    let resolve = |s: &Source| -> swofty_core::Result<PathBuf> {
        match s.root {
            Root::Addon => Ok(addon.join(&s.path)),
            _ => Err(swofty_core::Error::Other(format!(
                "{s:?} reads outside the addon"
            ))),
        }
    };
    let mut out = Got {
        db: None,
        text: BTreeMap::new(),
        erp: ErpBuild::new(original),
        removed: BTreeSet::new(),
        files: BTreeMap::new(),
        tracks: Vec::new(),
    };
    let mut db_ops: Vec<DbOp> = Vec::new();
    let mut defaults = Vec::new();
    for op in ops {
        match op {
            AssetOp::Db(op) => db_ops.push(op),
            AssetOp::Lang(LangOp::Set { language, key, value }) => {
                out.text.entry(language).or_default().insert(key, value);
            }
            AssetOp::Lang(LangOp::Default { language, key, value }) => defaults.push((language, key, value)),
            AssetOp::Lang(other) => return Err(format!("unexpected {other:?}")),
            AssetOp::File(FileOp::Put { source, target }) => {
                out.files
                    .insert(target, resolve(&source).map_err(|e| e.to_string())?);
            }
            AssetOp::Erp(ErpOp::AddMipmaps { source, target }) => {
                out.files
                    .insert(target, resolve(&source).map_err(|e| e.to_string())?);
            }
            AssetOp::Erp(op) => {
                match &op {
                    ErpOp::RemoveResource { erp, resource } => {
                        out.removed.insert((erp.clone(), resource.clone()));
                    }
                    ErpOp::PutResource { erp, resource, .. } | ErpOp::MergeSlices { erp, resource, .. } => {
                        out.removed.remove(&(erp.clone(), resource.clone()));
                    }
                    _ => {}
                }
                out.erp
                    .apply_for(&op, "addon", &resolve)
                    .map_err(|e| format!("{op:?}: {e}"))?;
            }
            AssetOp::Track(TrackOp::Add { source }) => {
                out.tracks.push(resolve(&source).map_err(|e| e.to_string())?)
            }
            other => return Err(format!("unexpected {other:?}")),
        }
    }
    for (language, key, value) in defaults {
        out.text.entry(language).or_default().entry(key).or_insert(value);
    }
    if !db_ops.is_empty() {
        let mut db = Gvdb::parse(
            &std::fs::read(game.original(DATABASE).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let ops = DbOps {
            schema_version: 1,
            fields: BTreeMap::new(),
            strict: false,
            ops: db_ops,
        };
        apply_db_ops(&mut db, &ops, &BTreeMap::<String, Vec<String>>::new()).map_err(|e| e.to_string())?;
        out.db = Some(db);
    }
    Ok(out)
}

fn data(r: &Resource) -> R<Vec<(String, u32, Vec<u8>)>> {
    r.fragments
        .iter()
        .map(|f| Ok((f.name(), f.flags, f.data().map_err(|e| e.to_string())?)))
        .collect()
}

fn same_resource(a: &Resource, b: &Resource) -> R<bool> {
    if a.kind != b.kind || a.unk != b.unk || a.fragments.len() != b.fragments.len() {
        return Ok(false);
    }
    let raw_same = a
        .fragments
        .iter()
        .zip(&b.fragments)
        .all(|(x, y)| x.compression == y.compression && x.raw == y.raw);
    Ok(raw_same || data(a)? == data(b)?)
}

fn compare_db(f: &mut Findings, exp: &Option<Gvdb>, got: &Option<Gvdb>, game: &Stock) -> R<()> {
    if exp.is_none() && got.is_none() {
        return Ok(());
    }
    let clean = || -> R<Gvdb> {
        Gvdb::parse(
            &std::fs::read(game.original(DATABASE).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    };
    let exp = match exp {
        Some(d) => d.clone(),
        None => clean()?,
    };
    let got = match got {
        Some(d) => d.clone(),
        None => clean()?,
    };
    let map = |d: &Gvdb| -> BTreeMap<u64, (u8, Vec<u8>)> {
        d.records
            .iter()
            .map(|r| (r.key, (r.tag, r.value.clone())))
            .collect()
    };
    let (e, g) = (map(&exp), map(&got));
    let mut diffs = Vec::new();
    for k in e.keys().chain(g.keys()).collect::<BTreeSet<_>>() {
        match (e.get(k), g.get(k)) {
            (Some(a), Some(b)) if a == b => {}
            (Some(_), Some(_)) => diffs.push(format!("0x{k:016x}: other value")),
            (Some(_), None) => diffs.push(format!("0x{k:016x}: missing in ours")),
            (None, Some(_)) => diffs.push(format!("0x{k:016x}: Simplified removes it, ours keeps it")),
            (None, None) => {}
        }
    }
    match diffs.is_empty() {
        true => f.same(format!("database ({} records)", e.len())),
        false => f.differ(format!("database: {} record(s)", diffs.len()), &diffs),
    }
    Ok(())
}

fn compare_text(
    f: &mut Findings,
    exp: &BTreeMap<String, BTreeMap<String, String>>,
    got: &BTreeMap<String, BTreeMap<String, String>>,
    game: &Stock,
) -> R<()> {
    let mut total = 0;
    let mut diffs = Vec::new();
    for code in LANGUAGES {
        let (e, g) = (exp.get(code), got.get(code));
        if e.is_none() && g.is_none() {
            continue;
        }
        let clean = entries(&lng(&game.original_lang(code).map_err(|e| e.to_string())?)?);
        let keys: BTreeSet<&String> = e.iter().chain(g.iter()).flat_map(|m| m.keys()).collect();
        for k in keys {
            total += 1;
            let want = e.and_then(|m| m.get(k)).or_else(|| clean.get(k));
            let have = g.and_then(|m| m.get(k)).or_else(|| clean.get(k));
            if want != have {
                diffs.push(format!(
                    "{code} {k}: Simplified {:?}, ours {:?}",
                    want.map(|s| s.chars().take(40).collect::<String>()),
                    have.map(|s| s.chars().take(40).collect::<String>())
                ));
            }
        }
    }
    if total == 0 {
        return Ok(());
    }
    match diffs.is_empty() {
        true => f.same(format!("text ({total} entries over the game's languages)")),
        false => f.differ(format!("text: {} of {total} entries", diffs.len()), &diffs),
    }
    Ok(())
}

fn compare_archives(f: &mut Findings, exp: &Expected, got: &Got, game: &Stock) -> R<()> {
    let mut owned: HashMap<(&str, &str), &Resource> = HashMap::new();
    for (erp, r, _, _) in got.erp.resources() {
        owned.insert((erp, r.name.as_str()), r);
    }
    let mut seen_owned: BTreeSet<(String, String)> = BTreeSet::new();
    for (rel, path) in &exp.archives {
        let modf = ErpFile::open(path).map_err(|e| format!("{rel}: {e}"))?;
        let stock = ErpFile::open(&game.original(rel).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{rel}: {e}"))?;
        let names: BTreeSet<&str> = modf
            .erp
            .resources
            .iter()
            .chain(&stock.erp.resources)
            .map(|r| r.name.as_str())
            .collect();
        let (mut same, mut diffs) = (0, Vec::new());
        for uri in names {
            // The Simplified Launcher's Python shader merge unions the game's archive
            // with the mod's: the mod wins where they overlap, and a stock resource the
            // mod's archive omits is kept. Other archives are the mod's whole, so a
            // stock resource the mod omits is gone. This is the content it would load.
            let want: Option<Resource> = match modf.erp.find(uri) {
                Ok(r) => Some(modf.load(r).map_err(|e| e.to_string())?),
                Err(_) if merge::carrier(rel) => match stock.erp.find(uri) {
                    Ok(r) => Some(stock.load(r).map_err(|e| e.to_string())?),
                    Err(_) => None,
                },
                Err(_) => None,
            };
            let from_overlay = owned
                .get(&(merge::BOOT_ERP, uri))
                .map(|r| (merge::BOOT_ERP, *r))
                .or_else(|| owned.get(&(rel.as_str(), uri)).map(|r| (rel.as_str(), *r)));
            if let Some((erp, _)) = from_overlay {
                seen_owned.insert((erp.to_owned(), uri.to_owned()));
                seen_owned.insert((rel.clone(), uri.to_owned()));
            }
            let have: Option<Resource> = match from_overlay {
                Some((_, r)) => Some(r.clone()),
                None if got.removed.contains(&(rel.clone(), uri.to_owned())) => None,
                None => match stock.erp.find(uri) {
                    Ok(r) => Some(stock.load(r).map_err(|e| e.to_string())?),
                    Err(_) => None,
                },
            };
            match (want, have) {
                (None, None) => same += 1,
                (Some(w), Some(h)) => match same_resource(&w, &h)? {
                    true => same += 1,
                    false => diffs.push(format!("{uri}: other content")),
                },
                (Some(_), None) if uri.starts_with(merge::MARKER) => {
                    same += 1;
                    f.note(format!(
                        "{uri} in {rel}: Simplified Studio's marker, no resource of the game"
                    ));
                }
                (Some(w), None) => match game.home_copy(uri)? {
                    Some((home, h)) if same_resource(&w, &h)? => {
                        same += 1;
                        f.note(format!("{uri} in {rel} is the game's own copy from {home}"));
                    }
                    Some((home, _)) => diffs.push(format!(
                        "{uri}: missing in ours, and differs from the game's in {home}"
                    )),
                    None => diffs.push(format!("{uri}: missing in ours")),
                },
                (None, Some(_)) => diffs.push(format!(
                    "{uri}: Simplified's archive lacks it, ours keeps the game's"
                )),
            }
        }
        if merge::carrier(rel) {
            let added: Vec<&str> = modf
                .erp
                .resources
                .iter()
                .map(|r| r.name.as_str())
                .filter(|n| stock.erp.find(n).is_err())
                .collect();
            let booted = added
                .iter()
                .filter(|n| owned.contains_key(&(merge::BOOT_ERP, **n)))
                .count();
            f.note(format!(
                "{rel}: {} resource(s) the game's archive lacks, {booted} of them in the boot overlay",
                added.len()
            ));
            if booted < added.len() {
                diffs.push(format!("{} added resource(s) not at boot", added.len() - booted));
            }
        }
        match diffs.is_empty() {
            true => f.same(format!("{rel} ({same} resources)")),
            false => f.differ(
                format!("{rel}: {} of {} resources", diffs.len(), same + diffs.len()),
                &diffs,
            ),
        }
    }
    // The boot archive's overlay replaces every copy of a resource. Shader archives and
    // the boot archive itself do so in the Simplified Launcher too; any other archive
    // replaces only its own copies there.
    let shipped: BTreeSet<&str> = exp.archives.iter().map(|(rel, _)| rel.as_str()).collect();
    let mut global: BTreeSet<String> = BTreeSet::new();
    for (rel, path) in &exp.archives {
        if merge::carrier(rel) || rel == merge::BOOT_ERP {
            let f = ErpFile::open(path).map_err(|e| format!("{rel}: {e}"))?;
            global.extend(f.erp.resources.iter().map(|r| r.name.clone()));
        }
    }
    let (mut beyond, mut checked) = (Vec::new(), 0);
    for (erp, r, _, _) in got.erp.resources() {
        if erp != merge::BOOT_ERP || global.contains(&r.name) || texarray::is_array(&r.name) {
            continue;
        }
        for holder in game.holders(&r.name)? {
            if shipped.contains(holder.as_str()) || holder == merge::BOOT_ERP {
                continue;
            }
            checked += 1;
            if !same_resource(&game.copy_in(&holder, &r.name)?, r)? {
                beyond.push(format!("{}: the game's copy in {holder} is replaced too", r.name));
            }
        }
    }
    match beyond.is_empty() {
        true if checked > 0 => f.same(format!(
            "{checked} copies in archives the mod does not ship already match"
        )),
        true => {}
        false => f.differ(
            format!(
                "{} of {checked} copies in archives the mod does not ship: ours replaces them, Simplified keeps the game's",
                beyond.len()
            ),
            &beyond,
        ),
    }
    let extra: Vec<String> = got
        .erp
        .resources()
        .filter(|(erp, r, _, _)| !seen_owned.contains(&(erp.to_string(), r.name.clone())))
        .map(|(erp, r, _, _)| format!("{} in {erp}", r.name))
        .collect();
    if !extra.is_empty() {
        f.differ(
            format!(
                "{} resource(s) written that no archive of the mod holds",
                extra.len()
            ),
            &extra,
        );
    }
    Ok(())
}

fn compare_files(f: &mut Findings, exp: &Expected, got: &Got) -> R<()> {
    let mut diffs = Vec::new();
    let mut same = 0;
    for (target, src) in &exp.files {
        match got.files.get(target) {
            Some(ours) if std::fs::read(ours).ok() == std::fs::read(src).ok() => same += 1,
            Some(_) => diffs.push(format!("{target}: other bytes")),
            None => diffs.push(format!("{target}: not written by ours")),
        }
    }
    for target in got.files.keys().filter(|t| !exp.files.contains_key(*t)) {
        diffs.push(format!("{target}: written by ours only"));
    }
    if same + diffs.len() == 0 {
        return Ok(());
    }
    match diffs.is_empty() {
        true => f.same(format!("loose files ({same})")),
        false => f.differ(
            format!("loose files: {} of {}", diffs.len(), same + diffs.len()),
            &diffs,
        ),
    }
    Ok(())
}

fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    package::files(dir)
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
            (rel, std::fs::read(&p).unwrap_or_default())
        })
        .collect()
}

fn check_package(library: &Library, p: &Package, game: &Stock, scratch: &Path) -> R<Findings> {
    let mut f = Findings::default();
    let addon = scratch.join("addon").join(&p.id);
    std::fs::remove_dir_all(&addon).ok();
    let c = convert::convert(library, p, Some(game), &addon, &mut |_, _| {}).map_err(|e| e.to_string())?;
    f.note(format!(
        "converted into {} ({}), {}",
        c.id,
        c.category.label(),
        c.summary
    ));
    for line in c
        .report
        .iter()
        .filter(|l| l.starts_with("warning:") || l.starts_with("needs:"))
    {
        f.note(line.clone());
    }
    for path in &p.skipped {
        f.differ(
            format!("{path}: the Simplified Launcher installs it, ours skips it"),
            &[],
        );
    }
    let files = library.dir(&p.id).join(FILES);
    if let Some(code) = &p.track {
        let original = |rel: &str| game.game.join(rel);
        let got = got(&addon, game, &original)?;
        let carried = got.tracks.iter().map(|t| tree(t)).collect::<Vec<_>>();
        let shipped = tree(&library.dir(&p.id).join(track::TRACK));
        match carried.first() == Some(&shipped) {
            true => f.same(format!(
                "track {code}: the package's {} file(s) ride in track.add",
                shipped.len()
            )),
            false => f.differ(
                format!("track {code}: the addon's package differs from the mod's"),
                &[],
            ),
        }
        return Ok(f);
    }
    let exp = expected(p, &files, game)?;
    let original = |rel: &str| game.original(rel).unwrap_or_else(|_| game.game.join(rel));
    let got = got(&addon, game, &original)?;
    compare_db(&mut f, &exp.db, &got.db, game)?;
    compare_text(&mut f, &exp.text, &got.text, game)?;
    compare_archives(&mut f, &exp, &got, game)?;
    compare_files(&mut f, &exp, &got)?;
    let logos = got
        .erp
        .resources()
        .filter(|(_, r, _, _)| texarray::is_array(&r.name))
        .count();
    if logos > 0 {
        f.note(format!("{logos} texture array(s) merged slice by slice"));
    }
    std::fs::remove_dir_all(&addon).ok();
    Ok(f)
}

fn names_of(path: &Path) -> Vec<String> {
    if path.is_dir() {
        return package::files(path)
            .unwrap_or_default()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext == "7z" {
        let Ok(r) = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty()) else {
            return Vec::new();
        };
        return r.archive().files.iter().map(|e| e.name().to_owned()).collect();
    }
    std::fs::File::open(path)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .map(|z| {
            z.file_names()
                .filter_map(|n| n.ok().map(|n| n.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

fn not_f1(path: &Path) -> Option<&'static str> {
    let names = names_of(path).join("\n").to_lowercase();
    if names.contains("assettocorsa/") || names.contains("ui_skin.json") {
        Some("an Assetto Corsa car skin")
    } else if names.contains("vehicles/textures/customliveries") {
        Some("an Automobilista 2 livery pack")
    } else if names.contains(".dds") && !names.contains(".erp") {
        Some("loose DDS textures for another game")
    } else {
        None
    }
}

fn main() {
    let game = env_path("PARITY_GAME", "/mnt/gaming/SteamLibrary/steamapps/common/F1 25");
    let data =
        std::env::var_os("PARITY_DATA").map_or_else(|| home().join(".local/share/swofty"), PathBuf::from);
    let scratch = env_path("PARITY_SCRATCH", "/mnt/work/.dbedit/parity-scratch");
    let stock = match Stock::open(game, &data) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    let mut failed = false;
    for arg in std::env::args().skip(1) {
        let path = PathBuf::from(&arg);
        let name = package::file_name(&path);
        println!("== {name}");
        let claimed = package::claims(&path) == Claim::Yes
            || track::claims(&path) == Claim::Yes
            || path.join(track::MANIFEST).is_file();
        if let Some(what) = not_f1(&path).filter(|_| !claimed || path.extension().is_some_and(|e| e == "7z"))
        {
            println!("  skipped: {what}, not an F1 25 mod");
            continue;
        }
        if !claimed {
            println!("  skipped: no F1 25 game folders, mod.serps or track package");
            continue;
        }
        let data = scratch.join("data").join(name.replace(' ', "_"));
        std::fs::remove_dir_all(&data).ok();
        let library = Library::new(&data);
        let started = std::time::Instant::now();
        let imported = match library.import(&path, Some(&stock), &mut |_, _| {}) {
            Ok(i) => i,
            Err(e) => {
                println!("  import failed: {e}");
                failed = true;
                continue;
            }
        };
        println!("  imported as {} package(s)", imported.packages.len());
        for p in &imported.packages {
            println!("  -- {} ({})", p.name, p.id);
            match check_package(&library, p, &stock, &scratch) {
                Ok(f) => {
                    for l in &f.lines {
                        println!("  {l}");
                    }
                    println!(
                        "    => {}",
                        match f.differences {
                            0 => "identical".to_owned(),
                            n => format!("{n} difference(s)"),
                        }
                    );
                }
                Err(e) => {
                    println!("    error: {e}");
                    failed = true;
                }
            }
        }
        println!("  ({:.1} s)", started.elapsed().as_secs_f64());
        if std::env::var_os("PARITY_KEEP").is_none() {
            std::fs::remove_dir_all(&data).ok();
        }
    }
    if failed {
        std::process::exit(1);
    }
}
