// The edits as asset operations: what the build hook returns on Play, and what an
// exported addon carries.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::grid::edits::{Edits, logos_dir};
use crate::grid::grids::{Catalog, Team, encode_colour};
use crate::grid::logos::{self, Library, Placement};

pub fn team_select(label: &str) -> String {
    format!("entrants.teams.team[_metadata_={label}]")
}

pub fn own_key(label: &str, part: &str) -> String {
    format!("lng_sge_{label}_{part}")
}

fn set(select: &str, field: &str, value: impl Into<String>) -> Value {
    json!({ "op": "db.set", "select": select, "field": field, "value": value.into() })
}

fn text(key: &str, value: &str) -> Value {
    json!({ "op": "lang.set", "language": "*", "key": key, "value": value })
}

#[derive(Default)]
pub struct Out {
    pub ops: Vec<Value>,
    pub lines: Vec<String>,
    pub warnings: Vec<String>,
    pub lang_keys: BTreeSet<String>,
}

fn team_ops(team: &Team, edits: &Edits, catalog: &Catalog, out: &mut Out) {
    // A registry team's row doesn't exist yet when the database edits apply.
    if team.registry.is_some() {
        return;
    }
    let Some(edit) = edits.teams.get(&team.label) else {
        return;
    };
    let select = team_select(&team.label);
    if let Some(boss) = &edit.boss {
        out.ops.push(set(&select, "team_boss", boss.clone()));
        // His name key reads a later principal's name: he gets his own.
        if let Some(p) = catalog.principals.iter().find(|p| p.label == *boss && p.renamed) {
            let key = own_key(boss, "name");
            if out.lang_keys.insert(key.clone()) {
                out.ops.push(set(
                    &format!("entrants.characters.character[_metadata_={boss}]"),
                    "full_name",
                    key.clone(),
                ));
                out.ops.push(text(&key, &p.name));
            }
        }
    }
    for nf in &team.names {
        let Some(value) = edit
            .names
            .get(nf.part)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        if nf.shared || nf.key.is_empty() {
            let key = own_key(&team.label, nf.part);
            out.ops.push(set(&select, nf.field, key.clone()));
            out.ops.push(text(&key, value));
            out.lang_keys.insert(key);
        } else {
            out.ops.push(text(&nf.key, value));
            out.lang_keys.insert(nf.key.clone());
        }
    }
    // Konnersport and APXGP are offered from menus that name them, so the menu
    // text follows a new name.
    let name = |part: &str| edit.names.get(part).map(|v| v.trim()).filter(|v| !v.is_empty());
    match team.label.as_str() {
        "zenith_25" => {
            if let Some(short) = name("short") {
                out.ops.push(text("lng_team_zenith_title_short", short));
            }
            if let Some(full) = name("full") {
                out.ops
                    .push(text("lng_team_icons_menu_zenith", &format!("Add {full}")));
            }
            if name("full").is_some() || name("short").is_some() {
                out.ops.push(text("lng_team_icons_zenith", "11th Team Enabled"));
            }
        }
        "konnersport" => {
            if let Some(short) = name("short") {
                out.ops
                    .push(text("lng_team_icons_menu_konnersport", &format!("Add {short}")));
            }
            if name("full").is_some() || name("short").is_some() {
                out.ops
                    .push(text("lng_team_icons_konnersport", "11th Team Enabled"));
            }
        }
        _ => {}
    }
    for (field, hex) in &edit.colours {
        match encode_colour(hex) {
            Some(v) => out.ops.push(set(&select, field, v)),
            None => out
                .warnings
                .push(format!("{}: {hex} isn't a #RRGGBB colour", team.label)),
        }
    }
}

fn seat_ops(team: &Team, edits: &Edits, catalog: &Catalog, out: &mut Out) {
    for seat in &team.seats {
        let Some(edit) = edits.seats.get(&seat.key) else {
            continue;
        };
        if let Some(label) = &edit.driver {
            // A seat pointing at a driver the game doesn't have crashes it, such as one
            // from an addon that's off now: the seat keeps its own driver instead.
            let Some(driver) = catalog.drivers.get(label) else {
                out.warnings.push(format!(
                    "{}: {label} isn't in the game with this profile's addons, so the seat keeps its own driver",
                    team.label
                ));
                if let Some(n) = edit.number {
                    out.ops
                        .push(set(&seat.select, "car_number", n.clamp(0, 255).to_string()));
                }
                continue;
            };
            match driver.pool.clone() {
                // The driver's own entrant brings his helmet, suit and number with him.
                Some(pool) => {
                    out.ops.push(set(&seat.select, "entrant_reference", pool));
                    if !seat.override_driver.is_empty() {
                        out.ops.push(set(&seat.select, "driver", ""));
                    }
                }
                None => out.ops.push(set(&seat.select, "driver", label.clone())),
            }
        }
        if let Some(n) = edit.number {
            out.ops
                .push(set(&seat.select, "car_number", n.clamp(0, 255).to_string()));
        }
    }
}

