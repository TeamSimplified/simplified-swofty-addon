use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use swofty_addon::asset::{AssetOp, FileOp, LangOp, Op, OpFile, Origin, Root, Source, TrackOp};
use swofty_addon::launcher::GameFiles;
use swofty_addon::manifest::{AddonManifest, Category, Kind};
use swofty_addon::ops::{DbOp, DbOps, ErpOp, apply_db_ops};
use swofty_formats::erp::{Erp, ErpFile, Fragment, HEADER_LEN, MAGIC, Resource};
use swofty_formats::gvdb::{Gvdb, Record, ValueType};
use swofty_formats::lng::Lng;

use sha2::Digest;

use super::*;
use crate::package::DATABASE;

const EXE: &str = "1.0.141.7865";
const BUILD: &str = "25251167-0123456789ab";
const CAR: &str = "2025_asset_groups/f1_2026_vehicle_package/teams/mclaren/wep/mclaren.erp";
const OPAQUE: &str = "shader_package_2025/object_opaque.nefx2.sm51.erp";
const UI_TEXTURE: &str = "shader_package_2025/ui_texture.nefx2.sm51.erp";
const FLOW_BOOT: &str = "2025_asset_groups/ui_package/flow_boot.erp";
const LOGO_ARRAY: &str =
    "eaid://ui_package/art/texture_arrays/overrides.uidb/overrides.team_logos.f1_2026.texarray";

struct Game {
    dir: PathBuf,
    database: Option<PathBuf>,
    installed: Option<PathBuf>,
}

impl GameFiles for Game {
    fn build_key(&self) -> String {
        BUILD.into()
    }

    fn exe_version(&self) -> Option<String> {
        Some(EXE.into())
    }

    fn is_stock(&self, rel: &str) -> bool {
        self.original(rel).is_ok()
    }

    fn original(&self, rel: &str) -> Result<PathBuf> {
        let path = match (&self.database, rel == DATABASE, &self.installed) {
            (Some(db), true, _) => db.clone(),
            (_, _, Some(game)) if !self.dir.join(rel).is_file() => game.join(rel),
            _ => self.dir.join(rel),
        };
        match path.is_file() {
            true => Ok(path),
            false => Err(err(format!("{rel} is not a game file"))),
        }
    }

    fn original_lang(&self, code: &str) -> Result<PathBuf> {
        let path = self.dir.join(format!("lang/language_{code}.lng"));
        match path.is_file() {
            true => Ok(path),
            false => Err(err(format!("no {code}"))),
        }
    }
}

struct Case {
    root: PathBuf,
    game: Game,
}

impl Case {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("simplified-convert-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        Self {
            game: Game {
                dir: root.join("game"),
                database: None,
                installed: None,
            },
            root,
        }
    }

    fn stock(&self, rel: &str, bytes: &[u8]) {
        write(&self.game.dir.join(rel), bytes);
    }

    fn library(&self) -> Library {
        Library::new(&self.data())
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn import(&self, files: &[(&str, Vec<u8>)], serps: &str) -> Package {
        let src = self.root.join("src").join("Mod");
        write(&src.join("mod.serps"), serps.as_bytes());
        for (rel, bytes) in files {
            write(&src.join("F1 25").join(rel), bytes);
        }
        let game: &dyn GameFiles = &self.game;
        let mut imported = self.library().import(&src, Some(game), &mut |_, _| {}).unwrap();
        imported.packages.remove(0)
    }

    fn convert(&self, p: &Package, game: Option<&dyn GameFiles>) -> (Converted, PathBuf) {
        let out = self.root.join("addon");
        let c = convert(&self.library(), p, game, &out, &mut |_, _| {}).unwrap();
        (c, out)
    }

    // The build hook's ops for this one mod, as the library merge gives them.
    fn oracle(&self, p: &Package) -> Vec<AssetOp> {
        let library = self.library();
        if p.track.is_some() {
            return vec![track::op(p)];
        }
        let game: &dyn GameFiles = &self.game;
        let diff = library.diff(p, game).unwrap();
        let records = library.records(&p.id, &diff.build_key).unwrap();
        let merged = merge::ops(&p.id, &diff, &records);
        let origin = Origin {
            addon: ID.into(),
            item: Some(p.id.clone()),
        };
        let wrap = |op| Op {
            op,
            origin: origin.clone(),
        };
        let mut ops: Vec<Op> = merged.ops.into_iter().map(wrap).collect();
        let defaults = merged.defaults.into_iter().map(wrap).collect();
        ops.splice(0..0, crate::only_defaults(defaults, &ops));
        let mut ops: Vec<AssetOp> = crate::without_repeats(ops).into_iter().map(|o| o.op).collect();
        let carried: Vec<texarray::Carried> = diff
            .archives
            .iter()
            .filter(|a| texarray::carrier(&a.path) && !a.added.is_empty())
            .map(|a| texarray::Carried {
                item: p.name.clone(),
                erp: library.dir(&p.id).join(FILES).join(&a.path),
                uris: crate::ui_uris(&a.added),
            })
            .collect();
        let ui = texarray::build(game, &carried, &self.root.join("cache").join(texarray::MERGED)).unwrap();
        ops.extend(ui.ops);
        ops
    }

    fn resolve(&self, s: &Source, addon: &Path) -> PathBuf {
        match s.root {
            Root::Addon => s.resolve(addon).unwrap(),
            Root::Data => self.data().join(&s.path),
            Root::Cache => self.root.join("cache").join(&s.path),
        }
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn lng(pairs: &[(&str, &str)]) -> Vec<u8> {
    let mut l = Lng::default();
    for (k, v) in pairs {
        l.set(k, *v);
    }
    l.to_bytes()
}

fn db(records: &[(&str, u8, &str)]) -> Vec<u8> {
    Gvdb {
        records: records.iter().map(|(k, t, v)| Record::new(k, *t, *v)).collect(),
    }
    .to_bytes()
}

fn fragment(data: &[u8]) -> Fragment {
    Fragment {
        name: *b"temp",
        offset: 0,
        size: data.len() as u64,
        flags: 0x10,
        compression: 0,
        packed: data.len() as u64,
        raw: data.to_vec(),
    }
}

fn resource(name: &str, kind: &str, fragments: &[&[u8]]) -> Resource {
    let mut k = [0u8; 16];
    k[..kind.len()].copy_from_slice(kind.as_bytes());
    Resource {
        name: name.into(),
        kind: k,
        unk: vec![0; 4],
        fragments: fragments.iter().map(|d| fragment(d)).collect(),
        hash: [7; 16],
    }
}

fn texture(name: &str, pixels: &[u8]) -> Resource {
    resource(name, "GfxSurfaceRes", &[b"head", pixels])
}

fn surface(name: &str, slices: &[[u8; 4]]) -> Resource {
    let mut head = vec![0u8; 32];
    head[12..16].copy_from_slice(&4u32.to_le_bytes());
    head[28..32].copy_from_slice(&(slices.len() as u32).to_le_bytes());
    resource(name, "GfxSurfaceRes", &[&head, &slices.concat()])
}

fn archive(resources: Vec<Resource>) -> Vec<u8> {
    let mut header = [0u8; HEADER_LEN];
    header[..4].copy_from_slice(MAGIC);
    header[4..8].copy_from_slice(&3u32.to_le_bytes());
    Erp {
        header,
        version: 3,
        resources,
    }
    .to_bytes()
}

fn addon_ops(dir: &Path) -> (AddonManifest, Vec<AssetOp>) {
    let m = AddonManifest::load(dir).unwrap();
    let mut ops = Vec::new();
    for (kind, rel) in m.operations.op_files() {
        let doc = serde_json::from_slice(&std::fs::read(dir.join(rel)).unwrap()).unwrap();
        ops.extend(OpFile::from_value(doc, kind).unwrap().ops);
    }
    ops.extend(m.operations.file_ops());
    (m, ops)
}

type Res = (Vec<u8>, Vec<u8>, Vec<(String, u32, u8, Vec<u8>)>, [u8; 16]);

#[derive(Debug, Default, PartialEq)]
struct Applied {
    db: Option<Vec<u8>>,
    text: BTreeMap<(String, String), String>,
    files: BTreeMap<String, Vec<u8>>,
    resources: BTreeMap<(String, String), Option<Res>>,
    tracks: Vec<BTreeMap<String, Vec<u8>>>,
}

fn res(r: &Resource) -> Res {
    let fragments = r
        .fragments
        .iter()
        .map(|f| (f.name(), f.flags, f.compression, f.raw.clone()))
        .collect();
    (r.kind.to_vec(), r.unk.clone(), fragments, r.hash)
}

fn load(path: &Path, uri: &str) -> Res {
    let file = ErpFile::open(path).unwrap();
    res(&file.load(file.erp.find(uri).unwrap()).unwrap())
}

fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    crate::package::files(dir)
        .unwrap()
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
            (rel, std::fs::read(&p).unwrap())
        })
        .collect()
}

