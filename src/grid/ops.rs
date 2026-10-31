// The edits as asset operations: what the build hook returns on Play, and what an
// exported addon carries.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::grid::db::Db;
use crate::grid::edits::{Edits, logos_dir};
use crate::grid::grids::{Catalog, Team, encode_colour};
use crate::grid::logos::{self, Library, Placement};
use crate::grid::world::{Progressive, World, progressive_year};

const HEAD_ROWS: &str = "entrants.animation.driver_heads";

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

/// More Driver Models' heads for the drivers another addon gives an older head of
/// its own, such as Classic Seasons' cs_jolyon_palmer: with the driver switched on
/// in More Driver Models, his row takes the head its file updates, under his own
/// label, wherever he races. Where More Driver Models' build makes no head row of
/// that label, as for a driver another addon already brings, the older row is
/// copied under it with the model path More Driver Models lists for him.
pub fn newer_heads(catalog: &Catalog, world: &World, out: &mut Out) {
    let Some(roster) = &world.roster else {
        return;
    };
    let mut moved = Vec::new();
    for d in catalog.drivers.values() {
        let older = world.head_origin.get(&d.head).is_some_and(|a| *a != roster.addon);
        let Some(mdm) = roster.drivers.get(&d.label).filter(|r| r.on) else {
            continue;
        };
        if d.head == d.label || !older || world.head_origin.get(&d.label) != Some(&roster.addon) {
            continue;
        }
        if world.db.find(HEAD_ROWS, "driver_head", &d.label).is_none() {
            let (Some(model), Some(_)) = (&mdm.model, world.db.find(HEAD_ROWS, "driver_head", &d.head))
            else {
                continue;
            };
            out.ops.push(json!({ "op": "db.add-row", "table": HEAD_ROWS,
                "from": format!("{HEAD_ROWS}.driver_head[_metadata_={}]", d.head),
                "set": { "_metadata_": d.label, "model_path": model } }));
        }
        let select = format!("entrants.drivers.driver[_metadata_={}]", d.label);
        out.ops.push(set(&select, "head", d.label.clone()));
        moved.push(d.label.clone());
    }
    if !moved.is_empty() {
        moved.sort();
        out.lines.push(format!(
            "{} driver(s) take {}'s head: {}",
            moved.len(),
            roster.addon,
            moved.join(", ")
        ));
    }
}

/// The seats edited on the grids Progressive Career plays, as seats of its seasons:
/// `{ "seats": { "2016": { "10": ["", "game/stoffel_vandoorne"] } } }`, by year and
/// career slot, "" keeping Progressive's driver. A seat is found by the driver the
/// grid had in it, where Progressive seats him that year. Simplified's DLL layers
/// these on Progressive's seasons when the game starts.
pub fn progressive_seats(
    catalog: &Catalog,
    edits: &Edits,
    world: &World,
    progressive: &Progressive,
    out: &mut Out,
) -> Value {
    let mut seasons: BTreeMap<String, BTreeMap<String, [String; 2]>> = BTreeMap::new();
    let mut moved = 0;
    let mut dressed = HashSet::new();
    for grid in &catalog.grids {
        let Some(year) = progressive_year(&grid.label) else {
            continue;
        };
        let Some(lineup) = progressive.seats.get(year) else {
            continue;
        };
        for seat in grid.teams.iter().flat_map(|t| &t.seats) {
            let Some(new) = edits.seats.get(&seat.key).and_then(|e| e.driver.as_ref()) else {
                continue;
            };
            if *new == seat.driver || !catalog.drivers.contains_key(new) {
                continue;
            }
            match lineup.get(&seat.driver) {
                Some((slot, place)) => {
                    seasons
                        .entry(year.to_owned())
                        .or_default()
                        .entry(slot.to_string())
                        .or_default()[*place] = identity(new, &world.stock_drivers);
                    dress(world, progressive, year, &seat.driver, new, &mut dressed, out);
                    moved += 1;
                }
                None => out.warnings.push(format!(
                    "{}: {} isn't in its {year} line-up, so {new} takes no seat there",
                    progressive.name, seat.driver
                )),
            }
        }
    }
    if moved > 0 {
        out.lines.push(format!(
            "{moved} seat(s) in {} {} season(s)",
            seasons.len(),
            progressive.name
        ));
    }
    json!({ "seats": seasons })
}