/// A driver's own details: names under keys of their own, so a name another
/// driver shares stays his, plus code, nationality, number and the helmet in each
/// of his looks presets.
fn driver_ops(catalog: &Catalog, edits: &Edits, out: &mut Out) {
    for (label, e) in &edits.drivers {
        let Some(d) = catalog.drivers.get(label) else {
            out.warnings.push(format!(
                "{label} isn't in the game with this profile's addons, so his details are left alone"
            ));
            continue;
        };
        let select = format!("entrants.drivers.driver[_metadata_={label}]");
        if e.first.is_some() || e.last.is_some() {
            let first = e.first.clone().unwrap_or_else(|| d.first.clone());
            let last = e.last.clone().unwrap_or_else(|| d.last.clone());
            let own = |part: &str| format!("lng_sge_drv_{label}_{part}");
            let mut names = vec![
                ("first_name", own("first"), first.clone()),
                ("last_name", own("last"), last.clone()),
            ];
            if !d.standard_key.is_empty() {
                names.push(("last_name_standard", own("standard"), last.clone()));
            }
            if !d.full_key.is_empty() {
                names.push((
                    "full_name",
                    own("full"),
                    format!("{first} {last}").trim().to_owned(),
                ));
            }
            for (field, key, value) in names {
                out.ops.push(set(&select, field, key.clone()));
                out.ops.push(text(&key, &value));
                out.lang_keys.insert(key);
            }
        }
        if let Some(code) = e.code.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            out.ops.push(set(&select, "initials", code.to_uppercase()));
        }
        if let Some(nation) = &e.nation {
            out.ops.push(set(&select, "nationality", nation.clone()));
        }
        if (e.helmet.is_some() || !e.helmets.is_empty()) && d.looks.is_empty() {
            out.warnings
                .push(format!("{label} has no looks preset, so his helmet stays"));
        }
        for look in &d.looks {
            if let Some(helmet) = e.helmets.get(&look.label).or(e.helmet.as_ref()) {
                out.ops.push(set(&look.select, "helmet", helmet.clone()));
            }
        }
        if let Some(n) = e.number {
            let n = n.clamp(1, 99).to_string();
            out.ops.push(set(&select, "car_number", n.clone()));
            if d.seasonal {
                out.ops.push(set(
                    &format!("entrants.drivers.seasonal_infos.seasonal_info[_metadata_={label}]"),
                    "preferred_car_number",
                    n,
                ));
            }
        }
    }
}

pub fn placements(catalog: &Catalog, edits: &Edits, lib: &Library, data_dir: &Path) -> Vec<Placement> {
    let dir = logos_dir(data_dir);
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for grid in &catalog.grids {
        for team in grid.teams.iter().filter(|t| t.registry.is_none()) {
            let Some(edit) = edits.teams.get(&team.label) else {
                continue;
            };
            for kind in logos::all_kinds() {
                let Some(file) = edit.picture(kind) else { continue };
                for (array, slice) in lib.slots(kind, &grid.ui_path, &team.label, &team.timing) {
                    if seen.insert((array.clone(), slice)) {
                        out.push(Placement {
                            array,
                            slice,
                            picture: dir.join(file),
                            size: edit.size(kind),
                        });
                    }
                }
            }
        }
    }
    out
}

/// Every op for the edits that name rows the profile has. `logo_root` is where the
/// logo archives go and `logo_rel` their path from the ops' source root. With
/// `join`, logo arrays other addons left differing between archives are made one
/// again, which only the profile's own build wants, not an exported addon.
#[allow(clippy::too_many_arguments)]
pub fn all(
    catalog: &Catalog,
    edits: &Edits,
    lib: Option<&Library>,
    data_dir: &Path,
    logo_root: &Path,
    logo_rel: &str,
    cached: bool,
    join: bool,
) -> Out {
    let mut out = Out::default();
    let mut done = BTreeSet::new();
    for grid in &catalog.grids {
        for team in &grid.teams {
            if done.insert(team.label.clone()) {
                team_ops(team, edits, catalog, &mut out);
            }
            seat_ops(team, edits, catalog, &mut out);
        }
    }
    driver_ops(catalog, edits, &mut out);
    if let Some(lib) = lib {
        let places = placements(catalog, edits, lib, data_dir);
        if !places.is_empty() || join {
            let baked = logos::bake(lib, &places, logo_root, logo_rel, cached, join);
            out.ops.extend(baked.ops);
            out.lines.extend(baked.lines);
            out.warnings.extend(baked.warnings);
        }
    }
    out
}