// Applies ops as the launcher's build does: merge-slices writes the slices that
// differ from the game's over the game's array, and a lang.default holds only where
// no lang.set gives the key.
fn apply(
    ops: &[AssetOp],
    stock: Option<&Gvdb>,
    game: &dyn GameFiles,
    resolve: &dyn Fn(&Source) -> PathBuf,
) -> Applied {
    static MERGES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut out = Applied::default();
    let mut db_ops = Vec::new();
    let mut defaults = Vec::new();
    for op in ops {
        match op {
            AssetOp::Db(op) => db_ops.push(op.clone()),
            AssetOp::Lang(LangOp::Set { language, key, value }) => {
                out.text.insert((language.clone(), key.clone()), value.clone());
            }
            AssetOp::Lang(LangOp::Default { language, key, value }) => {
                defaults.push(((language.clone(), key.clone()), value.clone()));
            }
            AssetOp::Erp(ErpOp::MergeSlices {
                erp,
                resource,
                source,
            }) => {
                let n = MERGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let to =
                    std::env::temp_dir().join(format!("simplified-merge-{}-{n}.erp", std::process::id()));
                let carried = texarray::Carried {
                    item: "addon".into(),
                    erp: resolve(source),
                    uris: vec![resource.clone()],
                };
                if !texarray::build(game, &[carried], &to).unwrap().ops.is_empty() {
                    out.resources
                        .insert((erp.clone(), resource.clone()), Some(load(&to, resource)));
                    std::fs::remove_file(&to).ok();
                }
            }
            AssetOp::File(FileOp::Put { source, target })
            | AssetOp::Erp(ErpOp::AddMipmaps { source, target }) => {
                out.files
                    .insert(target.clone(), std::fs::read(resolve(source)).unwrap());
            }
            AssetOp::Erp(ErpOp::PutResource {
                erp,
                resource,
                source,
                rename: None,
            }) => {
                let r = load(&resolve(source), resource);
                out.resources.insert((erp.clone(), resource.clone()), Some(r));
            }
            AssetOp::Erp(ErpOp::Overlay { erps, source }) => {
                let path = resolve(source);
                let file = ErpFile::open(&path).unwrap();
                for r in &file.erp.resources {
                    for erp in erps {
                        let loaded = res(&file.load(r).unwrap());
                        out.resources.insert((erp.clone(), r.name.clone()), Some(loaded));
                    }
                }
            }
            AssetOp::Erp(ErpOp::RemoveResource { erp, resource }) => {
                out.resources.insert((erp.clone(), resource.clone()), None);
            }
            AssetOp::Track(TrackOp::Add { source }) => out.tracks.push(tree(&resolve(source))),
            other => panic!("unexpected op {other:?}"),
        }
    }
    for (key, value) in defaults {
        out.text.entry(key).or_insert(value);
    }
    if let Some(stock) = stock {
        let mut db = stock.clone();
        let ops = DbOps {
            schema_version: 1,
            fields: BTreeMap::new(),
            strict: false,
            side_data: Default::default(),
            ops: db_ops,
        };
        apply_db_ops(&mut db, &ops, &BTreeMap::<String, Vec<String>>::new()).unwrap();
        out.db = Some(db.to_bytes());
    }
    out
}

