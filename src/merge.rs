use std::collections::BTreeMap;

use swofty_addon::asset::{AssetOp, FileOp, LangOp, Root, Source};
use swofty_addon::ops::{DbOp, ErpOp};
use swofty_formats::gvdb::Gvdb;

use crate::diff::Diff;
use crate::library::{FILES, LIBRARY};
use crate::texarray;

// A resource a mod adds or changes in an asset group archive goes into an overlay on
// that archive, as the mod's archive replaces the game's there and nowhere else.
// Resources a Simplified Studio shader archive carries go into that shader archive, as
// the Simplified Launcher merges them there: moved to the boot overlay instead, a car
// they hold never finishes loading. Resources from other archives no asset group lists
// go into an overlay on this archive, which the boot groups load before any car or UI
// group.
pub const BOOT_ERP: &str = "2025_asset_groups/vfx_package/common.erp";
const GROUPS: &str = "2025_asset_groups/";

pub fn target(rel: &str) -> &str {
    match rel.starts_with(GROUPS) || carrier(rel) {
        true => rel,
        false => BOOT_ERP,
    }
}

pub struct Merged {
    pub ops: Vec<AssetOp>,
    // Set before every mod's own changes, so that any mod's rename wins.
    pub defaults: Vec<AssetOp>,
    pub records: usize,
    pub resources: usize,
    pub carried: usize,
    pub strings: usize,
    pub files: usize,
    pub removals: usize,
}

pub const MARKER: &str = "eaid://tools_package/";
// Team Simplified's SERPs Base Files carry this marker. The Simplified Launcher turns
// them on for any mod with a shader archive that names "serps", and they add the
// shared materials and textures Simplified Studio mods use.
pub const BASE_MARKER: &str = "eaid://tools_package/default/serps_basefiles";
pub const BASE_ADDON: &str = "mod.team-simplified.serps-base-files-for-f1-25";

pub fn needs_base(diff: &Diff) -> bool {
    diff.archives.iter().any(|a| {
        carrier(&a.path)
            && a.changed
                .iter()
                .chain(&a.added)
                .any(|uri| uri.contains("serps") && uri != BASE_MARKER)
    })
}

// Simplified Studio adds this vehicle shadow material to every shader package it
// builds, beside the marker (SerpsOverlay.cs in the Simplified Suite). Both load with
// the rest of the mod, as they do from the shader package.
const SHADOW: &str = "/shared/shadows/materials/f1_vehicle_shadow.material";

// What Simplified Studio adds to every shader package it builds. Every Studio mod
// writes the same copies, so they are not conflicts.
pub fn studio_only(uri: &str) -> bool {
    uri.starts_with(MARKER) || uri.ends_with(SHADOW)
}

pub fn carrier(rel: &str) -> bool {
    rel.starts_with("shader_package")
}