/// The build reply, with only the op kinds the loader takes.
pub fn reply(mut out: Out, features: &Value) -> Value {
    let has = |kind: &str| {
        ["stable", "experimental"].iter().any(|k| {
            features[k]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(kind)))
        })
    };
    let before = out.ops.len();
    out.ops.retain(|op| {
        let kind = op["op"]
            .as_str()
            .unwrap_or_default()
            .split('.')
            .next()
            .unwrap_or_default();
        has(kind)
    });
    if out.ops.len() < before {
        out.warnings.push(format!(
            "{} op(s) left out: the in-game loader doesn't take them",
            before - out.ops.len()
        ));
    }
    let teams = out.ops.iter().filter(|o| o["op"] == "lang.set").count();
    out.lines
        .insert(0, format!("{} change(s), {teams} name(s)", out.ops.len()));
    json!({ "ops": out.ops, "lines": out.lines, "warnings": out.warnings })
}

/// The in-game addons page puts "by <author>, v<version>" and the description in
/// one menu string, which holds at most 255 bytes; a launch fails on a longer one.
pub const DESCRIPTION_MAX: usize = 180;

fn panel_safe(description: String) -> String {
    if description.len() <= DESCRIPTION_MAX {
        description
    } else {
        "A grid made with the Simplified Grid Editor.".into()
    }
}

/// Writes an addon that carries the edits on its own.
#[allow(clippy::too_many_arguments)]
pub fn export(
    out_dir: &Path,
    id: &str,
    name: &str,
    author: &str,
    catalog: &Catalog,
    edits: &Edits,
    lib: Option<&Library>,
    data_dir: &Path,
) -> Result<(PathBuf, usize), String> {
    let _ = std::fs::remove_dir_all(out_dir);
    std::fs::create_dir_all(out_dir.join("ops")).map_err(|e| e.to_string())?;
    let out = all(catalog, edits, lib, data_dir, out_dir, "logos", false, false);
    if out.ops.is_empty() {
        return Err("there's nothing to export: you haven't edited it".into());
    }
    let mut kinds = BTreeSet::new();
    for op in &out.ops {
        match op["op"].as_str().unwrap_or_default().split('.').next() {
            Some("db") => kinds.insert("database"),
            Some("lang") => kinds.insert("text"),
            Some("erp") => kinds.insert("erp"),
            _ => false,
        };
    }
    let manifest = json!({
        "schema_version": 1,
        "id": id,
        "name": name,
        "version": "1.0.0",
        "author": author,
        "description": panel_safe(format!("{name}, made with the Simplified Grid Editor.")),
        "kind": kinds,
        "compat": { "game": "f1_25", "launcher": ">=1.5.8" },
        "load_after": ["*"],
        "operations": { "assets": ["ops/grid.json"] },
        "origin": { "tool": "teamsimplified.grid-editor", "tool_version": env!("CARGO_PKG_VERSION") }
    });
    let write = |rel: &str, v: &Value| {
        let text = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
        std::fs::write(out_dir.join(rel), text).map_err(|e| e.to_string())
    };
    write("swofty.json", &manifest)?;
    write("ops/grid.json", &json!({ "schema_version": 1, "ops": out.ops }))?;
    Ok((out_dir.to_path_buf(), out.ops.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_fit_the_in_game_addons_page() {
        let m: Value = serde_json::from_str(include_str!("../../swofty.json")).unwrap();
        let text = |k: &str| m[k].as_str().unwrap().to_owned();
        let body = format!(
            "by {}, v{}\n{}",
            text("author"),
            text("version"),
            text("description")
        );
        assert!(body.len() <= 255, "{} bytes", body.len());
        assert!(text("description").len() <= DESCRIPTION_MAX);
        let long = panel_safe(format!(
            "{}, made with the Simplified Grid Editor.",
            "\u{1F3CE}".repeat(48)
        ));
        assert!(long.len() <= DESCRIPTION_MAX);
    }
}