fn check(case: &Case, p: &Package, out: &Path, stock: Option<&Gvdb>) -> (AddonManifest, Applied) {
    let (m, ops) = addon_ops(out);
    let game: &dyn GameFiles = &case.game;
    let expected = apply(&case.oracle(p), stock, game, &|s| case.resolve(s, out));
    let got = apply(&ops, stock, game, &|s| {
        assert_eq!(s.root, Root::Addon, "{s:?} reads outside the addon");
        case.resolve(s, out)
    });
    assert_eq!(got, expected);
    (m, got)
}

fn sorted(bytes: &[u8]) -> Vec<(u64, u8, Vec<u8>)> {
    let mut r: Vec<_> = Gvdb::parse(bytes)
        .unwrap()
        .records
        .into_iter()
        .map(|r| (r.key, r.tag, r.value))
        .collect();
    r.sort();
    r
}

fn serps(name: &str, version: &str) -> String {
    format!(r#"{{"mod_name":"{name}","author":"Venturi","version":"{version}","game_version":"{EXE}"}}"#)
}

fn reported(c: &Converted, start: &str) -> bool {
    c.report.iter().any(|l| l.starts_with(start))
}

#[test]
fn a_team_rename_becomes_database_and_text_ops() {
    let case = Case::new("rename");
    let stock_db = db(&[
        ("teams.ast.name", 2, "lng_team_ast"),
        ("teams.rac.name", 2, "lng_team_rac"),
        ("rules.laps", 0, "5"),
        ("rules.old", 0, "1"),
    ]);
    case.stock(DATABASE, &stock_db);
    for code in ["eng", "ger", "kor"] {
        let (ast, rac) = (format!("Aston {code}"), format!("Bulls {code}"));
        case.stock(
            &format!("lang/language_{code}.lng"),
            &lng(&[("lng_team_ast", &ast), ("lng_team_rac", &rac)]),
        );
    }
    let mod_db = db(&[
        ("teams.ast.name", 2, "lng_team_ast_26"),
        ("teams.rac.name", 2, "lng_team_rac_26"),
        ("rules.laps", 0, "7"),
        ("rules.new", 0, "9"),
    ]);
    let p = case.import(
        &[
            (DATABASE, mod_db.clone()),
            (
                "localisatioa/2025_release/language_eng.lng",
                lng(&[
                    ("lng_team_ast", "Aston eng"),
                    ("lng_team_rac", "Bulls eng"),
                    ("lng_team_ast_26", "Aston eng"),
                    ("lng_team_rac_26", "Cupra"),
                ]),
            ),
            (
                "localisatioa/2025_release/language_ger.lng",
                lng(&[("lng_team_ast_26", "Aston eng"), ("lng_team_rac_26", "Cupra DE")]),
            ),
        ],
        &serps("Cupra Racing Bulls", "1.0"),
    );
    let game: &dyn GameFiles = &case.game;
    let (c, out) = case.convert(&p, Some(game));
    assert!(!c.base, "a team rename needs no SERPs Base Files");
    assert_eq!(c.id, "mod.venturi.cupra-racing-bulls");
    assert_eq!(c.category, None);
    assert_eq!(c.kinds, [Kind::Database, Kind::Text, Kind::Files]);
    let stock = Gvdb::parse(&stock_db).unwrap();
    let (m, got) = check(&case, &p, &out, Some(&stock));
    assert_eq!(sorted(got.db.as_deref().unwrap()), sorted(&mod_db));
    let text = |l: &str, k: &str| got.text[&(l.to_owned(), k.to_owned())].as_str();
    assert_eq!(text("eng", "lng_team_rac_26"), "Cupra");
    assert_eq!(text("ger", "lng_team_rac_26"), "Cupra DE");
    assert_eq!(text("kor", "lng_team_rac_26"), "Cupra");
    assert_eq!(text("kor", "lng_team_ast_26"), "Aston kor");
    assert_eq!(got.text.len(), 6);
    assert_eq!(m.version.to_string(), "1.0.0");
    assert_eq!(m.compat.game_builds[0].to_string(), ">=25251167");
    assert_eq!(m.compat.launcher.as_ref().unwrap().to_string(), ">=1.3.0");
    let origin = m.origin.as_ref().unwrap();
    assert_eq!((origin.tool.as_str(), origin.file.as_deref()), (ID, Some("Mod")));
    assert_eq!(m.category, None);
    assert!(reported(&c, "database: 3 changed, 1 added, 1 removed"));
    assert!(reported(&c, "text: 3 entr(ies) the mod fills in"));
    assert!(!c.report.iter().any(|l| l.starts_with("warning:")));
    let (_, ops) = addon_ops(&out);
    let filled = ops
        .iter()
        .filter(|op| matches!(op, AssetOp::Lang(LangOp::Default { .. })))
        .count();
    assert_eq!(filled, 3, "the game's own text fills in with lang.default");
    assert_eq!(c.summary, "5 database records, 6 text entries");
    assert!(reported(&c, "text: kor take the mod's eng text"));
    assert!(out.join(REPORT).is_file());
}

#[test]
fn a_conversion_writes_a_log_for_developers() {
    let case = Case::new("log");
    case.stock(DATABASE, &db(&[("rules.laps", 0, "5"), ("rules.old", 0, "1")]));
    case.stock("lang/language_eng.lng", &lng(&[("lng_a", "A")]));
    let uri = "eaid://f1_2026_vehicle_package/teams/mclaren/paint.tif.image";
    case.stock(CAR, &archive(vec![texture(uri, b"small")]));
    let shader = resource("eaid://shader_package/opaque", "Shader", &[b"code"]);
    let amd = OPAQUE.replace(".sm51.erp", ".sm51_amd.erp");
    let livery = texture(
        "eaid://f1_2026_vehicle_package/teams/mercedes/livery.tif.image",
        b"w17",
    );
    let studio = archive(vec![
        shader.clone(),
        resource("eaid://tools_package/default/serps", "Marker", &[b"x"]),
        livery,
    ]);
    for rel in [OPAQUE, amd.as_str()] {
        case.stock(rel, &archive(vec![shader.clone()]));
    }
    let p = case.import(
        &[
            (OPAQUE, studio.clone()),
            (amd.as_str(), studio),
            (DATABASE, db(&[("rules.laps", 0, "7")])),
            (
                "localisatioa/2025_release/language_eng.lng",
                lng(&[("lng_a", "B")]),
            ),
            (CAR, archive(vec![texture(uri, b"bigger texture")])),
            ("readme.exe", b"MZ".to_vec()),
        ],
        &serps("Logged", "1.0"),
    );
    let game: &dyn GameFiles = &case.game;
    let (c, _) = case.convert(&p, Some(game));
    let logs: Vec<PathBuf> = std::fs::read_dir(case.data().join(crate::log::DIR))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(logs.len(), 1);
    let name = logs[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("convert-") && name.ends_with(&format!("-{}.log", c.id)),
        "{name}"
    );
    let text = std::fs::read_to_string(&logs[0]).unwrap();
    for line in [
        "[source]",
        "file: Mod",
        "sha256: none",
        "[game]",
        &format!("build key: {BUILD}"),
        "[files]",
        "class database: 1 file(s)",
        "class text: 1 file(s)",
        "class archive: 3 file(s)",
        "skipped file readme.exe: programs, EA AntiCheat and EA installer files are never copied",
        "[diff]",
        "database: 1 changed, 0 added, 1 removed record(s)",
        &format!("archive {CAR}: 1 changed, 0 added, 0 removed resource(s)"),
        "text eng: 1 changed, 0 added, 0 filled in",
        &format!("archive {OPAQUE}: 0 changed, 2 added, 0 removed resource(s)"),
        &format!("archive {amd}: 0 changed, 2 added, 0 removed resource(s)"),
        &format!(
            "skipped resource eaid://tools_package/default/serps in {OPAQUE}: Simplified Studio adds it to every shader archive"
        ),
        "gpu: 1 resource(s) the generic and AMD shader archives both carry load once, 0 of them differ",
        "ops/erp.json: erp.overlay erp/overlay-2.erp into 2025_asset_groups/vfx_package/common.erp (1 resource(s)",
        "[ops]",
        "ops/database.json: 1 put-record, 1 remove-record",
        "ops/text.json: 1 lang.set for eng",
        "[result]",
        &format!("addon: {}", c.id),
        "category: ",
        "time: diff ",
        "time: total ",
    ] {
        assert!(
            text.lines().any(|l| l.starts_with(line)),
            "no {line:?} in\n{text}"
        );
    }
    let out = case.root.join("failed");
    assert!(convert(&case.library(), &p, None, &out, &mut |_, _| {}).is_err());
    let logs = std::fs::read_dir(case.data().join(crate::log::DIR))
        .unwrap()
        .count();
    assert_eq!(logs, 2, "a failed conversion logs its error too");
}

#[test]
fn a_studio_livery_in_both_gpu_shader_archives_loads_once_at_boot() {
    let case = Case::new("gpu");
    let shader = resource("eaid://shader_package/mask", "Shader", &[b"code"]);
    let generic = "shader_package_2025/object_mask.nefx2.sm51.erp";
    let amd = "shader_package_2025/object_mask.nefx2.sm51_amd.erp";
    for rel in [generic, amd] {
        case.stock(rel, &archive(vec![shader.clone()]));
    }
    let livery = |n: usize| format!("eaid://f1_2026_vehicle_package/teams/mercedes/livery_{n}.tif.image");
    let build = |differs: &[u8]| {
        let mut r = vec![shader.clone()];
        r.extend((0..40).map(|n| texture(&livery(n), format!("w17 {n}").as_bytes())));
        r.push(texture(&livery(99), differs));
        archive(r)
    };
    let p = case.import(
        &[(generic, build(b"nvidia")), (amd, build(b"amd"))],
        &serps("Merc26", "1"),
    );
    let game: &dyn GameFiles = &case.game;
    let (c, out) = case.convert(&p, Some(game));
    let (_, ops) = addon_ops(&out);
    let overlays: Vec<&AssetOp> = ops
        .iter()
        .filter(|op| matches!(op, AssetOp::Erp(ErpOp::Overlay { erps, .. }) if erps == &[merge::BOOT_ERP]))
        .collect();
    assert_eq!(overlays.len(), 1);
    let overlay = ErpFile::open(&out.join(overlay_name(0))).unwrap();
    assert_eq!(overlay.erp.resources.len(), 41, "every livery resource, once");
    let kept = overlay.load(overlay.erp.find(&livery(99)).unwrap()).unwrap();
    assert_eq!(kept.fragments[1].data().unwrap(), b"nvidia");
    assert!(reported(
        &c,
        "warning: 1 resource(s) differ between the mod's generic and AMD shader archives"
    ));
}

#[test]
fn archive_changes_become_one_overlay_and_remove_resource_ops() {
    let case = Case::new("livery");
    let uri = |n: &str| format!("eaid://f1_2026_vehicle_package/teams/mclaren/{n}.tif.image");
    let genesis = "eaid://f1_2026_vehicle_package/teams/genesis/a.tif.image";
    let shadow = "eaid://f1_2025_vehicle_package/shared/shadows/materials/f1_vehicle_shadow.material";
    let shader = resource("eaid://shader_package/opaque", "Shader", &[b"code"]);
    case.stock(
        CAR,
        &archive(vec![
            texture(&uri("paint"), b"small"),
            texture(&uri("wheel"), b"same"),
            texture(&uri("old"), b"old"),
        ]),
    );
    case.stock(OPAQUE, &archive(vec![shader.clone()]));
    let p = case.import(
        &[
            (
                CAR,
                archive(vec![
                    texture(&uri("paint"), b"a much bigger texture"),
                    texture(&uri("wheel"), b"same"),
                    texture(&uri("decal"), b"decal"),
                ]),
            ),
            (
                OPAQUE,
                archive(vec![
                    shader,
                    texture(genesis, b"genesis"),
                    resource("eaid://tools_package/default/serps", "Marker", &[b"x"]),
                    resource(shadow, "Material", &[b"shadow"]),
                ]),
            ),
        ],
        &serps("McLaren Papaya", "v2.1"),
    );
    let game: &dyn GameFiles = &case.game;
    let (c, out) = case.convert(&p, Some(game));
    assert_eq!(c.category, Some(Category::Cars));
    assert_eq!(c.kinds, [Kind::Simplified]);
    let (m, got) = check(&case, &p, &out, None);
    assert_eq!(m.version.to_string(), "2.1.0");
    let key = |erp: &str, uri: &str| (erp.to_owned(), uri.to_owned());
    let boot = merge::BOOT_ERP;
    assert_eq!(
        got.resources.keys().cloned().collect::<Vec<_>>(),
        [
            key(CAR, &uri("decal")),
            key(CAR, &uri("old")),
            key(CAR, &uri("paint")),
            key(boot, genesis),
        ]
    );
    assert_eq!(got.resources[&key(CAR, &uri("old"))], None);
    let paint = got.resources[&key(CAR, &uri("paint"))].as_ref().unwrap();
    assert_eq!(paint.2[1].3, b"a much bigger texture");
    let (_, ops) = addon_ops(&out);
    let overlays: Vec<&AssetOp> = ops
        .iter()
        .filter(|op| matches!(op, AssetOp::Erp(ErpOp::Overlay { .. })))
        .collect();
    assert_eq!(overlays.len(), 2);
    let names = |n: usize| -> Vec<String> {
        let overlay = ErpFile::open(&out.join(overlay_name(n))).unwrap();
        overlay.erp.resources.iter().map(|r| r.name.clone()).collect()
    };
    assert_eq!(names(0), [uri("paint"), uri("decal")]);
    assert_eq!(names(1), [genesis]);
    assert!(reported(&c, "archives: 3 resource(s) → erp.overlay"));
    assert!(reported(&c, "archives: 2 Simplified Studio marker"));
    assert!(c.base, "a Simplified Studio mod needs the SERPs Base Files");
    assert!(reported(&c, "needs: Team Simplified's SERPs Base Files"));
    assert!(reported(&c, "archives: 1 resource(s) removed"));
}

#[test]
fn a_resource_a_mod_carries_twice_keeps_its_last_copy() {
    let case = Case::new("twice");
    let texture_uri = "eaid://f1_2026_vehicle_package/teams/genesis/a.tif.image";
    let glass = "shader_package_2025/object_glass.nefx2.sm51.erp";
    for rel in [glass, OPAQUE] {
        case.stock(
            rel,
            &archive(vec![resource("eaid://shader_package/x", "Shader", &[b"code"])]),
        );
    }
    let p = case.import(
        &[
            (glass, archive(vec![texture(texture_uri, b"first")])),
            (OPAQUE, archive(vec![texture(texture_uri, b"second")])),
        ],
        &serps("Genesis", "1"),
    );
    let game: &dyn GameFiles = &case.game;
    let (_, out) = case.convert(&p, Some(game));
    let (_, got) = check(&case, &p, &out, None);
    let r = got.resources[&(merge::BOOT_ERP.to_owned(), texture_uri.to_owned())]
        .as_ref()
        .unwrap();
    assert_eq!(r.2[1].3, b"second");
    let overlay = ErpFile::open(&out.join(overlay_name(0))).unwrap();
    assert_eq!(overlay.erp.resources.len(), 1);
}

#[test]
fn each_archive_of_a_mod_keeps_its_own_copies() {
    let case = Case::new("split");
    let logo = "eaid://environment_package/core_assets/shared_textures/myteam/logo.tif.image";
    let same = "eaid://f1_2025_vehicle_package/shared/same.tif.image";
    let common = "2025_asset_groups/f1_2025_vehicle_package/teams/common.erp";
    let fom = "2025_asset_groups/f1_2025_vehicle_package/teams/common_fom.erp";
    let kept = "eaid://environment_package/core_assets/shared_textures/myteam/kept.tif.image";
    let both =
        |a: &[u8], b: &[u8], c: &[u8]| archive(vec![texture(logo, a), texture(same, b), texture(kept, c)]);
    for rel in [common, fom] {
        case.stock(rel, &both(b"stock", b"stock", b"stock"));
    }
    let p = case.import(
        &[
            (common, both(b"in common", b"new", b"changed")),
            (fom, both(b"in fom", b"new", b"stock")),
        ],
        &serps("Logos", "1"),
    );
    let game: &dyn GameFiles = &case.game;
    let (c, out) = case.convert(&p, Some(game));
    let (_, ops) = addon_ops(&out);
    let mut targets: Vec<(Vec<String>, Vec<String>)> = ops
        .iter()
        .filter_map(|op| match op {
            AssetOp::Erp(ErpOp::Overlay { erps, source }) => {
                let f = ErpFile::open(&source.resolve(&out).unwrap()).unwrap();
                Some((
                    erps.clone(),
                    f.erp.resources.iter().map(|r| r.name.clone()).collect(),
                ))
            }
            _ => None,
        })
        .collect();
    targets.sort();
    assert_eq!(
        targets,
        [
            (
                vec![common.to_owned()],
                vec![logo.to_owned(), same.to_owned(), kept.to_owned()]
            ),
            (vec![fom.to_owned()], vec![logo.to_owned(), same.to_owned()]),
        ]
    );
    assert!(reported(&c, "archives: 5 resource(s) → erp.overlay"));
}

#[test]
fn team_logo_arrays_ship_merged_with_the_game_s_arrays() {
    let case = Case::new("logos");
    let image = format!("{LOGO_ARRAY}.image");
    let shader = resource("eaid://shader_package/ui", "Shader", &[b"code"]);
    case.stock(
        FLOW_BOOT,
        &archive(vec![surface(&image, &[[1; 4], [2; 4], [3; 4]])]),
    );
    case.stock(UI_TEXTURE, &archive(vec![shader.clone()]));
    let p = case.import(
        &[(
            UI_TEXTURE,
            archive(vec![
                shader,
                surface(&image, &[[1; 4], [9; 4], [3; 4]]),
                resource(LOGO_ARRAY, "UITextureArray", &[b"array"]),
            ]),
        )],
        &serps("Team Logos", "3"),
    );
    let game: &dyn GameFiles = &case.game;
    let (c, out) = case.convert(&p, Some(game));
    assert_eq!(c.category, None);
    let (_, got) = check(&case, &p, &out, None);
    let merged = got.resources[&(merge::BOOT_ERP.to_owned(), image.clone())]
        .as_ref()
        .unwrap();
    assert_eq!(merged.2[1].3, [[1; 4], [9; 4], [3; 4]].concat());
    assert!(
        got.resources
            .contains_key(&(merge::BOOT_ERP.to_owned(), LOGO_ARRAY.to_owned()))
    );
    assert!(out.join(LOGOS).is_file());
    let (_, ops) = addon_ops(&out);
    let logo_ops: Vec<&AssetOp> = ops
        .iter()
        .filter(|op| {
            matches!(
                op,
                AssetOp::Erp(ErpOp::MergeSlices { .. } | ErpOp::PutResource { rename: None, .. })
            )
        })
        .collect();
    assert_eq!(
        logo_ops,
        [
            &AssetOp::Erp(ErpOp::MergeSlices {
                erp: merge::BOOT_ERP.into(),
                resource: image.clone(),
                source: Source::addon(LOGOS),
            }),
            &AssetOp::Erp(ErpOp::PutResource {
                erp: merge::BOOT_ERP.into(),
                resource: LOGO_ARRAY.into(),
                source: Source::addon(LOGOS),
                rename: None,
            }),
        ]
    );
    let shipped = ErpFile::open(&out.join(LOGOS)).unwrap();
    let theirs = shipped.load(shipped.erp.find(&image).unwrap()).unwrap();
    assert_eq!(
        theirs.fragments[1].data().unwrap(),
        [[1; 4], [9; 4], [3; 4]].concat(),
        "the addon ships the mod's array; the launcher merges it"
    );
    assert!(reported(
        &c,
        "logos: 1 team logo texture array(s) → erp.merge-slices"
    ));
    assert!(reported(&c, "logos: 1 texture array view(s) → put-resource"));
    assert!(!c.report.iter().any(|l| l.starts_with("warning:")));
}

#[test]
fn loose_files_become_file_puts_and_programs_are_never_copied() {
    let case = Case::new("files");
    case.stock("audio/pc/sfx.bnk", b"stock");
    let mip = "f1_safetycar_package/textures/car.tif.mipmaps";
    let p = case.import(
        &[
            ("audio/pc/sfx.bnk", b"louder".to_vec()),
            ("audio/pc/extra.bnk", b"extra".to_vec()),
            (mip, b"mips".to_vec()),
            ("bin/hook.dll", b"MZ".to_vec()),
            ("EAAntiCheat.cfg", b"cfg".to_vec()),
        ],
        &serps("Engine Sounds", "1.0.141.7865"),
    );
    let game: &dyn GameFiles = &case.game;
    let (c, out) = case.convert(&p, Some(game));
    assert_eq!(c.category, None);
    assert_eq!(c.kinds, [Kind::Files]);
    let (m, got) = check(&case, &p, &out, None);
    assert_eq!(m.version.to_string(), "1.0.141+7865");
    assert_eq!(
        got.files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["audio/pc/extra.bnk", "audio/pc/sfx.bnk", mip]
    );
    assert_eq!(got.files["audio/pc/sfx.bnk"], b"louder");
    let (_, ops) = addon_ops(&out);
    assert!(
        ops.iter()
            .any(|op| matches!(op, AssetOp::Erp(ErpOp::AddMipmaps { target, .. }) if target == mip))
    );
    let shipped = tree(&out);
    assert!(
        !shipped
            .keys()
            .any(|k| k.ends_with(".dll") || k.contains("EAAntiCheat"))
    );
    for skipped in ["bin/hook.dll", "EAAntiCheat.cfg"] {
        assert!(reported(&c, &format!("skipped {skipped}")));
    }
    assert!(reported(&c, "files: 1 replaced and 2 added"));
}

#[test]
fn a_track_package_becomes_track_add_with_the_package_inside() {
    let case = Case::new("track");
    let src = case.root.join("src").join("Test Ring");
    write(
        &src.join("manifest.json"),
        br#"{"track_code":"test_ring","track_name":"Test Ring","version":"2","author_name":"Venturi"}"#,
    );
    write(&src.join("test_ring.erp"), b"track archive");
    write(&src.join("mipmaps/a.mipmaps"), b"mips");
    let p = case.library().import_track(&src, &mut |_, _| {}).unwrap();
    let (c, out) = case.convert(&p, None);
    assert_eq!(
        (c.id.as_str(), c.category, c.kinds.as_slice()),
        (
            "mod.test_ring",
            Some(Category::Tracks),
            &[Kind::Database, Kind::Erp][..]
        )
    );
    let (m, got) = check(&case, &p, &out, None);
    assert_eq!(
        (m.version.to_string().as_str(), m.author.as_str()),
        ("2.0.0", "Venturi")
    );
    assert_eq!(got.tracks.len(), 1);
    assert_eq!(got.tracks[0]["test_ring.erp"], b"track archive");
    assert!(m.compat.game_builds.is_empty());
    assert_eq!(m.operations.assets, [TRACK_OPS]);
}

#[test]
fn a_mod_needs_the_game_and_an_empty_folder() {
    let case = Case::new("refused");
    case.stock("audio/pc/sfx.bnk", b"stock");
    let p = case.import(&[("audio/pc/sfx.bnk", b"louder".to_vec())], &serps("Sounds", "1"));
    let out = case.root.join("addon");
    assert!(convert(&case.library(), &p, None, &out, &mut |_, _| {}).is_err());
    assert!(!out.exists());
    write(&out.join("x"), b"x");
    let game: &dyn GameFiles = &case.game;
    assert!(convert(&case.library(), &p, Some(game), &out, &mut |_, _| {}).is_err());
    assert!(out.join("x").is_file());
}

#[test]
fn versions_become_semver_and_ids_get_a_namespace() {
    for (from, to) in [
        (None, "1.0.0"),
        (Some("1.0"), "1.0.0"),
        (Some("v2.3.4"), "2.3.4"),
        (Some("1.0.0-beta.1"), "1.0.0-beta.1"),
        (Some("1.0.141.7865"), "1.0.141+7865"),
        (Some("2025 R3"), "2025.3.0"),
        (Some("final"), "1.0.0"),
    ] {
        assert_eq!(version(from), to, "{from:?}");
    }
    let mut p = Package {
        id: "Okayama_GP".into(),
        name: "Okayama".into(),
        author: None,
        version: None,
        game_version: None,
        studio_version: None,
        source: "okayama.zip".into(),
        sha256: None,
        source_size: None,
        imported: 0,
        files: Vec::new(),
        skipped: Vec::new(),
        track: Some("Okayama_GP".into()),
        part: None,
    };
    assert_eq!(addon_id(&p), "mod.okayama_gp");
    p.id = "rm-designs.scuderia-alphatauri-orlen.tyres".into();
    assert_eq!(addon_id(&p), "mod.rm-designs.scuderia-alphatauri-orlen.tyres");
    p.id = "pirelli-tyres-collection-v1-0-6.pirelli-tyres-collection-v1-0-6-default-weather-grip-track-pirelli-season-2011".into();
    let long = addon_id(&p);
    assert!(long.len() <= 64, "{long}");
    assert!(long.starts_with("mod.pirelli-tyres-collection-v1-0-6."), "{long}");
    assert_eq!(addon_id(&p), long);
    p.id.push('2');
    assert_ne!(addon_id(&p), long);
}

#[test]
fn a_track_zip_keeps_its_sha256() {
    let case = Case::new("track-zip");
    let zip = case.root.join("Test Ring.zip");
    std::fs::create_dir_all(&case.root).unwrap();
    let mut w = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
    for (name, data) in [
        (
            "manifest.json",
            &br#"{"track_code":"test_ring","track_name":"Test Ring"}"#[..],
        ),
        ("test_ring.erp", b"track archive"),
    ] {
        w.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut w, data).unwrap();
    }
    w.finish().unwrap();
    let p = case.library().import_track(&zip, &mut |_, _| {}).unwrap();
    let want: String = sha2::Sha256::digest(std::fs::read(&zip).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(p.sha256.as_deref(), Some(want.as_str()));
    let (_, out) = case.convert(&p, None);
    let m = AddonManifest::load(&out).unwrap();
    assert_eq!(m.origin.unwrap().sha256.as_deref(), Some(want.as_str()));
}

#[test]
fn a_real_database_round_trips_through_the_addon() {
    let Some(dir) = std::env::var_os("F1_25_DIR").map(PathBuf::from) else {
        eprintln!("F1_25_DIR is not set; skipped");
        return;
    };
    let stock_path = dir.join(DATABASE);
    let mut case = Case::new("real-db");
    case.game.database = Some(stock_path.clone());
    let stock = Gvdb::parse(&std::fs::read(&stock_path).unwrap()).unwrap();
    let mut ours = stock.clone();
    let strings = ours
        .records
        .iter_mut()
        .filter(|r| r.value_type() == ValueType::String && !r.value.is_empty());
    let mut changed = 0;
    for r in strings.step_by(997).take(25) {
        r.value.extend_from_slice(b"_x");
        changed += 1;
    }
    let removed = ours.records.remove(ours.records.len() / 2).key;
    ours.records.push(Record::new("swofty.convert.test", 2, "added"));
    let p = case.import(&[(DATABASE, ours.to_bytes())], &serps("Real Database", "1.2"));
    let game: &dyn GameFiles = &case.game;
    let (c, out) = case.convert(&p, Some(game));
    assert_eq!(c.category, None);
    let (_, got) = check(&case, &p, &out, Some(&stock));
    assert_eq!(sorted(got.db.as_deref().unwrap()), sorted(&ours.to_bytes()));
    let (_, ops) = addon_ops(&out);
    let remove = format!("0x{removed:016x}");
    assert!(
        ops.iter()
            .any(|op| matches!(op, AssetOp::Db(DbOp::RemoveRecord { key, .. }) if *key == remove))
    );
    assert!(reported(
        &c,
        &format!("database: {changed} changed, 1 added, 1 removed")
    ));
}

fn installed() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("F1_25_GAME")?);
    dir.join("F1_25.exe").is_file().then_some(dir)
}

