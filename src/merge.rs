use std::collections::BTreeMap;

use swofty_addon::asset::{AssetOp, FileOp, LangOp, Root, Source};
use swofty_addon::ops::{DbOp, ErpOp};
use swofty_formats::gvdb::Gvdb;

use crate::diff::Diff;
use crate::library::{FILES, LIBRARY};
use crate::texarray;

// Simplified Studio carries a mod's textures and models in the shader archives the
// game loads at boot. Those archives are not listed by asset groups the loader can
// patch, so the resources they carry go into an overlay on an archive the boot group
// loads instead.
pub const BOOT_ERP: &str = "2025_asset_groups/vfx_package/common.erp";

pub struct Merged {
    pub ops: Vec<AssetOp>,
    // Set before every mod's own changes, so that any mod's rename wins.
    pub defaults: Vec<AssetOp>,
    pub records: usize,
    pub resources: usize,
    pub carried: usize,
    pub relaid: usize,
    pub strings: usize,
    pub files: usize,
    pub removals: usize,
}

pub const MARKER: &str = "eaid://tools_package/";

pub fn carrier(rel: &str) -> bool {
    rel.starts_with("shader_package")
}

// The in-game loader replaces a resource in its own archive only when the
// replacement keeps the game's layout. One that doesn't, such as a texture of
// another size, loads with the boot archive instead, as resources carried in
// shader archives do, and overrides the game's.
fn changed_in<'a>(a: &'a crate::diff::Archive, uri: &String) -> &'a str {
    match a.relayout.contains(uri) && !carrier(&a.path) {
        true => BOOT_ERP,
        false => &a.path,
    }
}