/// A driver Progressive has no look for in a year he takes a seat in wears his
/// default look there, which it makes from a 2025 McLaren look. That look takes the
/// suit and gloves the seat's own driver wears that year, and the helmet of his own
/// nearest year. He has one default look, so the first such seat dresses it.
fn dress(
    world: &World,
    progressive: &Progressive,
    year: &str,
    was: &str,
    new: &str,
    dressed: &mut HashSet<String>,
    out: &mut Out,
) {
    let Ok(y) = year.parse::<i64>() else {
        return;
    };
    let Some(mine) = progressive.looks.get(new) else {
        return;
    };
    if mine.years.contains_key(&y) {
        return;
    }
    let (Some((select, _)), Some((_, seat_look))) = (
        mine.own_default.as_deref().and_then(|l| look_row(&world.db, l)),
        progressive
            .looks
            .get(was)
            .and_then(|l| l.years.get(&y))
            .and_then(|l| look_row(&world.db, l)),
    ) else {
        return;
    };
    if !dressed.insert(new.to_owned()) {
        out.warnings.push(format!(
            "{}: {new} has one look for the years he didn't race, so he wears his first new seat's in {year} too",
            progressive.name
        ));
        return;
    }
    for field in ["suit", "gloves"] {
        out.ops
            .push(set(&select, field, world.db.field(&seat_look, field)));
    }
    let nearest = mine.years.iter().min_by_key(|(at, _)| (*at - y).abs());
    if let Some((_, own)) = nearest.and_then(|(_, l)| look_row(&world.db, l)) {
        out.ops
            .push(set(&select, "helmet", world.db.field(&own, "helmet")));
    }
}

/// A looks preset by label, in whichever folder holds it: its selector and its row.
fn look_row(db: &Db, label: &str) -> Option<(String, String)> {
    db.rows("entrants.driver_customisations", "folder").into_iter().find_map(|folder| {
        let row = db.find(&folder, "preloaded_driver_customisation_properties", label)?;
        let select = format!(
            "entrants.driver_customisations.folder[_metadata_={}].preloaded_driver_customisation_properties[_metadata_={label}]",
            db.field(&folder, "_metadata_")
        );
        Some((select, row))
    })
}

/// A driver as the registry names him, as Progressive does: the game's own as
/// `game/<label>`, another addon's as the person he is, with his row to fall back on.
fn identity(label: &str, stock: &HashSet<String>) -> String {
    match stock.contains(label) {
        true => format!("game/{label}"),
        false => format!("person/{label}|game/{label}"),
    }
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
    fn drivers_are_named_as_progressive_names_them() {
        let stock = HashSet::from(["fernando_alonso".to_owned()]);
        assert_eq!(identity("fernando_alonso", &stock), "game/fernando_alonso");
        assert_eq!(
            identity("stoffel_vandoorne", &stock),
            "person/stoffel_vandoorne|game/stoffel_vandoorne"
        );
    }

    /// Addons that require Simplified, such as More Driver Models, build after it, so
    /// a pattern matching them, `*` above all, is a load order cycle that stops Play.
    #[test]
    fn load_after_leaves_out_the_addons_that_need_simplified() {
        let m: Value = serde_json::from_str(include_str!("../../swofty.json")).unwrap();
        let after: Vec<&str> = m["load_after"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(after.contains(&"swofty.classic-seasons*") && after.contains(&"swofty.progressive"));
        for needs_us in ["mildtdesign.more-driver-models", "swofty.driver-manager"] {
            assert!(
                !after.iter().any(|p| match p.strip_suffix('*') {
                    Some(prefix) => needs_us.starts_with(prefix),
                    None => *p == needs_us,
                }),
                "{needs_us} builds after Simplified"
            );
        }
    }

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