#[test]
fn a_mod_of_a_real_archive_round_trips_through_an_overlay_on_it() {
    let Some(game_dir) = installed() else {
        eprintln!("F1_25_GAME is not set; skipped");
        return;
    };
    let rel = "2025_asset_groups/ui_package/flow_steering_screen.erp";
    let mut case = Case::new("real-erp");
    case.game.installed = Some(game_dir.clone());
    let mut erp = Erp::parse(&std::fs::read(game_dir.join(rel)).unwrap()).unwrap();
    assert!(erp.resources.len() >= 3, "{rel} has too few resources");
    let changed = erp.resources[0].name.clone();
    let mut data = erp.resources[0].fragments[0].data().unwrap();
    data.extend_from_slice(b"swofty");
    erp.resources[0].set_fragment(0, &data).unwrap();
    let mut copy = erp.resources[1].clone();
    copy.name = "eaid://ui_package/swofty/convert_test.tif".into();
    erp.resources.push(copy);
    let removed = erp.resources.remove(erp.resources.len() - 2).name;
    let p = case.import(&[(rel, erp.to_bytes())], &serps("Steering Screen", "1"));
    let game: &dyn GameFiles = &case.game;
    let (c, out) = case.convert(&p, Some(game));
    let (_, got) = check(&case, &p, &out, None);
    let own = rel.to_owned();
    assert!(got.resources[&(own.clone(), changed.clone())].is_some());
    assert!(got.resources[&(own, "eaid://ui_package/swofty/convert_test.tif".into())].is_some());
    assert_eq!(got.resources[&(rel.to_owned(), removed)], None);
    let overlay = Erp::parse(&std::fs::read(out.join(overlay_name(0))).unwrap()).unwrap();
    assert_eq!(overlay.resources.len(), 2);
    assert_eq!(
        res(overlay.find(&changed).unwrap()),
        res(erp.find(&changed).unwrap())
    );
    assert!(reported(&c, "archives: 2 resource(s) → erp.overlay"));
}

