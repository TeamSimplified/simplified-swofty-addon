use std::collections::BTreeMap;

use swofty_addon::asset::{AssetOp, FileOp, LangOp, Root, Source};
use swofty_addon::manifest::Category;
use swofty_addon::ops::{DbOp, ErpOp};
use swofty_formats::gvdb::Gvdb;

use crate::diff::Diff;
use crate::library::{FILES, LIBRARY};
use crate::texarray;

// A resource a mod adds or changes in an asset group archive goes into an overlay on
// that archive, as the mod's archive replaces the game's there and nowhere else.
// Resources from shader archives, and from archives no asset group lists, go into an
// overlay on this archive, which the boot groups load before any car or UI group:
// they override the game's copies everywhere, as the shader archives Simplified
// Studio packs a mod into do.
pub const BOOT_ERP: &str = "2025_asset_groups/vfx_package/common.erp";
const GROUPS: &str = "2025_asset_groups/";

pub fn target(rel: &str) -> &str {
    match rel.starts_with(GROUPS) {
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

// A mod that loads resources from a shader package, as Simplified Studio and My Team
// packages do, is built on Team Simplified's SERPs Base Files.
pub fn needs_base(diff: &Diff) -> bool {
    !shader_packages(diff).is_empty()
}

// The shader packages a mod loads resources from.
fn shader_packages(diff: &Diff) -> Vec<&str> {
    diff.archives
        .iter()
        .filter(|a| carrier(&a.path))
        .filter(|a| a.changed.iter().chain(&a.added).any(|uri| loaded(a, uri)))
        .map(|a| a.path.rsplit('/').next().unwrap_or(&a.path))
        .collect()
}

// What each shader package a Simplified Studio mod builds replaces in the game, by the
// start of its file name (Team Simplified's shader package list), and what kind of
// change that is.
const REPLACES: [(&str, &str, Kind); 17] = [
    ("object_alpha.nefx2.sm51", "the Alpine team", Kind::Team),
    (
        "object_auto_illuminate.nefx2.sm51",
        "the Aston Martin team",
        Kind::Team,
    ),
    ("object_car_monitor.nefx2.sm51", "the Audi team", Kind::Team),
    (
        "object_driver_name_shader.nefx2.sm51",
        "the Cadillac team",
        Kind::Team,
    ),
    ("object_flag.nefx2.sm51", "the Ferrari team", Kind::Team),
    ("object_glass.nefx2.sm51", "the Haas team", Kind::Team),
    ("object_jumbotron.nefx2.sm51", "the McLaren team", Kind::Team),
    ("object_mask.nefx2.sm51", "the Mercedes team", Kind::Team),
    ("object_my_team.nefx2.sm51", "My Team", Kind::Team),
    ("effects_myteam.nefx2.sm51", "My Team", Kind::Team),
    ("object_opaque.nefx2.sm51", "the Racing Bulls team", Kind::Team),
    ("object_overlay.nefx2.sm51", "the Red Bull team", Kind::Team),
    ("object_roof.nefx2.sm51", "the Williams team", Kind::Team),
    ("object_skinned.nefx2.sm51", "the car performance", Kind::Car),
    ("object_steering_screen.nefx2.sm51", "the tyres", Kind::Car),
    ("object_sponsor.nefx2.sm51", "the sponsor boards", Kind::Track),
    ("ui_texture.nefx2.sm51", "the UI", Kind::Ui),
];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Team,
    Car,
    Track,
    Ui,
}

// A season replaces this many teams.
const SEASON_TEAMS: usize = 3;

// The rows of REPLACES a mod's shader packages match.
fn matched(diff: &Diff) -> Vec<(&'static str, Kind)> {
    let packages = shader_packages(diff);
    let mut out: Vec<(&'static str, Kind)> = REPLACES
        .iter()
        .filter(|(prefix, ..)| packages.iter().any(|p| p.starts_with(prefix)))
        .map(|(_, what, kind)| (*what, *kind))
        .collect();
    out.dedup();
    out
}

// What a mod's shader packages replace, each once, in the order of REPLACES.
pub fn replaces(diff: &Diff) -> Vec<&'static str> {
    matched(diff).into_iter().map(|(what, _)| what).collect()
}

// The category from what a mod's shader packages replace: Seasons for several teams,
// Cars for a team, the car performance or the tyres, Tracks for the sponsor boards.
// Anything else, the UI alone included, has none.
pub fn category(diff: &Diff) -> Option<Category> {
    category_of(
        &matched(diff)
            .into_iter()
            .map(|(_, kind)| kind)
            .collect::<Vec<_>>(),
    )
}

// The same from a description's "Replaces …" sentence, for mods converted before.
pub fn category_described(description: &str) -> Option<Category> {
    let mut found: Vec<(&str, Kind)> = Vec::new();
    for (_, what, kind) in REPLACES {
        if description.contains(what) && !found.iter().any(|(w, _)| *w == what) {
            found.push((what, kind));
        }
    }
    category_of(&found.into_iter().map(|(_, kind)| kind).collect::<Vec<_>>())
}

fn category_of(kinds: &[Kind]) -> Option<Category> {
    let teams = kinds.iter().filter(|k| **k == Kind::Team).count();
    if teams >= SEASON_TEAMS {
        Some(Category::Seasons)
    } else if teams > 0 || kinds.contains(&Kind::Car) {
        Some(Category::Cars)
    } else if kinds.contains(&Kind::Track) {
        Some(Category::Tracks)
    } else {
        None
    }
}
// Simplified Studio adds this vehicle shadow material to every shader package it
// builds, beside the marker (SerpsOverlay.cs in the Simplified Suite). Loaded at boot
// it would replace the game's own shadow material, so it is left out too.
const SHADOW: &str = "/shared/shadows/materials/f1_vehicle_shadow.material";

// What Simplified Studio adds to every shader package it builds.
pub fn studio_only(uri: &str) -> bool {
    uri.starts_with(MARKER) || uri.ends_with(SHADOW)
}

pub fn carrier(rel: &str) -> bool {
    rel.starts_with("shader_package")
}

// Resources added to ui_texture archives are merged across mods by texarray::build,
// and Simplified Studio's marker does nothing in the game.
pub fn loaded(a: &crate::diff::Archive, uri: &str) -> bool {
    let studio = uri.starts_with(MARKER) || carrier(&a.path) && studio_only(uri);
    !(texarray::carrier(&a.path) && texarray::merged_by_slice(uri) || studio)
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
    fn a_mod_is_categorised_by_what_its_shader_packages_replace() {
        let diff = |names: &[&str]| Diff {
            archives: names
                .iter()
                .map(|name| Archive {
                    path: format!("shader_package_2025/{name}.nefx2.sm51.erp"),
                    changed: Vec::new(),
                    added: vec!["eaid://f1_2026_vehicle_package/teams/a.material".into()],
                    removed: Vec::new(),
                })
                .collect(),
            ..Diff::default()
        };
        assert_eq!(category(&diff(&["object_flag"])), Some(Category::Cars));
        assert_eq!(
            category(&diff(&["object_my_team", "effects_myteam"])),
            Some(Category::Cars)
        );
        assert_eq!(category(&diff(&["object_steering_screen"])), Some(Category::Cars));
        assert_eq!(category(&diff(&["object_skinned"])), Some(Category::Cars));
        assert_eq!(category(&diff(&["object_sponsor"])), Some(Category::Tracks));
        assert_eq!(category(&diff(&["ui_texture"])), None);
        assert_eq!(category(&diff(&["bloom"])), None);
        assert_eq!(category(&diff(&[])), None);
        assert_eq!(
            category(&diff(&[
                "object_flag",
                "object_mask",
                "object_roof",
                "ui_texture"
            ])),
            Some(Category::Seasons)
        );
        assert_eq!(
            category_described("Replaces the Cadillac team and the UI. 3 car assets."),
            Some(Category::Cars)
        );
        assert_eq!(
            category_described("Replaces the Alpine team, the Haas team and My Team."),
            Some(Category::Seasons)
        );
        assert_eq!(category_described("Replaces the UI."), None);
        assert_eq!(category_described("3 car assets."), None);
    }

    #[test]
    fn a_mod_replaces_what_its_shader_packages_stand_for() {
        let archive = |name: &str| Archive {
            path: format!("shader_package_2025/{name}.nefx2.sm51.erp"),
            changed: Vec::new(),
            added: vec!["eaid://f1_2026_vehicle_package/teams/a.material".into()],
            removed: Vec::new(),
        };
        let diff = Diff {
            archives: vec![
                archive("object_overlay"),
                archive("object_flag"),
                archive("object_my_team"),
                archive("effects_myteam"),
                archive("bloom"),
            ],
            ..Diff::default()
        };
        assert_eq!(
            replaces(&diff),
            ["the Ferrari team", "My Team", "the Red Bull team"]
        );
    }

    #[test]
    fn a_mod_needs_the_base_files_when_it_loads_from_a_shader_package() {
        let diff = |path: &str, uri: &str| Diff {
            archives: vec![Archive {
                path: path.into(),
                changed: Vec::new(),
                added: vec![uri.into()],
                removed: Vec::new(),
            }],
            ..Diff::default()
        };
        let shader = "shader_package_2025/object_my_team.nefx2.sm51.erp";
        assert!(needs_base(&diff(
            shader,
            "eaid://f1_2026_vehicle_package/teams/a.material"
        )));
        assert!(!needs_base(&diff(shader, "eaid://tools_package/default/x")));
        assert!(!needs_base(&diff(
            "2025_asset_groups/vfx_package/common.erp",
            "eaid://f1_2026_vehicle_package/teams/a.material"
        )));
    }

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
                },
                Archive {
                    path: "2025_asset_groups/ui_package/flow_boot.erp".into(),
                    changed: Vec::new(),
                    added: vec!["eaid://ui_package/b".into()],
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
        assert_eq!(
            puts,
            [
                (
                    BOOT_ERP,
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
        assert_eq!((merged.resources, merged.carried), (3, 2));
        assert!(targets(&diff).contains(&format!(
            "eaid://f1_2026_vehicle_package/teams/genesis/a.tif.image in {BOOT_ERP}"
        )));
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
    fn studio_s_marker_and_shadow_material_are_left_out() {
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
                (car, opaque),
                (
                    shadow,
                    "library/m/files/2025_asset_groups/f1_2025_vehicle_package/shared/shared.erp"
                ),
            ]
        );
        assert_eq!(merged.carried, 1);
        assert!(
            !targets(&diff)
                .iter()
                .any(|t| t.contains("object_opaque") || t.starts_with("eaid://tools"))
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
