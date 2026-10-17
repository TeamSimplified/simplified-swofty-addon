use std::collections::BTreeMap;

use swofty_addon::asset::{AssetOp, FileOp, LangOp, Root, Source};
use swofty_addon::ops::{DbOp, ErpOp};
use swofty_formats::gvdb::Gvdb;

use crate::diff::Diff;
use crate::library::{FILES, LIBRARY};

pub struct Merged {
    pub ops: Vec<AssetOp>,
    pub records: usize,
    pub resources: usize,
    pub strings: usize,
    pub files: usize,
    pub unbuilt: usize,
}

pub fn key(k: u64) -> String {
    format!("0x{k:016x}")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn ops(id: &str, diff: &Diff, records: &Gvdb) -> Merged {
    let source = |rel: &str| Source {
        root: Root::Data,
        path: format!("{LIBRARY}/{id}/{FILES}/{rel}"),
    };
    let mut ops = Vec::new();
    for r in &records.records {
        ops.push(AssetOp::Db(DbOp::PutRecord {
            key: key(r.key),
            tag: r.tag,
            value: hex(&r.value),
        }));
    }
    for k in &diff.records.removed {
        ops.push(AssetOp::Db(DbOp::RemoveRecord { key: key(*k) }));
    }
    let mut resources = 0;
    let mut unbuilt = 0;
    for a in &diff.archives {
        for uri in a.changed.iter().chain(&a.added) {
            ops.push(AssetOp::Erp(ErpOp::PutResource {
                erp: a.path.clone(),
                resource: uri.clone(),
                source: source(&a.path),
                rename: None,
            }));
            resources += 1;
        }
        unbuilt += a.removed.len();
    }
    let mut strings = 0;
    for t in &diff.text {
        for (k, v) in t.entries() {
            ops.push(AssetOp::Lang(LangOp::Set {
                language: t.language.clone(),
                key: k.clone(),
                value: v.clone(),
            }));
            strings += 1;
        }
    }
    for f in &diff.files {
        ops.push(AssetOp::File(FileOp::Put {
            source: source(&f.path),
            target: f.path.clone(),
        }));
    }
    Merged {
        ops,
        records: records.records.len() + diff.records.removed.len(),
        resources,
        strings,
        files: diff.files.len(),
        unbuilt,
    }
}

pub fn targets(diff: &Diff) -> Vec<String> {
    let mut out = Vec::new();
    let r = &diff.records;
    out.extend(
        r.changed
            .iter()
            .chain(&r.added)
            .chain(&r.removed)
            .map(|k| key(*k)),
    );
    for a in &diff.archives {
        let uris = a.changed.iter().chain(&a.added).chain(&a.removed);
        out.extend(uris.map(|uri| format!("{uri} in {}", a.path)));
    }
    for t in &diff.text {
        out.extend(t.entries().map(|(k, _)| format!("{k} ({})", t.language)));
    }
    out.extend(diff.files.iter().map(|f| f.path.clone()));
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clash {
    pub winner: usize,
    pub loser: usize,
    pub targets: Vec<String>,
}

pub fn clashes(diffs: &[Option<&Diff>]) -> Vec<Clash> {
    let mut writers: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, d) in diffs.iter().enumerate() {
        for t in d.map(targets).unwrap_or_default() {
            writers.entry(t).or_default().push(i);
        }
    }
    let mut pairs: BTreeMap<(usize, usize), Vec<String>> = BTreeMap::new();
    for (target, w) in writers {
        let Some((&winner, earlier)) = w.split_last() else {
            continue;
        };
        for &loser in earlier {
            pairs.entry((winner, loser)).or_default().push(target.clone());
        }
    }
    pairs
        .into_iter()
        .map(|((winner, loser), targets)| Clash {
            winner,
            loser,
            targets,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{Archive, Records};

    #[test]
    fn later_mods_win_shared_targets() {
        let a = Diff {
            records: Records {
                changed: vec![1, 2],
                ..Records::default()
            },
            ..Diff::default()
        };
        let b = Diff {
            records: Records {
                changed: vec![2],
                ..Records::default()
            },
            archives: vec![Archive {
                path: "x.erp".into(),
                changed: vec!["eaid://r".into()],
                added: Vec::new(),
                removed: Vec::new(),
            }],
            ..Diff::default()
        };
        let c = Diff {
            archives: b.archives.clone(),
            ..Diff::default()
        };
        let found = clashes(&[Some(&a), None, Some(&b), Some(&c)]);
        assert_eq!(
            found,
            [
                Clash {
                    winner: 2,
                    loser: 0,
                    targets: vec![key(2)],
                },
                Clash {
                    winner: 3,
                    loser: 2,
                    targets: vec!["eaid://r in x.erp".into()],
                },
            ]
        );
    }
}
