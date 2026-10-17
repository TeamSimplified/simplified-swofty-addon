use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use swofty_addon::launcher::GameFiles;
use swofty_formats::erp::{ErpFile, Resource};
use swofty_formats::gvdb::{Gvdb, Record};
use swofty_formats::lng::{Entry, Lng};

use crate::library::Package;
use crate::package::{self, Class};
use crate::{Result, err};

pub const FALLBACK: &str = "eng";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    pub build_key: String,
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
        self.len() == 0
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
    let removals = p.game_version.is_some() && p.game_version == game.exe_version();
    let mut diff = Diff {
        build_key: game.build_key(),
        ..Diff::default()
    };
    let mut records = Gvdb::default();
    let mut texts = Vec::new();
    for f in &p.files {
        let path = files.join(&f.path);
        match f.class {
            Class::Database => {
                let (r, values, dropped) = database(&path, game, removals)?;
                diff.records = r;
                diff.dropped += dropped;
                records = values;
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
    diff.text = text(&texts, files, game)?;
    Ok((diff, records))
}

fn read(path: &Path) -> Result<Vec<u8>> {
    package::read_file(path)
}

fn database(path: &Path, game: &dyn GameFiles, removals: bool) -> Result<(Records, Gvdb, usize)> {
    let stock = Gvdb::parse(&read(&game.original(package::DATABASE)?)?)
        .map_err(|e| err(format!("the game's database: {e}")))?;
    let ours = Gvdb::parse(&read(path)?).map_err(|e| err(format!("{}: {e}", path.display())))?;
    let index: HashMap<u64, &Record> = stock.records.iter().map(|r| (r.key, r)).collect();
    let mut out = Records::default();
    let mut values = Gvdb::default();
    let mut seen = HashSet::with_capacity(ours.records.len());
    for r in &ours.records {
        seen.insert(r.key);
        match index.get(&r.key) {
            Some(s) if s.tag == r.tag && s.value == r.value => continue,
            Some(_) => out.changed.push(r.key),
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
    Ok((out, values, dropped))
}

fn same(stock: &ErpFile, s: &Resource, ours: &ErpFile, r: &Resource) -> swofty_formats::Result<bool> {
    if s.hash != [0; 16] && r.hash != [0; 16] {
        return Ok(s.hash == r.hash);
    }
    Ok(stock.load(s)?.content_hash()? == ours.load(r)?.content_hash()?)
}

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
    let names: HashSet<&str> = ours.erp.resources.iter().map(|r| r.name.as_str()).collect();
    let removed: Vec<String> = stock
        .erp
        .resources
        .iter()
        .filter(|r| !names.contains(r.name.as_str()))
        .map(|r| r.name.clone())
        .collect();
    let dropped = match removals {
        true => {
            out.removed = removed;
            0
        }
        false => removed.len(),
    };
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

fn text(paths: &[&str], files: &Path, game: &dyn GameFiles) -> Result<Vec<Text>> {
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
    out.retain(|t| !t.is_empty());
    out.sort_by(|a, b| a.language.cmp(&b.language));
    Ok(out)
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
    fn diffs_records_and_text_and_keeps_removals_for_the_same_version() {
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
            imported: 0,
            files: vec![
                file(package::DATABASE.into(), Class::Database),
                file(text("eng"), Class::Text),
                file(text("ger"), Class::Text),
            ],
            skipped: Vec::new(),
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
        std::fs::remove_dir_all(&root).ok();
    }
}
