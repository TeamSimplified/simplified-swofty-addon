// What the Mods page shows of each mod's diff, and the conflicts between enabled
// mods, kept between views. The launcher keeps this helper running, so the cache
// lives in memory, and in library/.view-cache/<build key>.json for the next start.
// A mod's entry is reused while its diff file keeps the same size and modification
// time; the conflicts are reused while the enabled mods, their order and their diffs
// are the same.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::diff::{Diff, FileChange};
use crate::library::{Library, Package};
use crate::merge;
use crate::package::Class;
use crate::{Result, io_err};

pub const VERSION: u32 = 1;
pub const DIR: &str = ".view-cache";
pub const SAMPLE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub len: u64,
    pub modified: u128,
}

pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    Some(Stamp {
        len: meta.len(),
        modified,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub empty: bool,
    pub records: [usize; 3],
    pub text: Vec<TextCount>,
    pub archives: Vec<ArchiveCount>,
    pub replaced: Vec<String>,
    pub added: Vec<String>,
    pub dropped: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextCount {
    pub language: String,
    pub from: Option<String>,
    pub changed: usize,
    pub added: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveCount {
    pub path: String,
    pub changed: usize,
    pub added: usize,
    pub removed: usize,
}

impl Summary {
    pub fn of(d: &Diff) -> Self {
        let paths = |class: Class| -> Vec<String> {
            d.files
                .iter()
                .filter(|f: &&FileChange| f.class == class)
                .map(|f| f.path.clone())
                .collect()
        };
        Self {
            empty: d.is_empty(),
            records: [
                d.records.changed.len(),
                d.records.added.len(),
                d.records.removed.len(),
            ],
            text: d
                .text
                .iter()
                .map(|t| TextCount {
                    language: t.language.clone(),
                    from: t.from.clone(),
                    changed: t.changed.len(),
                    added: t.added.len(),
                })
                .collect(),
            archives: d
                .archives
                .iter()
                .map(|a| ArchiveCount {
                    path: a.path.clone(),
                    changed: a.changed.len(),
                    added: a.added.len(),
                    removed: a.removed.len(),
                })
                .collect(),
            replaced: paths(Class::Replacement),
            added: paths(Class::Addition),
            dropped: d.dropped,
        }
    }

    pub fn has_records(&self) -> bool {
        self.records.iter().any(|n| *n > 0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Overlap {
    pub winner: String,
    pub loser: String,
    pub count: usize,
    pub sample: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViewCache {
    version: u32,
    pub build_key: String,
    mods: BTreeMap<String, (Stamp, Summary)>,
    order: Vec<(String, Stamp)>,
    overlaps: Vec<Overlap>,
}

impl ViewCache {
    pub fn path(library: &Library, key: &str) -> PathBuf {
        library.root().join(DIR).join(format!("{key}.json"))
    }

    pub fn load(library: &Library, key: &str) -> Self {
        std::fs::read(Self::path(library, key))
            .ok()
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .filter(|c| c.version == VERSION && c.build_key == key)
            .unwrap_or_else(|| Self {
                version: VERSION,
                build_key: key.to_owned(),
                ..Self::default()
            })
    }

    pub fn save(&self, library: &Library) -> Result<()> {
        let path = Self::path(library, &self.build_key);
        let dir = library.root().join(DIR);
        std::fs::create_dir_all(&dir).map_err(io_err(&dir))?;
        let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec(self).expect("serialisable")).map_err(io_err(&tmp))?;
        std::fs::rename(&tmp, &path).map_err(io_err(&path))
    }

    pub fn overlaps(&self) -> &[Overlap] {
        &self.overlaps
    }

    // The summary of each mod's diff for this build, None for a track or a mod
    // without one, and whether anything had to be read again.
    pub fn refresh(&mut self, library: &Library, mods: &[(Package, bool)]) -> (Vec<Option<Summary>>, bool) {
        let key = self.build_key.clone();
        let mut dirty = false;
        let mut summaries = Vec::with_capacity(mods.len());
        let mut stamps = Vec::with_capacity(mods.len());
        for (p, _) in mods {
            let current = p
                .track
                .is_none()
                .then(|| stamp(&library.diff_file(&p.id, &key)))
                .flatten();
            let summary = match current {
                Some(s) => match self.mods.get(&p.id) {
                    Some((cached, summary)) if *cached == s => Some(summary.clone()),
                    _ => {
                        dirty = true;
                        library.cached(&p.id, &key).map(|d| {
                            let summary = Summary::of(&d);
                            self.mods.insert(p.id.clone(), (s, summary.clone()));
                            summary
                        })
                    }
                },
                None => None,
            };
            if summary.is_none() {
                dirty |= self.mods.remove(&p.id).is_some();
            }
            stamps.push(current.filter(|_| summary.is_some()));
            summaries.push(summary);
        }
        let ids: BTreeSet<&str> = mods.iter().map(|(p, _)| p.id.as_str()).collect();
        let before = self.mods.len();
        self.mods.retain(|id, _| ids.contains(id.as_str()));
        dirty |= self.mods.len() != before;
        let order: Vec<(String, Stamp)> = mods
            .iter()
            .zip(&stamps)
            .filter(|((_, on), _)| *on)
            .filter_map(|((p, _), s)| s.map(|s| (p.id.clone(), s)))
            .collect();
        if order != self.order {
            let diffs: Vec<Option<Diff>> = order.iter().map(|(id, _)| library.cached(id, &key)).collect();
            let refs: Vec<Option<&Diff>> = diffs.iter().map(Option::as_ref).collect();
            self.overlaps = merge::clashes(&refs)
                .into_iter()
                .map(|c| Overlap {
                    winner: order[c.winner].0.clone(),
                    loser: order[c.loser].0.clone(),
                    count: c.targets.len(),
                    sample: c.targets.into_iter().take(SAMPLE).collect(),
                })
                .collect();
            self.order = order;
            dirty = true;
        }
        (summaries, dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{Archive, VERSION as DIFF_VERSION};

    const KEY: &str = "25251167-d873a1c8d204";

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("simplified-viewcache-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn package(id: &str, track: bool) -> Package {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id.to_uppercase(), "source": "x.zip", "imported": 0, "files": [],
            "track": track.then_some("trk"),
        }))
        .unwrap()
    }

    fn write_diff(library: &Library, id: &str, uris: &[String]) {
        let diff = Diff {
            build_key: KEY.into(),
            version: DIFF_VERSION,
            archives: vec![Archive {
                path: "shader_package_2025/object_opaque.nefx2.sm51.erp".into(),
                changed: uris.to_vec(),
                added: Vec::new(),
                removed: Vec::new(),
            }],
            ..Diff::default()
        };
        let path = library.diff_file(id, KEY);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&diff).unwrap()).unwrap();
    }

    fn uris(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("eaid://shader_package/r{i:04}")).collect()
    }

    #[test]
    fn reuses_summaries_until_a_diff_changes() {
        let data = tmp("reuse");
        let library = Library::new(&data);
        write_diff(&library, "a", &uris(3));
        write_diff(&library, "b", &uris(2));
        let mods = vec![
            (package("a", false), true),
            (package("b", false), false),
            (package("t", true), true),
        ];
        let mut cache = ViewCache::load(&library, KEY);
        let (summaries, dirty) = cache.refresh(&library, &mods);
        assert!(dirty);
        assert_eq!(summaries[0].as_ref().unwrap().archives[0].changed, 3);
        assert_eq!(summaries[1].as_ref().unwrap().archives[0].changed, 2);
        assert!(summaries[2].is_none());
        cache.save(&library).unwrap();
        let mut again = ViewCache::load(&library, KEY);
        let (same, dirty) = again.refresh(&library, &mods);
        assert!(!dirty);
        assert_eq!(same, summaries);
        write_diff(&library, "b", &uris(5));
        let (changed, dirty) = again.refresh(&library, &mods);
        assert!(dirty);
        assert_eq!(changed[1].as_ref().unwrap().archives[0].changed, 5);
        std::fs::remove_file(library.diff_file("a", KEY)).unwrap();
        let (gone, dirty) = again.refresh(&library, &mods);
        assert!(dirty && gone[0].is_none());
        let (other, _) = ViewCache::load(&library, "other-key").refresh(&library, &mods);
        assert!(other.iter().all(Option::is_none));
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn recomputes_conflicts_when_the_order_changes_and_lists_a_sample() {
        let data = tmp("overlaps");
        let library = Library::new(&data);
        write_diff(&library, "a", &uris(SAMPLE + 20));
        write_diff(&library, "b", &uris(SAMPLE + 20));
        let mut mods = vec![(package("a", false), true), (package("b", false), true)];
        let mut cache = ViewCache::load(&library, KEY);
        cache.refresh(&library, &mods);
        let o = &cache.overlaps()[0];
        assert_eq!((o.winner.as_str(), o.loser.as_str()), ("b", "a"));
        assert_eq!((o.count, o.sample.len()), (SAMPLE + 20, SAMPLE));
        assert!(!cache.refresh(&library, &mods).1);
        mods.swap(0, 1);
        assert!(cache.refresh(&library, &mods).1);
        assert_eq!(cache.overlaps()[0].winner, "a");
        mods[0].1 = false;
        cache.refresh(&library, &mods);
        assert!(cache.overlaps().is_empty());
        std::fs::remove_dir_all(&data).ok();
    }
}