fn added_to(path: &str) -> &str {
    match (texarray::carrier(path), carrier(path)) {
        (true, _) => texarray::UI_ERP,
        (false, true) => BOOT_ERP,
        (false, false) => path,
    }
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
    let mut carried = 0;
    let mut relaid = 0;
    let mut removals = 0;
    for a in &diff.archives {
        let added = added_to(&a.path);
        let changed = a.changed.iter().map(|uri| (changed_in(a, uri), uri));
        relaid += a
            .relayout
            .iter()
            .filter(|uri| changed_in(a, uri) == BOOT_ERP)
            .count();
        // ui_texture archives are merged across mods by texarray::build, and
        // Simplified Studio's marker does nothing in the game.
        let carried_ui = added == texarray::UI_ERP;
        let in_carrier = added != a.path;
        let adds = a
            .added
            .iter()
            .filter(|uri| !(carried_ui || in_carrier && uri.starts_with(MARKER)));
        for (erp, uri) in changed.chain(adds.map(|uri| (added, uri))) {
            ops.push(AssetOp::Erp(ErpOp::PutResource {
                erp: erp.to_owned(),
                resource: uri.clone(),
                source: source(&a.path),
                rename: None,
            }));
            resources += 1;
        }
        if added == BOOT_ERP {
            carried += a.added.iter().filter(|uri| !uri.starts_with(MARKER)).count();
        }
        for uri in &a.removed {
            ops.push(AssetOp::Erp(ErpOp::RemoveResource {
                erp: a.path.clone(),
                resource: uri.clone(),
            }));
        }
        removals += a.removed.len();
    }
    let mut strings = 0;
    let mut defaults = Vec::new();
    let set = |t: &crate::diff::Text, k: &String, v: &String| {
        AssetOp::Lang(LangOp::Set {
            language: t.language.clone(),
            key: k.clone(),
            value: v.clone(),
        })
    };
    for t in &diff.text {
        for (k, v) in t.entries() {
            ops.push(set(t, k, v));
            strings += 1;
        }
        defaults.extend(t.defaults.iter().map(|(k, v)| set(t, k, v)));
    }
    for f in &diff.files {
        ops.push(AssetOp::File(FileOp::Put {
            source: source(&f.path),
            target: f.path.clone(),
        }));
    }
    Merged {
        ops,
        defaults,
        records: records.records.len() + diff.records.removed.len(),
        resources,
        carried,
        relaid,
        strings,
        files: diff.files.len(),
        removals,
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
        out.extend(
            a.changed
                .iter()
                .map(|uri| format!("{uri} in {}", changed_in(a, uri))),
        );
        out.extend(a.removed.iter().map(|uri| format!("{uri} in {}", a.path)));
        let added = added_to(&a.path);
        let adds = a.added.iter().filter(|uri| {
            let merged = added == texarray::UI_ERP && texarray::merged_by_slice(uri);
            !(uri.starts_with(MARKER) || merged)
        });
        out.extend(adds.map(|uri| format!("{uri} in {added}")));
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
            let w = writers.entry(t).or_default();
            if w.last() != Some(&i) {
                w.push(i);
            }
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
    fn resources_carried_in_shader_archives_go_to_the_boot_archive() {
        let shader = "shader_package_2025/object_auto_illuminate.nefx2.sm51.erp";
        let diff = Diff {
            archives: vec![
                Archive {
                    path: shader.into(),
                    changed: vec!["eaid://shader_package/x".into()],
                    added: vec!["eaid://f1_2026_vehicle_package/teams/genesis/a.tif.image".into()],
                    removed: Vec::new(),
                    relayout: Vec::new(),
                },
                Archive {
                    path: "2025_asset_groups/ui_package/flow_boot.erp".into(),
                    changed: Vec::new(),
                    added: vec!["eaid://ui_package/b".into()],
                    removed: Vec::new(),
                    relayout: Vec::new(),
                },
            ],
            ..Diff::default()
        };
        let merged = ops("m", &diff, &Gvdb::default());
        let puts: Vec<(&str, &str, &str)> = merged
            .ops
            .iter()
            .filter_map(|op| match op {
                AssetOp::Erp(ErpOp::PutResource {
                    erp,
                    resource,
                    source,
                    ..
                }) => Some((erp.as_str(), resource.as_str(), source.path.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            puts,
            [
                (
                    shader,
                    "eaid://shader_package/x",
                    "library/m/files/shader_package_2025/object_auto_illuminate.nefx2.sm51.erp"
                ),
                (
                    BOOT_ERP,
                    "eaid://f1_2026_vehicle_package/teams/genesis/a.tif.image",
                    "library/m/files/shader_package_2025/object_auto_illuminate.nefx2.sm51.erp"
                ),
                (
                    "2025_asset_groups/ui_package/flow_boot.erp",
                    "eaid://ui_package/b",
                    "library/m/files/2025_asset_groups/ui_package/flow_boot.erp"
                ),
            ]
        );
        assert_eq!((merged.resources, merged.carried), (3, 1));
        assert!(targets(&diff).contains(&format!(
            "eaid://f1_2026_vehicle_package/teams/genesis/a.tif.image in {BOOT_ERP}"
        )));
    }

    #[test]
    fn a_change_of_layout_loads_with_the_boot_archive() {
        let car = "2025_asset_groups/f1_2026_vehicle_package/teams/mclaren/wep/mclaren.erp";
        let diff = Diff {
            archives: vec![Archive {
                path: car.into(),
                changed: vec!["eaid://paint".into(), "eaid://handling".into()],
                added: Vec::new(),
                removed: Vec::new(),
                relayout: vec!["eaid://paint".into()],
            }],
            ..Diff::default()
        };
        let merged = ops("m", &diff, &Gvdb::default());
        let puts: Vec<(&str, &str)> = merged
            .ops
            .iter()
            .filter_map(|op| match op {
                AssetOp::Erp(ErpOp::PutResource { erp, resource, .. }) => {
                    Some((erp.as_str(), resource.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(puts, [(BOOT_ERP, "eaid://paint"), (car, "eaid://handling")]);
        assert_eq!(merged.relaid, 1);
        assert!(targets(&diff).contains(&format!("eaid://paint in {BOOT_ERP}")));
    }

    #[test]
    fn removed_resources_become_remove_resource_ops() {
        let flow = "2025_asset_groups/ui_package/flow_boot.erp";
        let shader = "shader_package_2025/object_glass.nefx2.sm51.erp";
        let diff = Diff {
            archives: vec![
                Archive {
                    path: flow.into(),
                    changed: vec!["eaid://ui_package/a".into()],
                    added: Vec::new(),
                    removed: vec!["eaid://ui_package/b".into(), "eaid://ui_package/c".into()],
                    relayout: Vec::new(),
                },
                Archive {
                    path: shader.into(),
                    changed: Vec::new(),
                    added: Vec::new(),
                    removed: vec!["eaid://shader_package/d".into()],
                    relayout: Vec::new(),
                },
            ],
            ..Diff::default()
        };
        let merged = ops("m", &diff, &Gvdb::default());
        let removed: Vec<(&str, &str)> = merged
            .ops
            .iter()
            .filter_map(|op| match op {
                AssetOp::Erp(ErpOp::RemoveResource { erp, resource }) => {
                    Some((erp.as_str(), resource.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            removed,
            [
                (flow, "eaid://ui_package/b"),
                (flow, "eaid://ui_package/c"),
                (shader, "eaid://shader_package/d"),
            ]
        );
        assert_eq!((merged.resources, merged.removals), (1, 3));
        assert!(targets(&diff).contains(&format!("eaid://ui_package/b in {flow}")));
    }

    #[test]
    fn a_mod_does_not_clash_with_itself() {
        let texture = "eaid://f1_2026_vehicle_package/teams/genesis/a.tif.image";
        let carrier = |path: &str| Archive {
            path: path.into(),
            changed: Vec::new(),
            added: vec![texture.into()],
            removed: Vec::new(),
            relayout: Vec::new(),
        };
        let d = Diff {
            archives: vec![
                carrier("shader_package_2025/object_auto_illuminate.nefx2.sm51.erp"),
                carrier("shader_package_2025/object_glass.nefx2.sm51.erp"),
            ],
            ..Diff::default()
        };
        assert_eq!(clashes(&[Some(&d)]), []);
        let found = clashes(&[Some(&d), Some(&d)]);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].winner, found[0].loser), (1, 0));
    }

    #[test]
    fn markers_and_merged_logo_arrays_are_not_conflicts() {
        let logos =
            "eaid://ui_package/art/texture_arrays/overrides.uidb/overrides.team_logos.f1_2026.texarray";
        let d = Diff {
            archives: vec![Archive {
                path: "shader_package_2025/ui_texture.nefx2.sm51.erp".into(),
                changed: Vec::new(),
                added: vec![
                    "eaid://tools_package/default/serps".into(),
                    logos.into(),
                    format!("{logos}.image"),
                ],
                removed: Vec::new(),
                relayout: Vec::new(),
            }],
            ..Diff::default()
        };
        assert!(targets(&d).is_empty());
        assert_eq!(clashes(&[Some(&d), Some(&d)]), []);
        let merged = ops("m", &d, &Gvdb::default());
        assert_eq!((merged.resources, merged.carried), (0, 0));
    }

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
                relayout: Vec::new(),
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