#[test]
fn a_real_team_logo_array_merges_one_slice_into_the_game_s() {
    let Some(game_dir) = installed() else {
        eprintln!("F1_25_GAME is not set; skipped");
        return;
    };
    let stock = [
        "2025_asset_groups/ui_package/common_front_end.erp",
        "2025_asset_groups/ui_package/flow_frontend.erp",
    ];
    let found = stock.iter().find_map(|rel| {
        let file = ErpFile::open(&game_dir.join(rel)).ok()?;
        let r = file
            .erp
            .resources
            .iter()
            .find(|r| r.name.contains("team_logos") && r.name.ends_with(".texarray.image"))?;
        Some(file.load(r).unwrap())
    });
    let Some(logos) = found else {
        eprintln!("no team logo array in the game's ui archives; skipped");
        return;
    };
    let mut case = Case::new("real-logos");
    case.game.installed = Some(game_dir.clone());
    let head = logos.fragments[0].data().unwrap();
    let count = u32::from_le_bytes(head[28..32].try_into().unwrap()) as usize;
    let mut pixels = logos.fragments[1].data().unwrap();
    let slice = pixels.len() / count;
    for b in &mut pixels[slice..2 * slice] {
        *b = !*b;
    }
    let mut ours = logos.clone();
    ours.set_fragment(1, &pixels).unwrap();
    let mut carrier = Erp::parse(&std::fs::read(game_dir.join(UI_TEXTURE)).unwrap()).unwrap();
    carrier.resources.push(ours);
    let p = case.import(&[(UI_TEXTURE, carrier.to_bytes())], &serps("Real Logos", "1"));
    let game: &dyn GameFiles = &case.game;
    let (c, out) = case.convert(&p, Some(game));
    let (_, got) = check(&case, &p, &out, None);
    let merged = ErpFile::open(&out.join(LOGOS)).unwrap();
    let r = merged.load(merged.erp.find(&logos.name).unwrap()).unwrap();
    assert_eq!(r.fragments[1].data().unwrap(), pixels);
    assert!(
        got.resources[&(merge::BOOT_ERP.to_owned(), logos.name.clone())].is_some(),
        "one slice merges into the game's"
    );
    assert!(reported(
        &c,
        "logos: 1 team logo texture array(s) → erp.merge-slices"
    ));
}