// Texture arrays in ui_texture archives merge slice by slice instead.
pub fn loaded(a: &crate::diff::Archive, uri: &str) -> bool {
    !(texarray::carrier(&a.path) && texarray::merged_by_slice(uri))
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
    let mut removals = 0;
    for a in &diff.archives {
        let changed: Vec<&String> = a.changed.iter().filter(|uri| loaded(a, uri)).collect();
        let adds: Vec<&String> = a.added.iter().filter(|uri| loaded(a, uri)).collect();
        for &uri in changed.iter().chain(&adds) {
            ops.push(AssetOp::Erp(ErpOp::PutResource {
                erp: target(&a.path).into(),
                resource: uri.clone(),
                source: source(&a.path),
                rename: None,
            }));
            resources += 1;
        }
        if carrier(&a.path) {
            carried += changed.len() + adds.len();
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
        let written = a
            .changed
            .iter()
            .chain(&a.added)
            .filter(|uri| loaded(a, uri) && !studio_only(uri));
        out.extend(written.map(|uri| format!("{uri} in {}", target(&a.path))));
        out.extend(a.removed.iter().map(|uri| format!("{uri} in {}", a.path)));
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
    fn resources_carried_in_shader_archives_stay_in_their_shader_archive() {
        let shader = "shader_package_2025/object_auto_illuminate.nefx2.sm51.erp";
        let amd = "shader_package_2025/object_auto_illuminate.nefx2.sm51_amd.erp";
        let loose = "character_package/drivers/x.erp";
        let genesis = "eaid://f1_2026_vehicle_package/teams/genesis/a.tif.image";
        let carried = |path: &str| Archive {
            path: path.into(),
            changed: vec!["eaid://shader_package/x".into()],
            added: vec![genesis.into()],
            removed: Vec::new(),
        };
        let diff = Diff {
            archives: vec![
                carried(shader),
                carried(amd),
                Archive {
                    path: "2025_asset_groups/ui_package/flow_boot.erp".into(),
                    changed: Vec::new(),
                    added: vec!["eaid://ui_package/b".into()],
                    removed: Vec::new(),
                },
                Archive {
                    path: loose.into(),
                    changed: Vec::new(),
                    added: vec!["eaid://character_package/c".into()],
                    removed: Vec::new(),
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
        let from = |rel: &str| format!("library/m/files/{rel}");
        let (from_shader, from_amd) = (from(shader), from(amd));
        assert_eq!(
            puts,
            [
                (shader, "eaid://shader_package/x", from_shader.as_str()),
                (shader, genesis, from_shader.as_str()),
                (amd, "eaid://shader_package/x", from_amd.as_str()),
                (amd, genesis, from_amd.as_str()),
                (
                    "2025_asset_groups/ui_package/flow_boot.erp",
                    "eaid://ui_package/b",
                    "library/m/files/2025_asset_groups/ui_package/flow_boot.erp"
                ),
                (BOOT_ERP, "eaid://character_package/c", from(loose).as_str()),
            ]
        );
        assert_eq!((merged.resources, merged.carried), (6, 4));
        let t = targets(&diff);
        assert!(t.contains(&format!("{genesis} in {shader}")));
        assert!(t.contains(&format!("{genesis} in {amd}")));
        assert!(
            !t.iter().any(|t| t.contains(BOOT_ERP) && t.contains("genesis")),
            "nothing a shader archive carries goes to the boot overlay"
        );
    }

    #[test]
    fn every_change_to_a_car_archive_loads_with_that_archive() {
        let car = "2025_asset_groups/f1_2026_vehicle_package/teams/mclaren/wep/mclaren.erp";
        let diff = Diff {
            archives: vec![Archive {
                path: car.into(),
                changed: vec!["eaid://paint".into(), "eaid://handling".into()],
                added: vec!["eaid://decal".into()],
                removed: vec!["eaid://old".into()],
            }],
            ..Diff::default()
        };
        let merged = ops("m", &diff, &Gvdb::default());
        let writes: Vec<(&str, &str)> = merged
            .ops
            .iter()
            .filter_map(|op| match op {
                AssetOp::Erp(ErpOp::PutResource { erp, resource, .. })
                | AssetOp::Erp(ErpOp::RemoveResource { erp, resource }) => {
                    Some((erp.as_str(), resource.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            writes,
            [
                (car, "eaid://paint"),
                (car, "eaid://handling"),
                (car, "eaid://decal"),
                (car, "eaid://old"),
            ]
        );
        assert_eq!((merged.resources, merged.carried), (3, 0));
        assert!(targets(&diff).contains(&format!("eaid://paint in {car}")));
    }

    #[test]
    fn studio_s_marker_and_shadow_material_load_with_the_mod() {
        let shadow = "eaid://f1_2025_vehicle_package/shared/shadows/materials/f1_vehicle_shadow.material";
        let car = "eaid://f1_2026_vehicle_package/teams/genesis/a.tif.image";
        let diff = Diff {
            archives: vec![
                Archive {
                    path: "shader_package_2025/object_opaque.nefx2.sm51.erp".into(),
                    changed: vec![shadow.into()],
                    added: vec![
                        shadow.into(),
                        car.into(),
                        "eaid://tools_package/default/serps".into(),
                    ],
                    removed: Vec::new(),
                },
                Archive {
                    path: "2025_asset_groups/f1_2025_vehicle_package/shared/shared.erp".into(),
                    changed: vec![shadow.into()],
                    added: Vec::new(),
                    removed: Vec::new(),
                },
            ],
            ..Diff::default()
        };
        let merged = ops("m", &diff, &Gvdb::default());
        let puts: Vec<(&str, &str)> = merged
            .ops
            .iter()
            .filter_map(|op| match op {
                AssetOp::Erp(ErpOp::PutResource { resource, source, .. }) => {
                    Some((resource.as_str(), source.path.as_str()))
                }
                _ => None,
            })
            .collect();
        let opaque = "library/m/files/shader_package_2025/object_opaque.nefx2.sm51.erp";
        assert_eq!(
            puts,
            [
                (shadow, opaque),
                (shadow, opaque),
                (car, opaque),
                ("eaid://tools_package/default/serps", opaque),
                (
                    shadow,
                    "library/m/files/2025_asset_groups/f1_2025_vehicle_package/shared/shared.erp"
                ),
            ]
        );
        assert_eq!(merged.carried, 4);
        let opaque_rel = "shader_package_2025/object_opaque.nefx2.sm51.erp";
        assert_eq!(
            targets(&diff)
                .into_iter()
                .filter(|t| t.ends_with(opaque_rel))
                .collect::<Vec<_>>(),
            [format!("{car} in {opaque_rel}")],
            "the marker and shadow material travel with their shader archive and are no conflict"
        );
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
                },
                Archive {
                    path: shader.into(),
                    changed: Vec::new(),
                    added: Vec::new(),
                    removed: vec!["eaid://shader_package/d".into()],
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
            }],
            ..Diff::default()
        };
        assert!(targets(&d).is_empty());
        assert_eq!(clashes(&[Some(&d), Some(&d)]), []);
        let merged = ops("m", &d, &Gvdb::default());
        assert_eq!((merged.resources, merged.carried), (1, 1));
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
                    targets: vec![format!("eaid://r in {BOOT_ERP}")],
                },
            ]
        );
    }
}
