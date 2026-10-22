use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use swofty_addon::launcher::GameFiles;
use swofty_formats::erp::{ErpFile, Resource};
use swofty_formats::gvdb::{Gvdb, Record, ValueType};
use swofty_formats::lng::{Entry, Lng};

use crate::library::Package;
use crate::package::{self, Class};
use crate::{Result, err};

pub const FALLBACK: &str = "eng";
pub const VERSION: u32 = 5;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    pub build_key: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub records: Records,
    #[serde(default)]
    pub archives: Vec<Archive>,
    #[serde(default)]
    pub text: Vec<Text>,
    #[serde(default)]
    pub files: Vec<FileChange>,
    #[serde(default)]
    pub dropped: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Records {
    pub changed: Vec<u64>,
    pub added: Vec<u64>,
    pub removed: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Archive {
    pub path: String,
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Text {
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub changed: BTreeMap<String, String>,
    pub added: BTreeMap<String, String>,
    // Added keys whose text is the game's own: a mod's database points a record
    // at a key of its own, and fills in the text of the key the record pointed at
    // before. Other mods' renames of the same key win over these.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub class: Class,
}

impl Records {
    pub fn len(&self) -> usize {
        self.changed.len() + self.added.len() + self.removed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Archive {
    pub fn len(&self) -> usize {
        self.changed.len() + self.added.len() + self.removed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Text {
    pub fn len(&self) -> usize {
        self.changed.len() + self.added.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0 && self.defaults.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &String)> {
        self.changed.iter().chain(&self.added)
    }
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.records.is_empty() && self.archives.is_empty() && self.text.is_empty() && self.files.is_empty()
    }
}

pub fn compute(p: &Package, files: &Path, game: &dyn GameFiles) -> Result<(Diff, Gvdb)> {
    let mut diff = Diff {
        build_key: game.build_key(),
        version: VERSION,
        ..Diff::default()
    };
    let mut records = Gvdb::default();
    let mut names = HashMap::new();
    let mut texts = Vec::new();
    let removals = p.game_version.is_some() && p.game_version == game.exe_version();
    for f in &p.files {
        let path = files.join(&f.path);
        match f.class {
            Class::Database => {
                let (r, values, dropped, pointed) = database(&path, game, removals)?;
                diff.dropped += dropped;
                diff.records = r;
                records = values;
                names = pointed;
            }
            Class::Text => texts.push(f.path.as_str()),
            _ => match package::class(&f.path, Some(game)) {
                Some(Class::Archive) => {
                    let (a, dropped) = archive(&f.path, &path, game, removals)?;
                    diff.dropped += dropped;
                    if !a.is_empty() {
                        diff.archives.push(a);
                    }
                }
                Some(class) => diff.files.push(FileChange {
                    path: f.path.clone(),
                    class,
                }),
                None => {}
            },
        }
    }
    diff.text = text(&texts, files, game, &names)?;
    Ok((diff, records))
}

fn read(path: &Path) -> Result<Vec<u8>> {
    package::read_file(path)
}

type Database = (Records, Gvdb, usize, HashMap<String, String>);

// Besides the records, the text keys the mod points string records at, each with
// the key the game's own record points at. A stock record the mod's database lacks
// is removed only when the mod was made for this game version: a database exported
// from an older version lacks every record later updates added, and the game
// crashes on boot when it reads one of them.
fn database(path: &Path, game: &dyn GameFiles, removals: bool) -> Result<Database> {
    let stock = Gvdb::parse(&read(&game.original(package::DATABASE)?)?)
        .map_err(|e| err(format!("the game's database: {e}")))?;
    let ours = Gvdb::parse(&read(path)?).map_err(|e| err(format!("{}: {e}", path.display())))?;
    let index: HashMap<u64, &Record> = stock.records.iter().map(|r| (r.key, r)).collect();
    let mut out = Records::default();
    let mut values = Gvdb::default();
    let mut seen = HashSet::with_capacity(ours.records.len());
    let mut pointed = HashMap::new();
    for r in &ours.records {
        seen.insert(r.key);
        match index.get(&r.key) {
            Some(s) if s.tag == r.tag && s.value == r.value => continue,
            Some(s) => {
                out.changed.push(r.key);
                if s.value_type() == ValueType::String && r.value_type() == ValueType::String {
                    pointed.insert(r.text().into_owned(), s.text().into_owned());
                }
            }
            None => out.added.push(r.key),
        }
        values.records.push(r.clone());
    }
    let removed: Vec<u64> = stock
        .records
        .iter()
        .map(|r| r.key)
        .filter(|k| !seen.contains(k))
        .collect();
    let dropped = match removals {
        true => {
            out.removed = removed;
            0
        }
        false => removed.len(),
    };
    Ok((out, values, dropped, pointed))
}

fn layout_same(s: &Resource, r: &Resource) -> bool {
    s.kind == r.kind
        && s.unk == r.unk
        && s.fragments.len() == r.fragments.len()
        && s.fragments
            .iter()
            .zip(&r.fragments)
            .all(|(a, b)| a.name == b.name && a.flags == b.flags && a.size == b.size)
}

// Mod tools rewrite a resource and keep the game's digest in the index, so a
// digest only counts when the layout matches too.
fn same(stock: &ErpFile, s: &Resource, ours: &ErpFile, r: &Resource) -> swofty_formats::Result<bool> {
    let packed = |x: &Resource| x.fragments.iter().map(|f| f.packed).collect::<Vec<_>>();
    if !layout_same(s, r) || packed(s) != packed(r) {
        return Ok(false);
    }
    if s.hash != [0; 16] && r.hash != [0; 16] {
        return Ok(s.hash == r.hash);
    }
    Ok(stock.load(s)?.content_hash()? == ours.load(r)?.content_hash()?)
}

// A stock resource the mod's archive lacks is removed when the mod was made for this
// game version, as the archive replaces the game's whole. Shader archives are merged
// into the game's instead, so they remove nothing.
fn archive(rel: &str, path: &Path, game: &dyn GameFiles, removals: bool) -> Result<(Archive, usize)> {
    let fail = |e: swofty_formats::Error| err(format!("{rel}: {e}"));
    let stock = ErpFile::open(&game.original(rel)?).map_err(&fail)?;
    let ours = ErpFile::open(path).map_err(&fail)?;
    let index: HashMap<&str, &Resource> = stock.erp.resources.iter().map(|r| (r.name.as_str(), r)).collect();
    let mut out = Archive {
        path: rel.to_owned(),
        changed: Vec::new(),
        added: Vec::new(),
        removed: Vec::new(),
    };
    for r in &ours.erp.resources {
        match index.get(r.name.as_str()) {
            Some(s) if same(&stock, s, &ours, r).map_err(&fail)? => {}
            Some(_) => out.changed.push(r.name.clone()),
            None => out.added.push(r.name.clone()),
        }
    }
    let mut dropped = 0;
    if !crate::merge::carrier(rel) {
        let names: HashSet<&str> = ours.erp.resources.iter().map(|r| r.name.as_str()).collect();
        let removed: Vec<String> = stock
            .erp
            .resources
            .iter()
            .filter(|r| !names.contains(r.name.as_str()))
            .map(|r| r.name.clone())
            .collect();
        match removals {
            true => out.removed = removed,
            false => dropped = removed.len(),
        }
    }
    Ok((out, dropped))
}

pub fn language(path: &str) -> Option<&str> {
    path.rsplit('/')
        .next()?
        .strip_prefix("language_")?
        .strip_suffix(".lng")
}

fn lng(path: &Path) -> Result<Lng> {
    Lng::parse(&read(path)?).map_err(|e| err(format!("{}: {e}", path.display())))
}

fn text(
    paths: &[&str],
    files: &Path,
    game: &dyn GameFiles,
    names: &HashMap<String, String>,
) -> Result<Vec<Text>> {
    let mut out: Vec<Text> = Vec::new();
    let mut stock_dir = None;
    for rel in paths {
        let Some(code) = language(rel) else { continue };
        let Ok(stock) = game.original_lang(code) else {
            continue;
        };
        if stock_dir.is_none() {
            stock_dir = stock.parent().map(Path::to_path_buf);
        }
        let d = lng(&stock)?.diff(&lng(&files.join(rel))?);
        let map = |e: &[Entry]| e.iter().map(|e| (e.key.clone(), e.value.clone())).collect();
        out.push(Text {
            language: code.to_owned(),
            from: None,
            changed: map(&d.changed),
            added: map(&d.added),
            defaults: BTreeMap::new(),
        });
    }
    let Some(source) = out
        .iter()
        .find(|t| t.language == FALLBACK)
        .or_else(|| out.iter().min_by(|a, b| a.language.cmp(&b.language)))
        .cloned()
    else {
        return Ok(out);
    };
    let have: HashSet<String> = out.iter().map(|t| t.language.clone()).collect();
    let mut stock: Vec<String> = stock_dir
        .and_then(|d| std::fs::read_dir(d).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| language(&e.file_name().to_string_lossy()).map(str::to_owned))
        .filter(|code| !have.contains(code))
        .collect();
    stock.sort();
    for code in stock {
        out.push(Text {
            language: code,
            from: Some(source.language.clone()),
            ..source.clone()
        });
    }
    defaults(&mut out, game, names)?;
    out.retain(|t| !t.is_empty());
    out.sort_by(|a, b| a.language.cmp(&b.language));
    Ok(out)
}

fn defaults(out: &mut [Text], game: &dyn GameFiles, names: &HashMap<String, String>) -> Result<()> {
    let mut stock: HashMap<String, Option<Lng>> = HashMap::new();
    let mut lookup = |code: &str, key: &str| -> Result<Option<String>> {
        if !stock.contains_key(code) {
            let l = match game.original_lang(code) {
                Ok(path) => Some(lng(&path)?),
                Err(_) => None,
            };
            stock.insert(code.to_owned(), l);
        }
        Ok(stock[code].as_ref().and_then(|l| l.get(key)).map(str::to_owned))
    };
    for t in out.iter_mut() {
        // A translation replaced with the game's English text is untranslated
        // filler: it applies, as in the Simplified Launcher, unless an addon sets
        // the key.
        if t.language != FALLBACK && t.from.is_none() {
            let keys: Vec<String> = t.changed.keys().cloned().collect();
            for key in keys {
                if lookup(FALLBACK, &key)?.as_deref() == Some(t.changed[&key].as_str()) {
                    let value = t.changed.remove(&key).expect("listed above");
                    t.defaults.insert(key, value);
                }
            }
        }
        let source = t.from.clone().unwrap_or_else(|| t.language.clone());
        let keys: Vec<String> = t
            .added
            .keys()
            .filter(|k| names.contains_key(*k))
            .cloned()
            .collect();
        for key in keys {
            let was = &names[&key];
            // Mods fill in untranslated names too, so the English text counts.
            let value = Some(t.added[&key].clone());
            if lookup(&source, was)? != value && lookup(FALLBACK, was)? != value {
                continue;
            }
            let value = t.added.remove(&key).expect("listed above");
            let own = lookup(&t.language, was)?.unwrap_or(value);
            t.defaults.insert(key, own);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::library::File;

    struct Game {
        dir: PathBuf,
        exe: &'static str,
    }

    impl GameFiles for Game {
        fn build_key(&self) -> String {
            "k".into()
        }

        fn exe_version(&self) -> Option<String> {
            Some(self.exe.into())
        }

        fn is_stock(&self, rel: &str) -> bool {
            self.dir.join(rel).is_file()
        }

        fn original(&self, rel: &str) -> Result<PathBuf> {
            Ok(self.dir.join(rel))
        }

        fn original_lang(&self, code: &str) -> Result<PathBuf> {
            Ok(self.dir.join(format!("lang/language_{code}.lng")))
        }
    }

    fn texture(name: &str, pixels: &[u8], hash: [u8; 16]) -> Resource {
        use swofty_formats::erp::Fragment;
        let frag = |data: &[u8]| Fragment {
            name: *b"temp",
            offset: 0,
            size: data.len() as u64,
            flags: 0x10,
            compression: 0,
            packed: data.len() as u64,
            raw: data.to_vec(),
        };
        let mut kind = [0u8; 16];
        kind[..13].copy_from_slice(b"GfxSurfaceRes");
        Resource {
            name: name.into(),
            kind,
            unk: vec![0; 4],
            fragments: vec![frag(b"head"), frag(pixels)],
            hash,
        }
    }

    fn archive_of(path: &Path, resources: Vec<Resource>) {
        use swofty_formats::erp::{Erp, HEADER_LEN, MAGIC};
        let mut header = [0u8; HEADER_LEN];
        header[..4].copy_from_slice(MAGIC);
        header[4..8].copy_from_slice(&3u32.to_le_bytes());
        let mut erp = Erp {
            header,
            version: 3,
            resources,
        };
        write(path, &erp.to_bytes());
    }

    #[test]
    fn a_resource_rewritten_under_the_game_s_digest_is_a_change() {
        let root = std::env::temp_dir().join(format!("simplified-relayout-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let (stock, ours) = (root.join("game"), root.join("files"));
        let rel = "2025_asset_groups/f1_2026_vehicle_package/teams/mclaren/wep/mclaren.erp";
        let digest = [7; 16];
        archive_of(
            &stock.join(rel),
            vec![
                texture("paint", b"small", digest),
                texture("wheel", b"same", digest),
            ],
        );
        archive_of(
            &ours.join(rel),
            vec![
                texture("paint", b"a much bigger texture", digest),
                texture("wheel", b"same", digest),
            ],
        );
        let game = Game {
            dir: stock,
            exe: "1.0",
        };
        let (a, _) = archive(rel, &ours.join(rel), &game, true).unwrap();
        assert_eq!(a.changed, ["paint"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn shader_archives_remove_nothing_and_others_remove_what_they_lack() {
        let root = std::env::temp_dir().join(format!("simplified-removals-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let (stock, ours) = (root.join("game"), root.join("files"));
        let shader = "shader_package_2025/object_opaque.nefx2.sm51.erp";
        let car = "2025_asset_groups/f1_2026_vehicle_package/teams/mclaren/wep/mclaren.erp";
        for rel in [shader, car] {
            archive_of(
                &stock.join(rel),
                vec![texture("kept", b"a", [1; 16]), texture("gone", b"b", [2; 16])],
            );
            archive_of(&ours.join(rel), vec![texture("kept", b"a", [1; 16])]);
        }
        let game = Game {
            dir: stock,
            exe: "1.0",
        };
        assert!(
            archive(shader, &ours.join(shader), &game, true)
                .unwrap()
                .0
                .removed
                .is_empty()
        );
        assert_eq!(
            archive(car, &ours.join(car), &game, true).unwrap().0.removed,
            ["gone"]
        );
        assert_eq!(
            archive(car, &ours.join(car), &game, false).unwrap(),
            (
                Archive {
                    path: car.into(),
                    changed: Vec::new(),
                    added: Vec::new(),
                    removed: Vec::new(),
                },
                1
            )
        );
        std::fs::remove_dir_all(&root).ok();
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

    #[test]
    fn text_a_mod_fills_in_from_the_game_is_a_default() {
        let root = std::env::temp_dir().join(format!("simplified-defaults-{}", std::process::id()));
        let (stock, ours) = (root.join("game"), root.join("files"));
        let db = |records: &[(&str, &str)]| Gvdb {
            records: records.iter().map(|(k, v)| Record::new(k, 2, *v)).collect(),
        };
        write(
            &stock.join(package::DATABASE),
            &db(&[("ast", "lng_ast"), ("rac", "lng_rac")]).to_bytes(),
        );
        write(
            &ours.join(package::DATABASE),
            &db(&[("ast", "lng_ast_26"), ("rac", "lng_rac_26")]).to_bytes(),
        );
        let names = |code: &str| {
            [
                ("lng_ast", format!("Aston {code}")),
                ("lng_rac", format!("Bulls {code}")),
            ]
        };
        for code in ["eng", "ger", "kor"] {
            let n = names(code);
            write(
                &stock.join(format!("lang/language_{code}.lng")),
                &lng(&[(n[0].0, &n[0].1), (n[1].0, &n[1].1)]),
            );
        }
        write(
            &ours.join("localisation/2025_release/language_eng.lng"),
            &lng(&[
                ("lng_ast", "Aston eng"),
                ("lng_rac", "Bulls eng"),
                ("lng_ast_26", "Aston eng"),
                ("lng_rac_26", "Cupra"),
            ]),
        );
        write(
            &ours.join("localisation/2025_release/language_ger.lng"),
            &lng(&[
                ("lng_ast", "Aston ger"),
                ("lng_rac", "Bulls eng"),
                ("lng_ast_26", "Aston eng"),
                ("lng_rac_26", "Cupra DE"),
            ]),
        );
        let p = Package {
            id: "m".into(),
            name: "M".into(),
            author: None,
            version: None,
            game_version: None,
            studio_version: None,
            source: "m.zip".into(),
            sha256: None,
            source_size: None,
            imported: 0,
            files: vec![
                File {
                    path: package::DATABASE.into(),
                    class: Class::Database,
                    size: 0,
                },
                File {
                    path: "localisation/2025_release/language_eng.lng".into(),
                    class: Class::Text,
                    size: 0,
                },
                File {
                    path: "localisation/2025_release/language_ger.lng".into(),
                    class: Class::Text,
                    size: 0,
                },
            ],
            skipped: Vec::new(),
            track: None,
            part: None,
        };
        let game = Game {
            dir: stock,
            exe: "1.0",
        };
        let (d, _) = compute(&p, &ours, &game).unwrap();
        let eng = &d.text[0];
        assert_eq!(eng.language, "eng");
        assert_eq!(
            eng.added,
            BTreeMap::from([("lng_rac_26".to_owned(), "Cupra".to_owned())])
        );
        assert_eq!(
            eng.defaults,
            BTreeMap::from([("lng_ast_26".to_owned(), "Aston eng".to_owned())])
        );
        let ger = &d.text[1];
        assert_eq!((ger.language.as_str(), ger.from.as_deref()), ("ger", None));
        assert!(ger.changed.is_empty());
        assert_eq!(
            ger.added,
            BTreeMap::from([("lng_rac_26".to_owned(), "Cupra DE".to_owned())])
        );
        assert_eq!(
            ger.defaults,
            BTreeMap::from([
                ("lng_ast_26".to_owned(), "Aston ger".to_owned()),
                ("lng_rac".to_owned(), "Bulls eng".to_owned()),
            ])
        );
        let kor = &d.text[2];
        assert_eq!((kor.language.as_str(), kor.from.as_deref()), ("kor", Some("eng")));
        assert_eq!(
            kor.defaults,
            BTreeMap::from([("lng_ast_26".to_owned(), "Aston kor".to_owned())])
        );
        let merged = crate::merge::ops("m", &d, &Gvdb::default());
        assert_eq!((merged.strings, merged.defaults.len()), (3, 4));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn diffs_records_and_text_and_keeps_removals_whatever_the_version() {
        let root = std::env::temp_dir().join(format!("simplified-diff-{}", std::process::id()));
        let (stock, ours) = (root.join("game"), root.join("files"));
        let db = |records: &[(&str, &str)]| Gvdb {
            records: records.iter().map(|(k, v)| Record::new(k, 2, *v)).collect(),
        };
        write(
            &stock.join(package::DATABASE),
            &db(&[("a", "1"), ("b", "2"), ("c", "3")]).to_bytes(),
        );
        write(
            &ours.join(package::DATABASE),
            &db(&[("a", "1"), ("b", "9"), ("d", "4")]).to_bytes(),
        );
        for code in ["eng", "ger", "kor"] {
            write(
                &stock.join(format!("lang/language_{code}.lng")),
                &lng(&[("x", code)]),
            );
        }
        let text = |code: &str| format!("localisation/2025_release/language_{code}.lng");
        write(&ours.join(text("eng")), &lng(&[("x", "eng"), ("y", "new")]));
        write(&ours.join(text("ger")), &lng(&[("x", "neu")]));
        let file = |path: String, class| File { path, class, size: 0 };
        let p = Package {
            id: "m".into(),
            name: "M".into(),
            author: None,
            version: None,
            game_version: Some("1.0".into()),
            studio_version: None,
            source: "m.zip".into(),
            sha256: None,
            source_size: None,
            imported: 0,
            files: vec![
                file(package::DATABASE.into(), Class::Database),
                file(text("eng"), Class::Text),
                file(text("ger"), Class::Text),
            ],
            skipped: Vec::new(),
            track: None,
            part: None,
        };
        let game = Game {
            dir: stock.clone(),
            exe: "1.0",
        };
        let (d, values) = compute(&p, &ours, &game).unwrap();
        let key = |k: &str| swofty_formats::hash::fnv1a64(k);
        assert_eq!(d.records.changed, [key("b")]);
        assert_eq!(d.records.added, [key("d")]);
        assert_eq!(d.records.removed, [key("c")]);
        assert_eq!(values.records.len(), 2);
        assert_eq!(d.dropped, 0);
        let langs: Vec<(&str, Option<&str>, usize)> = d
            .text
            .iter()
            .map(|t| (t.language.as_str(), t.from.as_deref(), t.len()))
            .collect();
        assert_eq!(
            langs,
            [("eng", None, 1), ("ger", None, 1), ("kor", Some("eng"), 1)]
        );
        assert_eq!(d.text[2].added["y"], "new");
        let older = Game {
            dir: stock,
            exe: "1.1",
        };
        let (d, _) = compute(&p, &ours, &older).unwrap();
        assert!(d.records.removed.is_empty());
        assert_eq!(d.dropped, 1);
        assert_eq!(d.records.changed, [key("b")]);
        let unversioned = Package {
            game_version: None,
            ..p.clone()
        };
        let (d, _) = compute(&unversioned, &ours, &game).unwrap();
        assert!(d.records.removed.is_empty());
        assert_eq!(d.dropped, 1);
        std::fs::remove_dir_all(&root).ok();
    }
}