#[test]
fn a_simplified_mod_says_what_it_replaces_and_a_copy_and_paste_one_what_it_changes() {
    let mut w = Writer::default();
    w.text.push(AssetOp::Lang(LangOp::Set {
        language: "eng".into(),
        key: "k".into(),
        value: "v".into(),
    }));
    assert_eq!(w.description(Some("1.0")), "1 text entry.");
    w.base = true;
    w.replaces = vec!["the Ferrari team", "the McLaren team"];
    assert_eq!(
        w.description(Some("1.0")),
        "Replaces the Ferrari team and the McLaren team. Made using the Simplified Studio. 1 text entry."
    );
    assert_eq!(
        w.description(None),
        "Replaces the Ferrari team and the McLaren team. 1 text entry."
    );
    assert_eq!(and_list(&["a", "b", "c"]), "a, b and c");
}

const COMMON: &str = "2025_asset_groups/f1_2025_vehicle_package/teams/common.erp";

fn plain(name: &str, byte: u8) -> Resource {
    Resource {
        hash: [0; 16],
        ..texture(name, &[byte; 4])
    }
}

#[test]
fn a_whole_archive_from_a_game_without_it_becomes_an_overlay_once_the_game_has_it() {
    let case = Case::new("rediff");
    let shipped = archive(vec![
        plain("eaid://teams/a", 1),
        plain("eaid://teams/b", 2),
        plain("eaid://teams/c", 3),
    ]);
    let p = case.import(&[(COMMON, shipped.clone())], &serps("Cars", "1.0.140.0000"));
    let game: &dyn GameFiles = &case.game;
    let (_, out) = case.convert(&p, Some(game));
    let (m, ops) = addon_ops(&out);
    assert_eq!(m.kind, [Kind::Files]);
    assert!(
        ops.iter()
            .any(|op| matches!(op, AssetOp::File(FileOp::Put { target, .. }) if target == COMMON))
    );
    assert!(rediff(&out, game).unwrap().is_empty());

    let stock = archive(vec![plain("eaid://teams/a", 9), plain("eaid://teams/b", 2)]);
    case.stock(COMMON, &stock);
    let lines = rediff(&out, game).unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with(&format!("{COMMON}: the game has it again")));
    let applied = |case: &Case, out: &Path| {
        let (m, ops) = addon_ops(out);
        let game: &dyn GameFiles = &case.game;
        (m, apply(&ops, None, game, &|s| case.resolve(s, out)))
    };
    let fresh = Case::new("rediff-fresh");
    fresh.stock(COMMON, &stock);
    let fresh_p = fresh.import(&[(COMMON, shipped)], &serps("Cars", "1.0.140.0000"));
    let (_, fresh_out) = fresh.convert(&fresh_p, Some(&fresh.game as &dyn GameFiles));
    let (m, got) = applied(&case, &out);
    assert_eq!(got, applied(&fresh, &fresh_out).1);
    assert_eq!(m.kind, [Kind::Files]);
    assert_eq!(m.operations.erp, [ERP_OPS]);
    assert!(m.operations.assets.is_empty());
    assert!(got.files.is_empty());
    let uris: Vec<&str> = got.resources.keys().map(|(_, uri)| uri.as_str()).collect();
    assert_eq!(uris, ["eaid://teams/a", "eaid://teams/c"]);
    assert!(!out.join(FILES_DIR).join(COMMON).exists());
    assert!(!out.join(FILE_OPS).exists());
    assert!(
        std::fs::read_to_string(out.join(REPORT))
            .unwrap()
            .contains("go in as an overlay instead of the whole archive")
    );
    assert!(rediff(&out, game).unwrap().is_empty());
}

#[test]
fn a_game_archive_that_is_the_mod_s_own_copy_keeps_the_whole_archive() {
    let case = Case::new("rediff-own");
    let shipped = archive(vec![plain("eaid://teams/a", 1)]);
    let p = case.import(&[(COMMON, shipped.clone())], &serps("Cars", "1.0.140.0000"));
    let game: &dyn GameFiles = &case.game;
    let (_, out) = case.convert(&p, Some(game));
    case.stock(COMMON, &shipped);
    let before = tree(&out);
    assert!(rediff(&out, game).unwrap().is_empty());
    assert_eq!(tree(&out), before);
}
