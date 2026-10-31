use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use swofty_formats::hash::fnv1a64;

use crate::grid::app::{App, Loaded, Page};
use crate::grid::edits::{self, Edits, Preset, TeamPreset, logos_dir};
use crate::grid::emblems;
use crate::grid::grids::{COLOUR_FIELDS, Catalog, Driver, Grid, Team, encode_colour};
use crate::grid::host::Host;
use crate::grid::image;
use crate::grid::logos::{self, Library};
use crate::grid::ops;
use crate::grid::world::{self, World};

pub const SECTION: &str = "grids";
const PRESETS_TAB: &str = "presets";
const DRIVERS_TAB: &str = "drivers";
const EMBLEM_TAB: &str = "emblem";
pub const MY_TEAM: &str = "myteam";

fn button(id: &str, label: &str, icon: &str, style: &str) -> Value {
    json!({ "id": id, "label": label, "icon": icon, "style": style })
}

fn pick(id: &str, label: &str, title: &str) -> Value {
    json!({ "id": id, "label": label, "icon": "upload", "style": "outline",
        "kind": { "pick": { "title": title, "extensions": image::PICTURES, "folders": false, "multiple": false } } })
}

fn confirm(mut b: Value, title: &str, text: &str) -> Value {
    b["kind"] = json!({ "confirm": { "title": title, "text": text } });
    b
}

fn text(t: &str, tone: &str) -> Value {
    json!({ "text": { "text": t, "tone": tone } })
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ── What the editor shows, edits over the live values ──────────────────────────

fn name_of(edits: &Edits, team: &Team, part: &str) -> String {
    edits
        .teams
        .get(&team.label)
        .and_then(|e| e.names.get(part))
        .cloned()
        .unwrap_or_else(|| team.name(part).to_owned())
}

fn colour_of(edits: &Edits, team: &Team, field: &str) -> Option<String> {
    edits
        .teams
        .get(&team.label)
        .and_then(|e| e.colours.get(field))
        .cloned()
        .or_else(|| team.colour(field).map(str::to_owned))
}

struct SeatView<'a> {
    driver: Option<&'a Driver>,
    label: String,
    number: i64,
    /// Last season's champion: the game gives him #1 whatever number the seat has.
    champion: bool,
}

/// A driver's own number, with the player's edit.
fn own_number(cat: &Catalog, edits: &Edits, label: &str) -> i64 {
    edits
        .drivers
        .get(label)
        .and_then(|e| e.number)
        .or_else(|| cat.drivers.get(label).map(|d| d.number))
        .unwrap_or(0)
}

/// A driver's first and last name, with the player's edits.
fn driver_names(edits: &Edits, d: &Driver) -> (String, String) {
    let e = edits.drivers.get(&d.label);
    (
        e.and_then(|e| e.first.clone()).unwrap_or_else(|| d.first.clone()),
        e.and_then(|e| e.last.clone()).unwrap_or_else(|| d.last.clone()),
    )
}

fn full_name(edits: &Edits, d: &Driver) -> String {
    let (first, last) = driver_names(edits, d);
    format!("{first} {last}").trim().to_owned()
}

/// The number a seat races with before the champion's #1: the one set on the seat,
/// else its driver's own.
fn seat_number(cat: &Catalog, edits: &Edits, seat: &crate::grid::grids::Seat, label: &str) -> i64 {
    let own = own_number(cat, edits, label);
    edits
        .seats
        .get(&seat.key)
        .and_then(|e| e.number)
        .unwrap_or(if (0..255).contains(&seat.car_number) {
            seat.car_number
        } else {
            own
        })
}

fn seat_view<'a>(
    cat: &'a Catalog,
    edits: &Edits,
    grid: &Grid,
    seat: &crate::grid::grids::Seat,
) -> SeatView<'a> {
    let label = edits
        .seats
        .get(&seat.key)
        .and_then(|e| e.driver.clone())
        .unwrap_or_else(|| seat.driver.clone());
    let champion = grid.champion.as_deref() == Some(label.as_str());
    let number = if champion {
        1
    } else {
        seat_number(cat, edits, seat, &label)
    };
    SeatView {
        driver: cat.drivers.get(&label),
        label,
        number,
        champion,
    }
}

fn driver_name(edits: &Edits, v: &SeatView) -> String {
    match v.driver {
        Some(d) => full_name(edits, d),
        None if v.label.is_empty() => "Empty seat".into(),
        None => v.label.clone(),
    }
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The picture a place shows for a team: the player's, else the slice as the
/// profile's addons leave it.
fn logo_for(
    app: &App,
    lib: &Library,
    grid: &Grid,
    team: &Team,
    kind: &str,
) -> Option<(PathBuf, Option<String>)> {
    if let Some(e) = app.edits.teams.get(&team.label)
        && let Some(file) = e.picture(kind)
        && let Some((w, h)) = slot_size(lib, grid, team, kind)
        && let Some(p) = placed_logo(app, file, w, h, e.size(kind))
    {
        return Some((p, Some("yours".into())));
    }
    let (array, slice) = lib
        .slots(kind, &grid.ui_path, &team.label, &team.timing)
        .into_iter()
        .next()?;
    let by = lib.changed_by(&array, slice).map(str::to_owned);
    Some((lib.preview(&array, slice, &app.cache_dir)?, by))
}

/// The player's picture as a place will hold it, at the slot's size and the size
/// they chose, cached by both.
fn placed_logo(app: &App, file: &str, w: u32, h: u32, size: u32) -> Option<PathBuf> {
    let stem = file.trim_end_matches(".png");
    let path = app
        .cache_dir
        .join("placed")
        .join(format!("{stem}-{w}x{h}-{size}.png"));
    if !path.is_file() {
        let picture = image::load_png(&logos_dir(&app.data_dir).join(file)).ok()?;
        image::save_png(&image::place(&picture, w, h, size, None), &path).ok()?;
    }
    Some(path)
}

/// The logo a team's card shows: the little timing tower one, else any.
/// A registry team's logo: its picture from the addon's registry, fitted and kept in
/// the cache, where the launcher shows pictures from.
fn registry_logo(app: &App, l: &Loaded, team: &Team) -> Option<PathBuf> {
    let src = l.world.registry_teams.get(&team.label)?.logo.as_ref()?;
    let len = src.metadata().ok()?.len();
    let path = app
        .cache_dir
        .join("registry")
        .join(format!("{}-{len:x}.png", team.label));
    if !path.is_file() {
        let img = image::load_picture(src, 256).ok()?;
        image::save_png(&img, &path).ok()?;
    }
    Some(path)
}

fn card_logo(app: &App, lib: &Library, grid: &Grid, team: &Team) -> Option<PathBuf> {
    ["standings", "team_select", "starting_grid", logos::SHOWROOM]
        .iter()
        .find_map(|k| logo_for(app, lib, grid, team, k))
        .map(|(p, _)| p)
}

/// The size a place's picture must be: its first array's slice size.
fn slot_size(lib: &Library, grid: &Grid, team: &Team, kind: &str) -> Option<(u32, u32)> {
    let (array, _) = lib
        .slots(kind, &grid.ui_path, &team.label, &team.timing)
        .into_iter()
        .next()?;
    lib.size(&array)
}

/// Purple for the player's own edit, amber for another addon's change.
fn toned(mut cell: Value, mine: bool, addons: &[String]) -> Value {
    if mine {
        cell["tone"] = json!("accent");
    } else if !addons.is_empty() {
        cell["tone"] = json!("warn");
    }
    cell
}

/// Cards four across that step down to two across, then one, as the page narrows.
/// The launcher drops a grid straight to one column once its columns can't each be
/// 280 pixels wide, so two grids of two sit in a grid of two: each pair goes to one
/// column first, then the pairs do.
fn four_up(cards: Vec<Value>) -> Value {
    if cards.len() <= 2 {
        return json!({ "grid": { "columns": 2, "blocks": cards } });
    }
    let half = cards.len().div_ceil(2);
    let (left, right) = cards.split_at(half);
    json!({ "grid": { "columns": 2, "blocks": [
        { "grid": { "columns": 2, "blocks": left } },
        { "grid": { "columns": 2, "blocks": right } } ] } })
}

/// The same as pills, for a field's card.
fn status_pills(mine: bool, addons: &[String]) -> Vec<Value> {
    if mine {
        return vec![json!({ "text": "Edited", "tone": "accent", "dot": true })];
    }
    addons
        .iter()
        .map(|a| json!({ "text": a, "tone": "warn", "dot": true }))
        .collect()
}

fn mine(edits: &Edits, team: &Team, part: &str) -> bool {
    edits
        .teams
        .get(&team.label)
        .is_some_and(|e| e.names.contains_key(part) || e.colours.contains_key(part))
}

fn restore_button(team: &Team, part: &str) -> Value {
    button(
        &format!("restore:{}:{part}", team.label),
        "Restore",
        "undo",
        "ghost",
    )
}

/// Puts a card's status pills under its content: in the header they squeeze its
/// title and subtitle.
fn pills_below(card: &mut Value, pills: Vec<Value>) {
    if let (false, Some(blocks)) = (pills.is_empty(), card["card"]["blocks"].as_array_mut()) {
        blocks.push(json!({ "pills": pills }));
    }
}

/// A field in a card of its own: its title, what it reads now underneath, a
/// restore button beside the title once it's edited, and the input below.
fn field_card(
    title: &str,
    subtitle: &str,
    pills: Vec<Value>,
    restore: Option<Value>,
    blocks: Vec<Value>,
) -> Value {
    let mut card = json!({ "card": { "title": title, "subtitle": subtitle, "blocks": blocks } });
    pills_below(&mut card, pills);
    if let Some(b) = restore {
        card["card"]["buttons"] = json!([b]);
    }
    card
}

// ── The page ────────────────────────────────────────────────────────────────────

/// A faint line across the sidebar between its groups: a wide picture one pixel high,
/// which the launcher narrows to the column. Grey at half strength shows on either theme.
fn rule(app: &App) -> Option<Value> {
    let path = app.cache_dir.join("dots").join("rule.png");
    if !path.is_file() {
        let rgba = [128u8, 128, 128, 90].repeat(4);
        image::save_png(
            &image::Image {
                width: 4,
                height: 1,
                rgba,
            },
            &path,
        )
        .ok()?;
    }
    Some(
        json!({ "image": { "path": path_str(&path), "width": 1000, "height": 1, "fit": "cover", "transparent": true } }),
    )
}

fn nav_button(id: &str, label: &str, active: bool, edited: usize) -> Value {
    let label = if edited > 0 {
        format!("{label} · {edited}")
    } else {
        label.to_owned()
    };
    json!({ "buttons": [ { "id": id, "label": label, "style": if active { "primary" } else { "ghost" } } ] })
}

/// The grids down the left: the game's own, then the ones addons add, then presets.
fn nav(app: &App, cat: &Catalog) -> Value {
    let on = |g: &Grid| matches!(app.page, Page::Grid | Page::Team(_)) && app.grid == g.label;
    let grid_button = |g: &Grid| {
        nav_button(
            &format!("nav:{}", g.label),
            &g.title,
            on(g),
            app.edits.count_in(g),
        )
    };
    let sub = |t: &str| text(t, "muted");
    let mut blocks = vec![json!({ "label": "DEFAULT" })];
    for (title, prefix) in [("Formula 1", "f1_"), ("Formula 2", "f2_")] {
        let grids: Vec<&Grid> = cat
            .grids
            .iter()
            .filter(|g| g.origin.is_none() && g.label.starts_with(prefix))
            .collect();
        if !grids.is_empty() {
            blocks.push(sub(title));
            blocks.extend(grids.into_iter().map(grid_button));
        }
    }
    let rest: Vec<&Grid> = cat
        .grids
        .iter()
        .filter(|g| g.origin.is_none() && !g.label.starts_with("f1_") && !g.label.starts_with("f2_"))
        .collect();
    blocks.extend(rest.into_iter().map(grid_button));
    // The grids addons add. Classic Seasons is an addon per year, so they're one group.
    blocks.extend(rule(app));
    blocks.push(json!({ "label": "OTHER" }));
    let added: Vec<&Grid> = cat.grids.iter().filter(|g| g.origin.is_some()).collect();
    if added.is_empty() {
        blocks.push(text(
            "Install and activate addons, such as Classic Seasons, to show up and edit.",
            "muted",
        ));
    } else {
        blocks.push(sub("Addon"));
        blocks.extend(added.into_iter().map(grid_button));
    }
    blocks.extend(rule(app));
    blocks.push(json!({ "label": "MISC" }));
    blocks.push(nav_button("nav:presets", "Presets", app.page == Page::Presets, 0));
    blocks.push(nav_button(
        "nav:drivers",
        "Driver Profiles",
        matches!(app.page, Page::Driver { .. }),
        app.edits.drivers.len(),
    ));
    blocks.push(nav_button(
        "nav:emblem",
        "My Team Emblem",
        app.page == Page::Emblem,
        0,
    ));
    json!({ "card": { "blocks": blocks } })
}

/// Whether the drivers lists offer a driver: the game's own and More Driver Models'
/// ones. Other addons' drivers, such as Classic Seasons' past drivers, only show
/// where they already race.
fn offered(d: &Driver, current: &str) -> bool {
    d.label == current
        || d.origin
            .as_deref()
            .is_none_or(|a| a.to_lowercase().contains("more driver models"))
}

/// The addons the offered drivers come from, More Driver Models first.
fn sort_addons(addons: &mut Vec<&str>) {
    addons.sort_unstable_by_key(|a| (!a.to_lowercase().contains("more driver models"), a.to_lowercase()));
    addons.dedup();
}

fn sorted(mut list: Vec<&Driver>) -> Vec<&Driver> {
    list.sort_by_key(|d| d.name().to_lowercase());
    list
}

/// The drivers a seat can take: the game's own with a head model, then More Driver
/// Models'. A seat's own driver stays in its list even with a generic face.
fn driver_options(cat: &Catalog, edits: &Edits, current: &str, taken: &[(String, String)]) -> Vec<Value> {
    let option = |d: &Driver| json!({ "value": d.label, "label": full_name(edits, d) });
    // The drivers racing elsewhere on the grid come first, by their team: picking one
    // swaps the two seats, as a driver races in one seat of a grid. The rest are the
    // ones free to come in.
    let mut out = Vec::new();
    let others: Vec<Value> = taken
        .iter()
        .filter(|(label, _)| label != current)
        .filter_map(|(label, team)| {
            let d = cat.drivers.get(label)?;
            Some(json!({ "value": d.label, "label": format!("{} · {team} (swap)", full_name(edits, d)) }))
        })
        .collect();
    if !others.is_empty() {
        out.push(json!({ "heading": "On this grid" }));
        out.extend(others);
    }
    let free = |d: &&Driver| {
        offered(d, current) && (d.label == current || !taken.iter().any(|(l, _)| *l == d.label))
    };
    out.push(json!({ "heading": "Default" }));
    out.extend(
        sorted(
            cat.drivers
                .values()
                .filter(free)
                .filter(|d| d.has_model && d.origin.is_none())
                .collect(),
        )
        .into_iter()
        .map(option),
    );
    let mut addons: Vec<&str> = cat
        .drivers
        .values()
        .filter(free)
        .filter(|d| d.has_model)
        .filter_map(|d| d.origin.as_deref())
        .collect();
    sort_addons(&mut addons);
    for addon in addons {
        out.push(json!({ "heading": addon }));
        out.extend(
            sorted(
                cat.drivers
                    .values()
                    .filter(free)
                    .filter(|d| d.has_model && d.origin.as_deref() == Some(addon))
                    .collect(),
            )
            .into_iter()
            .map(option),
        );
    }
    let rest = sorted(
        cat.drivers
            .values()
            .filter(free)
            .filter(|d| !d.has_model && d.label == current)
            .collect(),
    );
    if !rest.is_empty() {
        out.push(json!({ "heading": "Generic faces" }));
        out.extend(rest.into_iter().map(option));
    }
    out
}

const NAME_PARTS: [(&str, &str, u32); 3] = [
    ("full", "Title", 64),
    ("short", "Manufacturer", 32),
    ("super_short", "Short Name", 24),
];

/// A team's logos, place by place: the team's page. Names, line-up, principal and
/// colours are edited in the team's row on the grid page.
fn logo_detail(app: &App, l: &Loaded, grid: &Grid, team: &Team) -> Vec<Value> {
    let mut blocks = Vec::new();
    if team.optional.is_some() {
        blocks.push(json!({ "pills": [ { "text": format!("Optional team on {}", grid.title), "tone": "muted", "icon": "plus" } ] }));
        blocks.push(text(
            "The game adds it to this grid when you turn it on in its menus.",
            "muted",
        ));
    }
    if let Some(lib) = &l.lib {
        blocks.push(json!({ "label": "LOGO" }));
        let team_name = name_of(&app.edits, team, "full");
        blocks.push(json!({ "buttons": [ pick(&format!("logo:{}:all", team.label), "Choose one picture for every place…",
            &format!("Logo for {team_name}: PNG, JPEG or SVG")) ] }));
        blocks.push(text("PNG, JPEG or SVG of any size: it's fitted into each place without stretching. A logo on a transparent background works best.", "muted"));
        let mut cards = Vec::new();
        for kind in logos::all_kinds() {
            let Some((sw, sh)) = slot_size(lib, grid, team, kind) else {
                continue;
            };
            let label = logos::kind_label(kind);
            let edit = app.edits.teams.get(&team.label);
            let edited = edit.is_some_and(|e| e.picture(kind).is_some());
            let mut inner = Vec::new();
            let mut by = None;
            if let Some((path, who)) = logo_for(app, lib, grid, team, kind) {
                // At the slot's own size, as the game draws it.
                inner.push(json!({ "image": { "path": path_str(&path), "width": sw, "height": sh, "fit": "contain", "transparent": true } }));
                by = who.filter(|w| w != "yours");
            }
            if let Some(e) = edit.filter(|_| edited) {
                inner.push(json!({ "form": { "id": format!("logo-size:{}:{kind}", team.label), "live": true, "fields": [
                    { "slider": { "id": "size", "label": "Logo size", "value": e.size(kind), "min": 20, "max": 100, "step": 5, "unit": "%" } } ] } }));
            }
            let mut buttons = vec![pick(
                &format!("logo:{}:{kind}", team.label),
                "Choose…",
                &format!("{label} logo for {team_name}: PNG, JPEG or SVG"),
            )];
            if edited {
                buttons.push(restore_button(team, &format!("logo-{kind}")));
            }
            let mut card = json!({ "card": { "title": label, "subtitle": format!("{sw} × {sh}"), "buttons": buttons, "blocks": inner } });
            pills_below(&mut card, status_pills(edited, by.as_slice()));
            cards.push(card);
        }
        if !cards.is_empty() {
            blocks.push(four_up(cards));
        } else {
            blocks.push(text(
                "This team has no logo slot the editor knows. Play once if an addon added the team.",
                "muted",
            ));
        }
    }
    blocks
}

/// A colour as a round dot, cached by colour and size.
fn dot(app: &App, hex: &str, size: u32) -> Option<String> {
    let h = hex.trim_start_matches('#').to_uppercase();
    let path = app.cache_dir.join("dots").join(format!("{h}-{size}.png"));
    if !path.is_file() {
        image::save_png(&image::dot(&h, size)?, &path).ok()?;
    }
    Some(path_str(&path))
}

/// A driver's More Driver Models portrait, copied small into the cache: the launcher
/// shows pictures from the addon's own folders only.
fn portrait(app: &App, l: &Loaded, d: &Driver) -> Option<String> {
    let src = l
        .portraits
        .get(&d.head.to_lowercase())
        .or_else(|| l.portraits.get(&d.label))?;
    let len = src.metadata().ok()?.len();
    let stem = src.file_stem()?.to_string_lossy();
    let path = app
        .cache_dir
        .join("portraits")
        .join(format!("{stem}-{len:x}.png"));
    if !path.is_file() {
        let img = image::load_png(src).ok()?;
        image::save_png(&image::fit(&img, 128, 128), &path).ok()?;
    }
    Some(path_str(&path))
}

/// What a clicked row shows: the team's logo, its names and race numbers to edit, and
/// the way into its page. No long lists, so an open row stays quick to redraw.
fn row_detail(app: &App, l: &Loaded, grid: &Grid, team: &Team, presets: &[(String, Preset)]) -> Vec<Value> {
    let cat = &l.catalog;
    let w = &l.world;
    let id = |verb: &str| format!("team-form:{}:{verb}", team.label);
    // Under the panel's title, as close as a pill gets to it: the Status column's word.
    let status = status_cell(app, l, grid, team);
    let mut pills = vec![json!({ "text": status["text"], "tone": status["tone"], "dot": true })];
    if team.optional.is_some() {
        pills.push(json!({ "text": "Optional team", "tone": "muted", "icon": "plus" }));
    }
    let mut blocks = vec![json!({ "pills": pills })];
    // The preset and addon actions in one row: their labels short enough to fit the panel.
    blocks.push(json!({ "buttons": [
        { "id": id("save"), "label": "Save preset", "icon": "save", "style": "outline" },
        { "id": id("export"), "label": "Export addon", "icon": "package", "style": "outline" },
        { "id": id("replace"), "label": "Import preset", "icon": "folder-open", "style": "outline" } ] }));
    if let Some((_, which)) = app.team_form.as_ref().filter(|(t, _)| *t == team.label) {
        blocks.extend(team_form(app, team, which, presets));
    }
    // A registry team: its picture, and what can be edited.
    if let Some(addon) = &team.registry {
        if let Some(p) = registry_logo(app, l, team) {
            blocks.push(json!({ "image": { "path": path_str(&p), "width": 148, "height": 148, "fit": "contain", "transparent": true } }));
        }
        blocks.push(text(
            &format!("{addon} makes this team when the game starts, so its name, colours, logo and principal are that addon's. Its drivers and car numbers can be edited here."),
            "muted",
        ));
    }
    // The team selection logo, at most the panel's width, and the way to the logos page.
    if team.registry.is_none()
        && let Some(lib) = l.lib.as_ref()
        && let Some((path, _)) = logo_for(app, lib, grid, team, "team_select")
        && let Some((sw, sh)) = slot_size(lib, grid, team, "team_select")
    {
        let scale = (340.0 / sw as f32).min(160.0 / sh as f32).min(1.0);
        blocks.push(
            json!({ "image": { "path": path_str(&path), "width": (sw as f32 * scale).round() as u32,
            "height": (sh as f32 * scale).round() as u32, "fit": "contain", "transparent": true } }),
        );
    }
    if team.registry.is_none() {
        blocks.push(json!({ "buttons": [
            { "id": format!("edit-team:{}", team.label), "label": "Replace logos", "icon": "pencil", "style": "primary" } ] }));
    }

    // The names, race numbers and colours, saved as they're changed like on the team's page.
    blocks.push(json!({ "label": "TEAM NAME" }));
    let names: Vec<Value> = NAME_PARTS
        .iter()
        .map(|(part, title, max)| {
            let original = team.name(part);
            let mut field = json!({ "id": part, "label": title, "value": name_of(&app.edits, team, part),
                "placeholder": original, "max_len": max });
            if mine(&app.edits, team, part) {
                field["help"] = json!(format!("Original: {original}"));
            }
            json!({ "text": field })
        })
        .collect();
    blocks.push(
        json!({ "form": { "id": format!("names:{}", team.label), "live": true, "fields": names,
        "disabled": team.registry.is_some() } }),
    );

    if !team.seats.is_empty() {
        blocks.push(json!({ "label": "DRIVERS" }));
        // A table, as a form's fields split its width evenly: the name wide, the
        // number just wide enough for two digits. Clicking a driver opens his actions
        // under him; the long list of drivers only comes with Replace driver.
        let mut rows = Vec::new();
        for (i, s) in team.seats.iter().enumerate() {
            let v = seat_view(cat, &app.edits, grid, s);
            let name_text = driver_name(&app.edits, &v);
            let mut name = json!({ "text": name_text });
            if let Some(p) = v.driver.and_then(|d| portrait(app, l, d)) {
                name["image"] = json!({ "path": p, "width": 26, "height": 26 });
            }
            let edited = app.edits.seats.get(&s.key).is_some_and(|e| e.number.is_some());
            let number = toned(
                json!({ "text": v.number.to_string(), "edit": { "number": { "min": 0, "max": 99, "step": 1 } } }),
                edited,
                World::changed_by(&w.seats, &s.row, "number"),
            );
            let mut actions = Vec::new();
            if let Some(d) = v.driver {
                actions.push(button(
                    &format!("edit-driver:{}", d.label),
                    "Edit driver",
                    "pencil",
                    "outline",
                ));
            }
            let picking = app
                .seat_pick
                .as_ref()
                .is_some_and(|(t, n)| *t == team.label && *n == i);
            let mut detail = Vec::new();
            if picking {
                // The grid's drivers in its order, with the team each drives for.
                let taken: Vec<(String, String)> = grid
                    .teams
                    .iter()
                    .flat_map(|t| t.seats.iter().map(move |s| (t, s)))
                    .map(|(t, s)| {
                        (
                            seat_view(cat, &app.edits, grid, s).label,
                            name_of(&app.edits, t, "full"),
                        )
                    })
                    .collect();
                detail.push(json!({ "form": { "id": format!("seats:{}:d{i}", team.label), "live": true, "fields": [
                    { "select": { "id": format!("driver:{i}"), "label": format!("Driver {}", i + 1), "value": v.label, "search": true,
                        "options": driver_options(cat, &app.edits, &v.label, &taken) } } ] } }));
                actions.push(json!({ "id": "replace-seat-close", "label": "Cancel", "style": "ghost" }));
            } else {
                actions.push(button(
                    &format!("replace-seat:{}:{i}", team.label),
                    "Replace driver",
                    "arrow-right-left",
                    "outline",
                ));
            }
            if app.edits.seats.get(&s.key).is_some_and(|e| !e.is_empty()) {
                actions.push(button(
                    &format!("restore:{}:seat{i}", team.label),
                    "Restore",
                    "undo",
                    "ghost",
                ));
            }
            detail.insert(0, json!({ "buttons": actions }));
            rows.push(json!({ "id": i.to_string(), "cells": [name, number],
                "title": name_text, "subtitle": if v.champion { format!("Driver {} · #1, last season's champion", i + 1) } else { format!("Driver {} · #{}", i + 1, v.number) },
                "detail": detail }));
        }
        blocks.push(json!({ "table": {
            "id": format!("seat-cells:{}", team.label),
            "columns": [ { "key": "driver", "title": "Driver" }, { "key": "number", "title": "#", "width": 64, "align": "center" } ],
            "rows": rows,
            "row_height": 34
        } }));
    }

    if team.registry.is_none() {
        blocks.extend(principal_blocks(app, cat, team));
    }

    // Colour cells draw their own dot and open a picker, lighter than colour fields.
    blocks.push(json!({ "label": "COLOURS" }));
    let rows: Vec<Value> = COLOUR_FIELDS
        .iter()
        .map(|(field, title)| {
            let hex = colour_of(&app.edits, team, field).unwrap_or_default();
            let c = match team.registry {
                Some(_) => {
                    let mut c = json!({ "text": hex });
                    if let Some(p) = dot(app, &hex, 32) {
                        c["image"] = json!({ "path": p, "width": 14, "height": 14, "fit": "contain", "transparent": true });
                    }
                    c
                }
                None => json!({ "text": hex, "edit": { "colour": { "value": hex } } }),
            };
            let c = toned(
                c,
                mine(&app.edits, team, field),
                World::changed_by(&w.touched, &team.label, field),
            );
            json!({ "id": field, "cells": [ { "text": title }, c ] })
        })
        .collect();
    blocks.push(json!({ "table": {
        "id": format!("colours:{}", team.label),
        "columns": [ { "key": "colour", "title": "Colour" }, { "key": "value", "title": "Value" } ],
        "rows": rows,
        "compact": true
    } }));

    blocks
}

/// The team principal and a way to change him; the list of characters only once asked
/// for.
fn principal_blocks(app: &App, cat: &Catalog, team: &Team) -> Vec<Value> {
    let edit = app.edits.teams.get(&team.label).and_then(|e| e.boss.clone());
    if team.boss.is_empty() && edit.is_none() {
        return Vec::new();
    }
    let current = edit.clone().unwrap_or_else(|| team.boss.clone());
    let mut cell = json!({ "text": cat.principal_name(&current) });
    if edit.is_some() {
        cell["tone"] = json!("accent");
    }
    // Clicking him opens his actions under him; the list only comes with Replace.
    let picking = app.boss_pick.as_deref() == Some(team.label.as_str());
    let mut detail = Vec::new();
    let mut actions = vec![if picking {
        json!({ "id": "replace-boss-close", "label": "Cancel", "style": "ghost" })
    } else {
        button(
            &format!("replace-boss:{}", team.label),
            "Replace team principal",
            "arrow-right-left",
            "outline",
        )
    }];
    if edit.is_some() {
        actions.push(button(
            &format!("restore:{}:boss", team.label),
            "Restore",
            "undo",
            "ghost",
        ));
    }
    detail.push(json!({ "buttons": actions }));
    if picking {
        let options: Vec<Value> = cat
            .principals
            .iter()
            .map(|p| json!({ "value": p.label, "label": p.name }))
            .collect();
        detail.push(json!({ "form": { "id": format!("boss:{}", team.label), "live": true, "fields": [
            { "select": { "id": "boss", "label": "Team principal", "value": current, "search": true, "options": options,
                "help": format!("Original: {}", cat.principal_name(&team.boss)) } } ] } }));
    }
    vec![
        json!({ "label": "TEAM PRINCIPAL" }),
        json!({ "table": { "id": format!("boss-cells:{}", team.label),
            "columns": [ { "key": "name", "title": "Name" } ],
            "rows": [ { "id": "boss", "cells": [cell], "title": cat.principal_name(&current),
                "subtitle": "Team principal", "detail": detail } ], "row_height": 34 } }),
    ]
}

/// The name or preset form under a team row's buttons, with a way to close it.
fn team_form(app: &App, team: &Team, which: &str, presets: &[(String, Preset)]) -> Vec<Value> {
    let name = name_of(&app.edits, team, "full");
    let cancel = json!({ "buttons": [ { "id": "team-form-close", "label": "Cancel", "style": "ghost" } ] });
    let name_form = |id: &str, apply: &str, label: &str| {
        json!({ "form": { "id": format!("{id}:{}", team.label), "apply": apply, "fields": [
        { "text": { "id": "name", "label": label, "value": "", "placeholder": name, "max_len": 48 } } ] } })
    };
    match which {
        "save" => vec![
            text(
                "Keeps this team's names, colours, logos and line-up, to put back later or on another team.",
                "muted",
            ),
            name_form("save-team", "Save preset", "Preset name"),
            cancel,
        ],
        "export" => vec![
            text(
                "Writes this team's edits as an addon of its own and installs it. Turn the edits off here afterwards, or both apply.",
                "muted",
            ),
            name_form("export-team", "Export and install", "Addon name"),
            cancel,
        ],
        _ => {
            let options: Vec<Value> = presets
                .iter()
                .filter(|(_, p)| p.scope == "team")
                .map(|(slug, p)| json!({ "value": slug, "label": p.name }))
                .collect();
            if options.is_empty() {
                return vec![
                    text("No team presets yet. Save one with Save preset first.", "muted"),
                    cancel,
                ];
            }
            vec![
                text(
                    "Puts a saved team's names, colours, logos and line-up on this team, in place of your edits to it.",
                    "muted",
                ),
                json!({ "form": { "id": format!("apply-team:{}", team.label), "apply": "Import",
                    "fields": [ { "select": { "id": "preset", "label": "Team preset", "value": "", "options": options } } ] } }),
                cancel,
            ]
        }
    }
}

/// The addons that change a team: its names and colours, its seats or its logos.
fn team_addons(l: &Loaded, grid: &Grid, team: &Team) -> Vec<String> {
    let w = &l.world;
    let mut out = w.writers(&team.label);
    if let Some(a) = &team.registry
        && !out.contains(a)
    {
        out.push(a.clone());
    }
    let mut add = |a: &str| {
        if !out.iter().any(|x| x == a) {
            out.push(a.to_owned());
        }
    };
    for s in &team.seats {
        for part in ["driver", "number"] {
            for a in World::changed_by(&w.seats, &s.row, part) {
                add(a);
            }
        }
    }
    if let Some(lib) = l.lib.as_ref().filter(|lib| !lib.is_added(&team.label)) {
        for kind in logos::all_kinds() {
            for (array, slice) in lib.slots(kind, &grid.ui_path, &team.label, &team.timing) {
                if let Some(a) = lib.changed_by(&array, slice) {
                    add(a);
                }
            }
        }
    }
    out
}

/// Whether a team is as the game has it, edited by the player, or changed by an
/// addon: the player's edits go over an addon's, so they show first.
fn status_cell(app: &App, l: &Loaded, grid: &Grid, team: &Team) -> Value {
    let cat = &l.catalog;
    let mine = app.edits.team_edited(team)
        || team.seats.iter().any(|s| {
            app.edits
                .drivers
                .contains_key(&seat_view(cat, &app.edits, grid, s).label)
        });
    let addons = team_addons(l, grid, team);
    let (text, tone, sort) = if mine {
        (
            if addons.is_empty() {
                "Edited".to_owned()
            } else {
                "Edited · over addon".to_owned()
            },
            "accent",
            "0",
        )
    } else if !addons.is_empty() {
        ("Addon".to_owned(), "warn", "1")
    } else {
        ("Default".to_owned(), "muted", "2")
    };
    json!({ "text": text, "tone": tone, "pill": true, "sort": format!("{sort}{text}") })
}

/// The open grid's teams in a table: the UI colour, the timing tower logo, the name
/// and both drivers. Clicking a row shows the way into the team's page.
fn grid_page(app: &App, l: &Loaded, grid: &Grid, presets: &[(String, Preset)]) -> Vec<Value> {
    let cat = &l.catalog;
    let lib = l.lib.as_ref();
    let w = &l.world;
    let mut rows = Vec::new();
    for team in &grid.teams {
        let colour = colour_of(&app.edits, team, "ui_team_colour").unwrap_or_default();
        let mut ui_cell = json!({ "text": "", "sort": colour });
        if let Some(p) = dot(app, &colour, 32) {
            ui_cell["image"] =
                json!({ "path": p, "width": 18, "height": 18, "fit": "contain", "transparent": true });
        }
        let mut logo_cell = json!({ "text": "" });
        let logo = match team.registry {
            Some(_) => registry_logo(app, l, team),
            None => lib.and_then(|lib| card_logo(app, lib, grid, team)),
        };
        if let Some(p) = logo {
            logo_cell["image"] = json!({ "path": path_str(&p), "width": 28, "height": 28, "fit": "contain", "transparent": true });
        }
        let team_cell = toned(
            json!({ "text": name_of(&app.edits, team, "full") }),
            mine(&app.edits, team, "full"),
            World::changed_by(&w.touched, &team.label, "full"),
        );
        let mut cells = vec![ui_cell, logo_cell, team_cell];
        for i in 0..2 {
            let Some(seat) = team.seats.get(i) else {
                cells.push(json!({ "text": "", "tone": "muted" }));
                continue;
            };
            let v = seat_view(cat, &app.edits, grid, seat);
            let name = driver_name(&app.edits, &v);
            let mut spans = vec![json!({ "text": format!("#{}  ", v.number), "tone": "muted" })];
            match v.driver {
                Some(d) => {
                    let (first, last) = driver_names(&app.edits, d);
                    if first.is_empty() {
                        spans.push(json!({ "text": name }));
                    } else {
                        spans.push(json!({ "text": format!("{first} ") }));
                        spans.push(json!({ "text": last.to_uppercase(), "bold": true }));
                    }
                }
                None => spans.push(json!({ "text": name })),
            }
            let edited = app.edits.seats.get(&seat.key).is_some_and(|e| !e.is_empty())
                || app.edits.drivers.contains_key(&v.label);
            let mut addons = World::changed_by(&w.seats, &seat.row, "driver").to_vec();
            addons.extend(World::changed_by(&w.seats, &seat.row, "number").iter().cloned());
            let mut cell = toned(
                json!({ "text": format!("#{} {name}", v.number), "spans": spans, "sort": name,
                    "tone": if v.driver.is_some_and(|d| !d.has_model) { "muted" } else { "neutral" } }),
                edited,
                &addons,
            );
            if let Some(p) = v.driver.and_then(|d| portrait(app, l, d)) {
                cell["image"] = json!({ "path": p, "width": 30, "height": 30 });
            }
            cells.push(cell);
        }
        cells.push(status_cell(app, l, grid, team));
        rows.push(json!({
            "id": team.label,
            "title": name_of(&app.edits, team, "full"),
            "subtitle": format!("{} · {}", team.label, grid.title),
            "cells": cells,
            "detail": row_detail(app, l, grid, team, presets),
        }));
    }
    let mut blocks = Vec::new();
    let placeholder = format!("My {}", grid.title);
    let form = match app.grid_form.as_deref() {
        Some("save") => Some((
            "Save grid as preset",
            "save-grid",
            "Save preset",
            "Name",
            "Keeps every team's names, colours, logos and line-up of this grid, to put back later or on another grid.",
        )),
        Some("export") => Some((
            "Export grid as addon",
            "export",
            "Export and install",
            "Addon name",
            "Writes this grid's edits as an addon of its own and installs it, to share or to turn on per profile. Turn the edits off here afterwards, or both apply.",
        )),
        _ => None,
    };
    if let Some((title, id, apply, label, help)) = form {
        blocks.push(json!({ "card": { "title": title, "subtitle": help,
            "buttons": [ { "id": "grid-form-close", "label": "Cancel", "style": "ghost" } ],
            "blocks": [ { "form": { "id": format!("{id}:{}", grid.label), "apply": apply, "fields": [
                { "text": { "id": "name", "label": label, "value": "", "placeholder": placeholder, "max_len": 48 } } ] } } ] } }));
    }
    blocks.push(json!({ "pills": [
        { "text": "Edited by You", "tone": "accent", "dot": true },
        { "text": "Edited by Addons", "tone": "warn", "dot": true } ] }));
    for (d, seats) in doubles(&app.edits, grid) {
        let name = cat
            .drivers
            .get(&d)
            .map_or_else(|| d.clone(), |x| full_name(&app.edits, x));
        let teams: Vec<String> = seats
            .iter()
            .filter_map(|s| grid.teams.iter().find(|t| t.seats.iter().any(|x| x.key == s.key)))
            .map(|t| name_of(&app.edits, t, "full"))
            .collect();
        blocks.push(text(&format!("{name} is in more than one seat: {}. A driver races for one team; replace him in one of them.", teams.join(" and ")), "warn"));
    }
    blocks.push(json!({ "table": {
        "id": format!("teams:{}", grid.label),
        "columns": [
            { "key": "ui", "title": "UI", "width": 44, "align": "center", "sort": true },
            { "key": "logo", "title": "Logo", "width": 52, "align": "center" },
            { "key": "team", "title": "Team name", "sort": true },
            { "key": "d1", "title": "Driver 1", "sort": true },
            { "key": "d2", "title": "Driver 2", "sort": true },
            { "key": "status", "title": "Status", "width": 150, "sort": true }
        ],
        "rows": rows,
        "row_height": 44
    } }));
    let mut buttons = vec![
        button("grid-form:save", "Save grid as preset", "save", "outline"),
        button("grid-form:export", "Export grid as addon", "package", "outline"),
    ];
    if app.edits.count_in(grid) > 0 {
        buttons.push(confirm(
            button(&format!("restore-grid:{}", grid.label), "Restore grid to default", "undo", "outline"),
            &format!("Restore {} to default?", grid.title),
            "Your edits to every team on this grid go. What the profile's addons change stays, and your presets are kept.",
        ));
    }
    let mut card = json!({ "title": grid.title, "buttons": buttons, "blocks": blocks });
    if let Some(a) = l.world.list_origin.get(&grid.label) {
        card["subtitle"] = json!(format!("From {a}"));
    }
    vec![json!({ "card": card })]
}

/// One team on a page of its own, the way back to the grid first.
fn team_page(app: &App, l: &Loaded, grid: &Grid, team: &Team) -> Vec<Value> {
    let mut blocks = vec![
        json!({ "buttons": [ { "id": "back", "label": format!("← Back to all teams of {}", grid.title), "style": "primary" } ] }),
    ];
    blocks.extend(logo_detail(app, l, grid, team));
    let subtitle = format!(
        "{} · {} · every change is saved as you make it and applies when you press Play",
        team.label, grid.title
    );
    vec![
        json!({ "card": { "title": name_of(&app.edits, team, "full"), "subtitle": subtitle, "blocks": blocks } }),
    ]
}

/// The drivers a driver page can switch to, like a seat's list but every driver.
fn every_driver(cat: &Catalog, edits: &Edits, current: &str) -> Vec<Value> {
    let option = |d: &Driver| {
        let name = full_name(edits, d);
        let label = if edits.drivers.contains_key(&d.label) {
            format!("{name} · edited")
        } else {
            name
        };
        json!({ "value": d.label, "label": label })
    };
    let mut out = vec![json!({ "heading": "Default" })];
    out.extend(
        sorted(
            cat.drivers
                .values()
                .filter(|d| d.has_model && d.origin.is_none())
                .collect(),
        )
        .into_iter()
        .map(option),
    );
    let mut addons: Vec<&str> = cat
        .drivers
        .values()
        .filter(|d| d.has_model && offered(d, current))
        .filter_map(|d| d.origin.as_deref())
        .collect();
    sort_addons(&mut addons);
    for addon in addons {
        out.push(json!({ "heading": addon }));
        out.extend(
            sorted(
                cat.drivers
                    .values()
                    .filter(|d| d.has_model && offered(d, current) && d.origin.as_deref() == Some(addon))
                    .collect(),
            )
            .into_iter()
            .map(option),
        );
    }
    out.push(json!({ "heading": "Generic faces" }));
    out.extend(
        sorted(
            cat.drivers
                .values()
                .filter(|d| !d.has_model && offered(d, current))
                .collect(),
        )
        .into_iter()
        .map(option),
    );
    out
}

/// A driver's own details: names, code, number and nationality. They go with him
/// to every grid and seat.
/// The seasons a driver races in: each grid with his team and seat.
fn seasons_of<'a>(app: &App, cat: &'a Catalog, label: &str) -> Vec<(&'a Grid, &'a Team, usize)> {
    let mut out = Vec::new();
    for g in &cat.grids {
        for t in &g.teams {
            if let Some(i) = t
                .seats
                .iter()
                .position(|s| seat_view(cat, &app.edits, g, s).label == label)
            {
                out.push((g, t, i));
            }
        }
    }
    out
}

/// The looks preset a driver wears on a grid: his seat's, when the seat is his. A
/// driver put in another's seat wears what that seat has.
fn season_look<'a>(d: &'a Driver, seat: &crate::grid::grids::Seat) -> Option<&'a crate::grid::grids::Look> {
    if seat.driver != d.label {
        return None;
    }
    d.looks.iter().find(|l| l.label == seat.look)
}

/// A driver's own details, the same on every grid, then a tab per season for his
/// race number and helmet there, with the team he drives for.
fn driver_page(app: &App, l: &Loaded, label: &str, from: Option<&str>) -> Vec<Value> {
    let cat = &l.catalog;
    let mut blocks = Vec::new();
    if let Some(t) = from.and_then(|f| cat.team(f)) {
        blocks.push(json!({ "buttons": [ { "id": "back-team", "label": format!("← Back to {}", name_of(&app.edits, t, "full")), "style": "primary" } ] }));
    }
    blocks.push(json!({ "form": { "id": "driver-pick", "live": true, "fields": [
        { "select": { "id": "driver", "label": "Driver", "value": label, "search": true, "options": every_driver(cat, &app.edits, label) } } ] } }));
    let Some(d) = cat.drivers.get(label) else {
        blocks.push(text("Pick a driver to edit.", "muted"));
        return vec![json!({ "card": { "title": "Driver Profiles", "blocks": blocks } })];
    };
    let e = app.edits.drivers.get(label).cloned().unwrap_or_default();
    let mut nations: Vec<Value> = cat
        .nations
        .iter()
        .map(|n| json!({ "value": n.label, "label": n.name }))
        .collect();
    if !d.nation.is_empty() && !cat.nations.iter().any(|n| n.label == d.nation) {
        nations.insert(0, json!({ "value": d.nation, "label": d.nation }));
    }
    let code = e.code.clone().unwrap_or_else(|| d.code.clone());
    let nation = e.nation.clone().unwrap_or_else(|| d.nation.clone());
    let (first, last) = driver_names(&app.edits, d);
    let fields = vec![
        json!({ "text": { "id": "first", "label": "First name", "value": first, "placeholder": d.first, "max_len": 32 } }),
        json!({ "text": { "id": "last", "label": "Last name", "value": last, "placeholder": d.last, "max_len": 32 } }),
        json!({ "text": { "id": "code", "label": "Initials", "value": code, "placeholder": d.code, "max_len": 3 } }),
        json!({ "select": { "id": "nation", "label": "Nationality", "value": nation, "search": true, "options": nations } }),
    ];
    if let Some(p) = portrait(app, l, d) {
        blocks.push(json!({ "image": { "path": p, "width": 96, "height": 96, "fit": "contain" } }));
    }
    blocks.push(json!({ "label": "EVERY GRID" }));
    blocks.push(
        json!({ "form": { "id": format!("driver:{label}"), "live": true, "columns": 2, "fields": fields } }),
    );
    let parts = [
        ("first", "first name", e.first.is_some()),
        ("last", "last name", e.last.is_some()),
        ("code", "initials", e.code.is_some()),
        ("nation", "nationality", e.nation.is_some()),
    ];
    let mut restores: Vec<Value> = parts
        .iter()
        .filter(|(_, _, on)| *on)
        .map(|(part, name, _)| {
            button(
                &format!("restore-driver:{label}:{part}"),
                &format!("Restore {name}"),
                "undo",
                "ghost",
            )
        })
        .collect();
    if !e.is_empty() {
        restores.push(confirm(
            button(
                &format!("restore-driver:{label}:all"),
                "Restore driver",
                "undo",
                "danger",
            ),
            "Restore this driver to default?",
            "Your edits to his names, initials, nationality and helmets go. Car numbers stay with the seats.",
        ));
    }
    if !restores.is_empty() {
        blocks.push(json!({ "buttons": restores }));
    }

    // A tab per season he races in.
    let seasons = seasons_of(app, cat, label);
    blocks.push(json!({ "label": "PER SEASON" }));
    if seasons.is_empty() {
        blocks.push(text("Not in a seat on the profile's grids.", "muted"));
    } else {
        let open = app
            .driver_season
            .as_deref()
            .filter(|g| seasons.iter().any(|(x, _, _)| x.label == *g))
            .unwrap_or(&seasons[0].0.label)
            .to_owned();
        let items: Vec<Value> = seasons
            .iter()
            .map(|(g, _, _)| json!({ "id": g.label, "label": g.title }))
            .collect();
        blocks.push(json!({ "tabs": { "id": "driver-seasons", "items": items, "active": open } }));
        let (g, t, i) = seasons
            .iter()
            .find(|(g, _, _)| g.label == open)
            .copied()
            .expect("open is one of them");
        let seat = &t.seats[i];
        let v = seat_view(cat, &app.edits, g, seat);
        blocks.push(text(
            &format!(
                "Drives for {} on {}, seat {}.",
                name_of(&app.edits, t, "full"),
                g.title,
                i + 1
            ),
            "neutral",
        ));
        let mut fields = vec![
            json!({ "number": { "id": "number", "label": "Car number", "value": v.number, "min": 0, "max": 99, "step": 1 } }),
        ];
        let look = season_look(d, seat);
        if let Some(look) = look {
            let helmet = e
                .helmets
                .get(&look.label)
                .or(e.helmet.as_ref())
                .cloned()
                .unwrap_or_else(|| look.helmet.clone());
            let mine = format!("_{label}");
            let option = |h: &crate::grid::grids::Helmet| json!({ "value": h.label, "label": h.name });
            let mut options = vec![json!({ "heading": "His helmets" })];
            options.extend(
                cat.helmets
                    .iter()
                    .filter(|h| h.label.ends_with(&mine))
                    .map(option),
            );
            options.push(json!({ "heading": "Every helmet" }));
            options.extend(
                cat.helmets
                    .iter()
                    .filter(|h| !h.label.ends_with(&mine))
                    .map(option),
            );
            fields.push(json!({ "select": { "id": "helmet", "label": "Helmet", "value": helmet, "search": true, "options": options } }));
        }
        blocks.push(json!({ "form": { "id": format!("driver-season:{label}:{}", g.label), "live": true, "columns": 2, "fields": fields } }));
        if look.is_none() {
            let why = if seat.driver != d.label {
                let owner = cat
                    .drivers
                    .get(&seat.driver)
                    .map_or_else(|| seat.driver.clone(), |x| full_name(&app.edits, x));
                format!(
                    "He's in {owner}'s seat here, so the game dresses him in that seat's kit; its helmet can't be set for him."
                )
            } else {
                "He has no kit of his own on this grid, so his helmet can't be changed here.".to_owned()
            };
            blocks.push(text(&why, "muted"));
        }
        let mut season_restores = Vec::new();
        if app.edits.seats.get(&seat.key).is_some_and(|x| x.number.is_some()) {
            season_restores.push(button(
                &format!("restore-season:{label}:{}:number", g.label),
                "Restore car number",
                "undo",
                "ghost",
            ));
        }
        if look.is_some_and(|lk| e.helmets.contains_key(&lk.label)) {
            season_restores.push(button(
                &format!("restore-season:{label}:{}:helmet", g.label),
                "Restore helmet",
                "undo",
                "ghost",
            ));
        }
        if !season_restores.is_empty() {
            blocks.push(json!({ "buttons": season_restores }));
        }
    }

    let mut pills = status_pills(!e.is_empty(), &[]);
    if let Some(a) = &d.origin {
        pills.push(json!({ "text": format!("From {a}"), "tone": "muted" }));
    }
    if !d.has_model {
        pills.push(json!({ "text": "Generic face", "tone": "muted" }));
    }
    let mut card = json!({ "card": { "title": full_name(&app.edits, d), "subtitle": format!("{label} · applies when you press Play"), "blocks": blocks } });
    pills_below(&mut card, pills);
    vec![card]
}

fn emblem_page(app: &App, l: &Loaded) -> Vec<Value> {
    let Some(game) = app.game_dir.as_deref() else {
        return vec![text("The launcher hasn't said where the game is yet.", "muted")];
    };
    let Some(current) = l
        .emblems
        .iter()
        .find(|e| e.ui_name == app.emblem)
        .or_else(|| {
            l.emblems.iter().find(|e| {
                app.edits
                    .emblems
                    .get(&e.ui_name)
                    .is_some_and(|x| x.picture.is_some())
            })
        })
        .or_else(|| l.emblems.iter().find(|e| e.ui_name == emblems::FIRST))
        .or_else(|| l.emblems.first())
    else {
        return vec![text("The game database has no emblems.", "muted")];
    };
    let ui = current.ui_name.as_str();
    let edit = app.edits.emblems.get(ui);
    let picture = edit
        .and_then(|e| e.picture.as_ref())
        .map(|f| emblems::dir(&app.data_dir).join(f))
        .filter(|p| p.is_file());

    let options: Vec<Value> = l
        .emblems
        .iter()
        .map(|e| {
            let yours = app.edits.emblems.get(&e.ui_name).is_some_and(|x| x.picture.is_some());
            let mut o = json!({ "value": e.ui_name, "label": if yours { format!("{} · your logo", e.name) } else { e.name.clone() } });
            if let Some(p) = emblems::preview(game, &app.cache_dir, &e.ui_name) {
                o["image"] = json!(path_str(&p));
            }
            o
        })
        .collect();
    let choose = if picture.is_some() {
        "Change the logo…"
    } else {
        "Choose a logo…"
    };
    let mut main = vec![
        json!({ "form": { "id": "emblem-pick", "live": true, "fields": [
            { "select": { "id": "emblem", "label": "Emblem it replaces", "value": ui, "search": true, "options": options,
                "help": "Every player has the first one from the start." } } ] } }),
        json!({ "buttons": [ { "id": format!("emblem:{ui}"), "label": choose, "icon": "upload", "style": "primary",
            "kind": { "pick": { "title": "Choose a logo: PNG, JPEG or SVG", "extensions": image::PICTURES, "folders": false, "multiple": false } } } ] }),
    ];
    if game.join(emblems::place(ui).0).is_file() {
        if let (Some(e), true) = (edit, picture.is_some()) {
            match emblems::placed_preview(e, &app.data_dir, &app.cache_dir, ui) {
                Some(p) => main.push(json!({ "image": { "path": path_str(&p), "width": 160, "height": 160, "fit": "contain", "transparent": true } })),
                None => main.push(text("The picture can't be read. Choose it again.", "warn")),
            }
            main.push(json!({ "form": { "id": format!("emblem-opts:{ui}"), "live": true, "fields": [
                { "slider": { "id": "size", "label": "Logo size", "value": e.size(), "min": 20, "max": 100, "step": 5, "unit": "%" } },
                { "toggle": { "id": "transparent", "label": "Transparent background", "value": !e.opaque,
                    "help": "Turn it off if the logo doesn't show in the game: the background is then black." } } ] } }));
            main.push(json!({ "buttons": [ confirm(
                button(&format!("restore-emblem:{ui}:picture"), "Use the game's emblem", "undo", "ghost"),
                "Remove your logo?",
                "The game's own emblem comes back when you next press Play.",
            ) ] }));
        } else {
            main.push(text("No logo yet. A logo on a transparent background works best; a dark logo on white or a white logo on dark works too.", "muted"));
        }
    } else {
        main.push(text("The game's emblem file wasn't found, so the logo can't be built. Is the game folder set in the launcher?", "warn"));
    }
    let mut blocks = vec![json!({ "card": {
        "title": "Your My Team logo",
        "subtitle": "Choose a picture and it becomes an emblem in the My Team badge editor, drawn as it is, colours and all. It's built every time you press Play.",
        "blocks": main } })];

    let named = edit.and_then(|e| e.name.clone());
    blocks.push(field_card(
        "Name",
        &format!("{} · Original: {}", current.loc_tag, current.name),
        status_pills(named.is_some(), &[]),
        named.is_some().then(|| button(&format!("restore-emblem:{ui}:name"), "Restore", "undo", "ghost")),
        vec![json!({ "form": { "id": format!("emblem-name:{ui}"), "live": true, "fields": [
            { "text": { "id": "value", "value": named.clone().unwrap_or_else(|| current.name.clone()), "placeholder": current.name, "max_len": 32 } } ] } })],
    ));
    if let Some(original) = emblems::preview(game, &app.cache_dir, ui) {
        blocks.push(json!({ "card": { "title": "The game's emblem", "subtitle": "What your logo takes the place of", "blocks": [
            { "image": { "path": path_str(&original), "width": 96, "height": 96, "fit": "contain", "transparent": true } } ] } }));
    }
    blocks
}

fn presets_page(app: &App, l: &Loaded, presets: &[(String, Preset)]) -> Vec<Value> {
    let cat = &l.catalog;
    let mut blocks = Vec::new();
    let current = cat.grid(&app.grid).map_or("this grid", |g| g.title.as_str());
    let card = |slug: &str, p: &Preset| {
        let mut buttons = Vec::new();
        let what = if p.scope == "team" {
            format!(
                "From {} · apply it from a team's page",
                p.teams.first().map_or("", |t| t.from.as_str())
            )
        } else {
            let from = cat.grid(&p.grid).map_or(p.grid.clone(), |g| g.title.clone());
            buttons.push(button(
                &format!("apply:{slug}"),
                &format!("Apply to {current}"),
                "arrow-right-left",
                "outline",
            ));
            format!("From {from} · {}", plural(p.teams.len(), "team", "teams"))
        };
        buttons.push(confirm(
            button(&format!("delete:{slug}"), "Delete", "trash", "ghost"),
            "Delete this preset?",
            "The preset goes; your current edits stay.",
        ));
        json!({ "card": { "title": p.name, "subtitle": what, "buttons": buttons } })
    };
    for (scope, heading, none) in [
        (
            "grid",
            "GRID PRESETS",
            "No grid presets yet. Save one from a grid's page.",
        ),
        (
            "team",
            "TEAM PRESETS",
            "No team presets yet. Save one from a team's page.",
        ),
    ] {
        blocks.push(json!({ "label": heading }));
        let cards: Vec<Value> = presets
            .iter()
            .filter(|(_, p)| p.scope == scope)
            .map(|(slug, p)| card(slug, p))
            .collect();
        if cards.is_empty() {
            blocks.push(text(none, "muted"));
        } else {
            blocks.push(json!({ "grid": { "columns": 2, "blocks": cards } }));
        }
    }
    blocks
}

pub fn view(app: &mut App, host: &mut Host, section: &str) -> Value {
    if app.vanilla {
        return json!({ "subtitle": "Vanilla profile",
            "body": { "empty": { "title": "The vanilla profile takes no addons",
                "text": "Switch to a modded profile to edit its grids." } } });
    }
    app.refresh(host);
    let mut notices = Vec::new();
    if !app.enabled {
        notices.push(
            json!({ "text": "The Grid Editor is off in this profile, so your edits won't reach the game.",
            "tone": "warn", "action": { "id": "enable", "label": "Turn on" } }),
        );
    }
    let Some(l) = app.loaded.as_ref() else {
        if let Some(e) = &app.error {
            return json!({ "notices": notices, "body": { "empty": { "title": "The grids can't be read",
                "text": e, "action": { "id": "retry", "label": "Try again" } } } });
        }
        return json!({ "subtitle": "Reading the game and your addons…", "tick": true, "notices": notices,
            "body": { "blocks": [ { "meters": [ { "label": "Reading the game database, text and logos", "value": 50, "text": "" } ] } ] } });
    };
    if matches!(app.page, Page::Grid | Page::Team(_)) && l.catalog.grid(&app.grid).is_none() {
        app.grid = l
            .catalog
            .grids
            .first()
            .map(|g| g.label.clone())
            .unwrap_or_default();
        app.page = Page::Grid;
    }
    let l = app.loaded.as_ref().expect("checked");
    let presets = edits::list_presets(&app.data_dir);
    let (_, stale) = app.edits.live(&l.catalog);
    if stale > 0 {
        notices.push(json!({ "text": format!("{} name teams or seats this profile doesn't have, such as an addon grid that's off. They're kept, and apply again when it's on.",
            plural(stale, "edit", "edits")), "tone": "muted" }));
    }
    if !l.world.notes.is_empty() {
        notices.push(json!({ "text": format!("Some addon data can't be read here: {}", l.world.notes[0]), "tone": "muted" }));
    }
    if section == MY_TEAM {
        let mut v = json!({
            "subtitle": if app.edits.emblems.is_empty() { "Your own emblem, applied when you press Play".to_owned() }
                else { format!("{} · applied when you press Play", plural(app.edits.emblems.len(), "emblem made your own", "emblems made your own")) },
            "notices": notices,
            "body": { "blocks": emblem_page(app, l) },
            "footer": "Simplified Grid Editor by Team Simplified."
        });
        if app.busy() {
            v["tick"] = json!(true);
        }
        return v;
    }
    if matches!(app.page, Page::Grid | Page::Team(_))
        && let Some(p) = &l.world.progressive
        && let Some(year) = world::progressive_year(&app.grid)
    {
        notices.push(json!({ "text": format!("{} plays its {year} season with these teams, so their names, logos and drivers edited here show there too: in a new career, and in a running one from the next season it starts. Its car numbers stay its own.", p.name),
            "tone": "muted" }));
    }
    let content = match &app.page {
        Page::Presets => {
            vec![json!({ "card": { "title": "Presets", "blocks": presets_page(app, l, &presets) } })]
        }
        Page::Emblem => vec![json!({ "grid": { "columns": 1, "blocks": emblem_page(app, l) } })],
        Page::Driver { label, from } => driver_page(app, l, label, from.as_deref()),
        Page::Team(label) => match l
            .catalog
            .grid(&app.grid)
            .and_then(|g| Some((g, g.teams.iter().find(|t| &t.label == label)?)))
        {
            Some((g, t)) => team_page(app, l, g, t),
            None => match l.catalog.grid(&app.grid) {
                Some(g) => grid_page(app, l, g, &presets),
                None => vec![text("No grid to show.", "muted")],
            },
        },
        Page::Grid => match l.catalog.grid(&app.grid) {
            Some(g) => grid_page(app, l, g, &presets),
            None => vec![text("No grid to show.", "muted")],
        },
    };
    let blocks =
        vec![json!({ "grid": { "columns": [1, 5], "blocks": [nav(app, &l.catalog), content[0].clone()] } })];
    let mut v = json!({
        "subtitle": "Simplified Grid Editor by Team Simplified. Reads the profile's other addons live; your edits load after them.",
        "notices": notices,
        "body": { "blocks": blocks }
    });
    if app.busy() {
        v["tick"] = json!(true);
        v["subtitle"] = json!(format!(
            "{} · refreshing",
            v["subtitle"].as_str().unwrap_or_default()
        ));
    }
    v
}

// ── Events ──────────────────────────────────────────────────────────────────────

fn team<'a>(cat: &'a Catalog, label: &str) -> Result<&'a Team, String> {
    cat.team(label)
        .ok_or_else(|| format!("{label} isn't on any grid of this profile"))
}

fn set_name(edits: &mut Edits, team: &Team, part: &str, value: &str) {
    let value = value.trim();
    let e = edits.team_mut(&team.label);
    if value.is_empty() || value == team.name(part) {
        e.names.remove(part);
    } else {
        e.names.insert(part.to_owned(), value.to_owned());
    }
}

fn set_colour(edits: &mut Edits, team: &Team, field: &str, value: &str) -> Result<(), String> {
    let v = value.trim();
    let e = edits.team_mut(&team.label);
    if v.is_empty() {
        e.colours.remove(field);
        return Ok(());
    }
    let hex = format!("#{}", v.trim_start_matches('#').to_uppercase());
    encode_colour(&hex).ok_or_else(|| format!("{v} isn't a colour: write it as #RRGGBB"))?;
    if team.colour(field) == Some(hex.as_str()) {
        e.colours.remove(field);
    } else {
        e.colours.insert(field.to_owned(), hex);
    }
    Ok(())
}

/// The number a seat has before any edit: the one on the seat, else its driver's own.
fn base_number(cat: &Catalog, seat: &crate::grid::grids::Seat) -> i64 {
    if (0..255).contains(&seat.car_number) {
        seat.car_number
    } else {
        cat.drivers.get(&seat.driver).map_or(0, |d| d.number)
    }
}

/// Puts a driver and a number in a seat, as edits over what the seat has.
fn put_seat(
    edits: &mut Edits,
    cat: &Catalog,
    seat: &crate::grid::grids::Seat,
    driver: &str,
    number: Option<i64>,
) {
    let base = base_number(cat, seat);
    let e = edits.seat_mut(&seat.key);
    e.driver = (driver != seat.driver).then(|| driver.to_owned());
    e.number = number.filter(|n| *n > 0 && *n != base);
}

/// Every seat's driver on a grid, by seat key.
fn drivers_now(edits: &Edits, grid: &Grid) -> Vec<(String, String)> {
    grid.teams
        .iter()
        .flat_map(|t| &t.seats)
        .map(|s| {
            (
                s.key.clone(),
                edits
                    .seats
                    .get(&s.key)
                    .and_then(|e| e.driver.clone())
                    .unwrap_or_else(|| s.driver.clone()),
            )
        })
        .collect()
}

/// The seats of a grid that the same driver is in, by driver.
fn doubles(edits: &Edits, grid: &Grid) -> Vec<(String, Vec<crate::grid::grids::Seat>)> {
    let mut by: Vec<(String, Vec<crate::grid::grids::Seat>)> = Vec::new();
    for s in grid.teams.iter().flat_map(|t| &t.seats) {
        let d = edits
            .seats
            .get(&s.key)
            .and_then(|e| e.driver.clone())
            .unwrap_or_else(|| s.driver.clone());
        match by.iter_mut().find(|(x, _)| *x == d) {
            Some((_, seats)) => seats.push(s.clone()),
            None => by.push((d, vec![s.clone()])),
        }
    }
    by.retain(|(d, seats)| !d.is_empty() && seats.len() > 1);
    by
}

/// A driver races in one seat of a grid. After a preset, a driver it put in a seat
/// leaves the seat he had elsewhere on the grid, which gets a driver the preset took
/// off the grid, as a swap does. `before` is every seat's driver before, `fresh` the
/// seats the preset filled, which keep theirs.
fn keep_unique(
    edits: &mut Edits,
    cat: &Catalog,
    grid: &Grid,
    before: &[(String, String)],
    fresh: &HashSet<String>,
) {
    let now = |edits: &Edits, s: &crate::grid::grids::Seat| {
        edits
            .seats
            .get(&s.key)
            .and_then(|e| e.driver.clone())
            .unwrap_or_else(|| s.driver.clone())
    };
    let mut on_grid: HashSet<String> = grid
        .teams
        .iter()
        .flat_map(|t| &t.seats)
        .map(|s| now(edits, s))
        .collect();
    let mut freed: std::collections::VecDeque<String> = before
        .iter()
        .map(|(_, d)| d.clone())
        .filter(|d| !d.is_empty() && !on_grid.contains(d))
        .collect();
    for (_, seats) in doubles(edits, grid) {
        let keep = seats.iter().position(|s| fresh.contains(&s.key)).unwrap_or(0);
        for (i, seat) in seats.iter().enumerate() {
            if i == keep {
                continue;
            }
            let instead = freed
                .pop_front()
                .or_else(|| (!on_grid.contains(&seat.driver)).then(|| seat.driver.clone()));
            let Some(d) = instead else { continue };
            // Back in his own seat he has its number again; elsewhere his own.
            let number = if d == seat.driver {
                None
            } else {
                cat.drivers.get(&d).map(|x| x.number)
            };
            put_seat(edits, cat, seat, &d, number);
            on_grid.insert(d);
        }
    }
}

fn set_seat(
    edits: &mut Edits,
    cat: &Catalog,
    grid: &Grid,
    team: &Team,
    i: usize,
    driver: Option<&str>,
    number: Option<i64>,
) {
    let Some(seat) = team.seats.get(i) else { return };
    if let Some(d) = driver.filter(|d| !d.is_empty()) {
        let current = |s: &crate::grid::grids::Seat, edits: &Edits| {
            edits
                .seats
                .get(&s.key)
                .and_then(|e| e.driver.clone())
                .unwrap_or_else(|| s.driver.clone())
        };
        let here = current(seat, edits);
        if here != d {
            let here_number = seat_number(cat, edits, seat, &here);
            // A driver already on the grid swaps seats with the one he replaces,
            // each with the number he races with; the champion's #1 goes with him
            // by itself.
            let other = grid
                .teams
                .iter()
                .flat_map(|t| &t.seats)
                .find(|s| s.key != seat.key && current(s, edits) == d)
                .cloned();
            let brings = match &other {
                Some(o) => Some(seat_number(cat, edits, o, d)),
                None => cat.drivers.get(d).map(|x| x.number),
            };
            put_seat(edits, cat, seat, d, brings);
            if let Some(o) = other {
                put_seat(edits, cat, &o, &here, Some(here_number));
            }
        }
    }
    if let Some(n) = number {
        let base = base_number(cat, seat);
        let e = edits.seat_mut(&seat.key);
        e.number = (n != base).then_some(n);
    }
}

/// The grid a team's form belongs to: the open one when it has the team.
fn grid_of<'a>(cat: &'a Catalog, open: &str, team: &str) -> Option<&'a Grid> {
    let has = |g: &&Grid| g.teams.iter().any(|t| t.label == team);
    cat.grid(open).filter(has).or_else(|| cat.grids.iter().find(has))
}

/// Reads a picture of any kind and size and keeps it as a PNG under `dir`, named
/// after what it came from.
fn keep_picture(path: &str, dir: &Path, prefix: &str) -> Result<String, String> {
    let picture = image::load_picture(Path::new(path), 1024)?;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let name = format!("{prefix}-{:016x}.png", fnv1a64(&bytes));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    image::save_png(&picture, &dir.join(&name))?;
    Ok(name)
}

fn import_logo(app: &mut App, team_label: &str, kind: &str, path: &str) -> Result<(), String> {
    let l = app.loaded.as_ref().ok_or("the grids aren't read yet")?;
    team(&l.catalog, team_label)?;
    let name = keep_picture(path, &logos_dir(&app.data_dir), team_label)?;
    let e = app.edits.team_mut(team_label);
    if kind == "all" {
        e.logo = None;
        for k in logos::all_kinds() {
            e.logos.insert(k.to_owned(), name.clone());
        }
    } else {
        e.logos.insert(kind.to_owned(), name);
    }
    Ok(())
}

fn import_emblem(app: &mut App, ui: &str, path: &str) -> Result<(), String> {
    let name = keep_picture(path, &emblems::dir(&app.data_dir), ui)?;
    app.edits.emblems.entry(ui.to_owned()).or_default().picture = Some(name);
    Ok(())
}

fn notice(t: impl Into<String>) -> Value {
    json!({ "notice": t.into() })
}

pub fn event(app: &mut App, host: &mut Host, event: &Value) -> Result<Value, String> {
    if let Some(t) = event.get("tab") {
        if t["id"].as_str() == Some("driver-seasons") {
            app.driver_season = t["tab"].as_str().map(str::to_owned);
        }
        return Ok(json!("reload"));
    }
    if let Some(c) = event.get("click") {
        let id = c["id"].as_str().unwrap_or_default();
        if id == "enable" {
            return Ok(json!({ "set_addon_enabled": true }));
        }
        if id == "retry" {
            app.error = None;
            return Ok(json!("reload"));
        }
        if let Some(to) = id.strip_prefix("nav:") {
            app.grid_form = None;
            app.team_form = None;
            app.seat_pick = None;
            app.boss_pick = None;
            app.page = match to {
                PRESETS_TAB => Page::Presets,
                EMBLEM_TAB => Page::Emblem,
                DRIVERS_TAB => Page::Driver {
                    label: app.edits.drivers.keys().next().cloned().unwrap_or_default(),
                    from: None,
                },
                _ => {
                    app.grid = to.to_owned();
                    Page::Grid
                }
            };
            return Ok(json!("reload"));
        }
        if let Some(label) = id.strip_prefix("edit-team:") {
            app.seat_pick = None;
            app.page = Page::Team(label.to_owned());
            return Ok(json!("reload"));
        }
        if id == "back" {
            app.page = Page::Grid;
            return Ok(json!("reload"));
        }
        if let Some(which) = id.strip_prefix("grid-form:") {
            app.grid_form = Some(which.to_owned());
            return Ok(json!("reload"));
        }
        if id == "grid-form-close" {
            app.grid_form = None;
            return Ok(json!("reload"));
        }
        if let Some((label, which)) = id.strip_prefix("team-form:").and_then(|r| r.split_once(':')) {
            app.team_form = Some((label.to_owned(), which.to_owned()));
            return Ok(json!("reload"));
        }
        if id == "team-form-close" {
            app.team_form = None;
            return Ok(json!("reload"));
        }
        if let Some((label, i)) = id.strip_prefix("replace-seat:").and_then(|r| r.split_once(':')) {
            app.seat_pick = i.parse().ok().map(|i| (label.to_owned(), i));
            return Ok(json!("reload"));
        }
        if id == "replace-seat-close" {
            app.seat_pick = None;
            return Ok(json!("reload"));
        }
        if let Some(label) = id.strip_prefix("replace-boss:") {
            app.boss_pick = Some(label.to_owned());
            return Ok(json!("reload"));
        }
        if id == "replace-boss-close" {
            app.boss_pick = None;
            return Ok(json!("reload"));
        }
        if let Some(label) = id.strip_prefix("edit-driver:") {
            let from = match &app.page {
                Page::Team(t) => Some(t.clone()),
                Page::Driver { from, .. } => from.clone(),
                _ => None,
            };
            app.page = Page::Driver {
                label: label.to_owned(),
                from,
            };
            return Ok(json!("reload"));
        }
        if id == "back-team" {
            if let Page::Driver { from: Some(t), .. } = &app.page {
                app.page = Page::Team(t.clone());
            }
            return Ok(json!("reload"));
        }
        if let Some(rest) = id.strip_prefix("restore-driver:") {
            let (label, part) = rest.split_once(':').ok_or("restore what?")?;
            if part == "all" {
                app.edits.drivers.remove(label);
            } else if let Some(e) = app.edits.drivers.get_mut(label) {
                match part {
                    "first" => e.first = None,
                    "last" => e.last = None,
                    "code" => e.code = None,
                    "number" => e.number = None,
                    "nation" => e.nation = None,
                    "helmet" => e.helmet = None,
                    _ => {}
                }
            }
            app.save()?;
            return Ok(json!("reload"));
        }
        app.wait(host);
        let l = app.loaded.as_ref().ok_or("the grids aren't read yet")?;
        let (verb, arg) = id.split_once(':').unwrap_or((id, ""));
        match verb {
            "restore-season" => {
                let mut it = arg.splitn(3, ':');
                let (Some(label), Some(grid), Some(part)) = (it.next(), it.next(), it.next()) else {
                    return Err("restore what?".into());
                };
                let cat = &l.catalog;
                let d = cat.drivers.get(label).ok_or("no such driver")?;
                let Some((_, t, i)) = seasons_of(app, cat, label)
                    .into_iter()
                    .find(|(g, _, _)| g.label == grid)
                else {
                    return Err("he isn't on that grid".into());
                };
                let seat = t.seats[i].clone();
                let look = season_look(d, &seat).map(|lk| lk.label.clone());
                match part {
                    "number" => {
                        if let Some(e) = app.edits.seats.get_mut(&seat.key) {
                            e.number = None;
                        }
                    }
                    "helmet" => {
                        if let (Some(e), Some(lk)) = (app.edits.drivers.get_mut(label), look) {
                            e.helmets.remove(&lk);
                        }
                    }
                    _ => {}
                }
                app.save()?;
                return Ok(json!("reload"));
            }
            "reset" => {
                let t = team(&l.catalog, arg)?.clone();
                let g = grid_of(&l.catalog, &app.grid, &t.label).cloned();
                let before = g.as_ref().map(|g| drivers_now(&app.edits, g)).unwrap_or_default();
                app.edits.reset_team(&t);
                // A driver who swapped into the team elsewhere goes back to his seat.
                if let Some(g) = &g {
                    let fresh: HashSet<String> = t.seats.iter().map(|s| s.key.clone()).collect();
                    keep_unique(&mut app.edits, &l.catalog, g, &before, &fresh);
                }
                app.save()?;
                return Ok(notice(format!("{} is back to the profile's", t.name("full"))));
            }
            "restore" => {
                let (label, part) = arg.split_once(':').ok_or("restore what?")?;
                let t = team(&l.catalog, label)?.clone();
                let g = grid_of(&l.catalog, &app.grid, &t.label).cloned();
                let before = g.as_ref().map(|g| drivers_now(&app.edits, g)).unwrap_or_default();
                restore(&mut app.edits, &t, part);
                if let (Some(g), Some(i)) = (
                    &g,
                    part.strip_prefix("seat")
                        .or(part.strip_prefix("driver"))
                        .and_then(|i| i.parse::<usize>().ok()),
                ) {
                    let fresh: HashSet<String> = t.seats.get(i).map(|s| s.key.clone()).into_iter().collect();
                    keep_unique(&mut app.edits, &l.catalog, g, &before, &fresh);
                }
                app.save()?;
                return Ok(json!("reload"));
            }
            "restore-emblem" => {
                let (ui, part) = arg.split_once(':').ok_or("restore what?")?;
                match (part, app.edits.emblems.get_mut(ui)) {
                    ("all", _) => {
                        app.edits.emblems.remove(ui);
                    }
                    ("name", Some(e)) => e.name = None,
                    ("picture", Some(e)) => {
                        e.picture = None;
                        e.size = None;
                        e.opaque = false;
                    }
                    _ => {}
                }
                app.save()?;
                return Ok(json!("reload"));
            }
            "restore-grid" => {
                let grid = l.catalog.grid(arg).ok_or("no such grid")?.clone();
                for t in &grid.teams {
                    app.edits.reset_team(t);
                }
                app.save()?;
                return Ok(notice(format!("{} is back to default", grid.title)));
            }
            "save-team" => {
                let t = team(&l.catalog, arg)?;
                let name = name_of(&app.edits, t, "full");
                let preset = Preset {
                    schema_version: 1,
                    name: name.clone(),
                    scope: "team".into(),
                    grid: app.grid.clone(),
                    teams: vec![edits::team_preset(&app.edits, t)],
                };
                edits::save_preset(&app.data_dir, &preset)?;
                return Ok(notice(format!("Saved {name} as a team preset")));
            }
            "apply" => {
                let preset = load_preset(app, arg)?;
                let grid = l.catalog.grid(&app.grid).ok_or("no grid is open")?.clone();
                let before = drivers_now(&app.edits, &grid);
                let n = edits::apply_grid(&mut app.edits, &preset, &grid);
                let fresh: HashSet<String> = grid
                    .teams
                    .iter()
                    .flat_map(|t| &t.seats)
                    .map(|s| s.key.clone())
                    .collect();
                keep_unique(&mut app.edits, &l.catalog, &grid, &before, &fresh);
                app.save()?;
                app.page = Page::Grid;
                return Ok(notice(format!(
                    "{} put on {} {}",
                    preset.name,
                    plural(n, "team", "teams"),
                    grid.title
                )));
            }
            "delete" => {
                std::fs::remove_file(edits::presets_dir(&app.data_dir).join(format!("{arg}.json")))
                    .map_err(|e| e.to_string())?;
                return Ok(json!("reload"));
            }
            _ => return Err(format!("unknown button {id}")),
        }
    }
    if let Some(p) = event.get("picked") {
        let id = p["id"].as_str().unwrap_or_default();
        let Some(path) = p["paths"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_str)
        else {
            return Ok(json!("none"));
        };
        if let Some(ui) = id.strip_prefix("emblem:") {
            app.wait(host);
            import_emblem(app, ui, path)?;
            app.save()?;
            return Ok(notice("My Team logo set; it goes in when you press Play"));
        }
        let rest = id
            .strip_prefix("logo:")
            .ok_or_else(|| format!("unknown picker {id}"))?;
        let (label, kind) = rest
            .split_once(':')
            .ok_or_else(|| format!("unknown picker {id}"))?;
        app.wait(host);
        import_logo(app, label, kind, path)?;
        app.save()?;
        let place = if kind == "all" {
            "Logo set for every place".to_owned()
        } else {
            format!("{} logo set", logos::kind_label(kind))
        };
        return Ok(notice(format!("{place}; it goes in when you press Play")));
    }
    if let Some(s) = event.get("submit") {
        return submit(app, host, s);
    }
    Ok(json!("none"))
}

fn restore(edits: &mut Edits, team: &Team, part: &str) {
    if let Some(i) = part.strip_prefix("driver").and_then(|i| i.parse::<usize>().ok()) {
        if let Some(e) = team.seats.get(i).and_then(|s| edits.seats.get_mut(&s.key)) {
            e.driver = None;
        }
    } else if let Some(i) = part.strip_prefix("number").and_then(|i| i.parse::<usize>().ok()) {
        if let Some(e) = team.seats.get(i).and_then(|s| edits.seats.get_mut(&s.key)) {
            e.number = None;
        }
    } else if let Some(i) = part.strip_prefix("seat").and_then(|i| i.parse::<usize>().ok()) {
        if let Some(seat) = team.seats.get(i) {
            edits.seats.remove(&seat.key);
        }
    } else if let Some(e) = edits.teams.get_mut(&team.label) {
        if let Some(kind) = part.strip_prefix("logo-") {
            // A picture for every place from before 0.3 is split up, so one place
            // can go back on its own.
            if let Some(all) = e.logo.take() {
                for k in logos::all_kinds() {
                    e.logos.entry(k.to_owned()).or_insert_with(|| all.clone());
                }
            }
            e.logos.remove(kind);
            e.sizes.remove(kind);
        } else if part == "boss" {
            e.boss = None;
        } else {
            e.names.remove(part);
            e.colours.remove(part);
        }
    }
}

fn load_preset(app: &App, slug: &str) -> Result<Preset, String> {
    let path = edits::presets_dir(&app.data_dir).join(format!("{slug}.json"));
    serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn submit(app: &mut App, host: &mut Host, s: &Value) -> Result<Value, String> {
    let form = s["form"].as_str().unwrap_or_default().to_owned();
    let values = &s["values"];
    let changed: Vec<String> = s["changed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c.as_str().map(str::to_owned))
        .collect();
    let str_of = |k: &str| {
        values[k]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| values[k].to_string())
    };
    app.wait(host);
    let l = app.loaded.as_ref().ok_or("the grids aren't read yet")?;
    let cat = &l.catalog;
    let (kind, arg) = form.split_once(':').unwrap_or((form.as_str(), ""));
    match kind {
        "teams" => {
            let label = s["row"].as_str().unwrap_or_default();
            let t = team(cat, label)?.clone();
            for c in &changed {
                match c.as_str() {
                    "team" => set_name(&mut app.edits, &t, "full", &str_of(c)),
                    "short" => set_name(&mut app.edits, &t, "short", &str_of(c)),
                    "super" => set_name(&mut app.edits, &t, "super_short", &str_of(c)),
                    "n1" | "n2" => {
                        let i = usize::from(c == "n2");
                        let n = values[c].as_i64().or_else(|| str_of(c).trim().parse().ok());
                        let g = grid_of(cat, arg, label).ok_or("no grid")?;
                        set_seat(&mut app.edits, cat, g, &t, i, None, n);
                    }
                    _ => {}
                }
            }
        }
        "names" => {
            let t = team(cat, arg)?.clone();
            for c in &changed {
                set_name(&mut app.edits, &t, c, &str_of(c));
            }
        }
        "seats" => {
            let label = arg.split(':').next().unwrap_or(arg);
            let t = team(cat, label)?.clone();
            let g = grid_of(cat, &app.grid, label).ok_or("no grid")?;
            for c in &changed {
                let Some((what, i)) = c.split_once(':') else {
                    continue;
                };
                let Ok(i) = i.parse::<usize>() else { continue };
                match what {
                    "driver" => {
                        set_seat(&mut app.edits, cat, g, &t, i, Some(&str_of(c)), None);
                        app.seat_pick = None;
                    }
                    "number" => {
                        let n = values[c]
                            .as_i64()
                            .or_else(|| values[c].as_str()?.trim().parse().ok());
                        set_seat(&mut app.edits, cat, g, &t, i, None, n);
                    }
                    _ => {}
                }
            }
        }
        "colours" => {
            let t = team(cat, arg)?.clone();
            match s["row"].as_str() {
                // The team page: one row, a column per colour.
                Some("now") => {
                    for c in &changed {
                        let hex = values[c].as_str().unwrap_or_default();
                        set_colour(&mut app.edits, &t, c, hex)?;
                    }
                }
                // A team row's panel: a row per colour.
                Some(field) if COLOUR_FIELDS.iter().any(|(f, _)| *f == field) => {
                    let hex = values["value"].as_str().unwrap_or_default();
                    set_colour(&mut app.edits, &t, field, hex)?;
                }
                _ => {}
            }
        }
        "seat-cells" => {
            let t = team(cat, arg)?.clone();
            let g = grid_of(cat, &app.grid, arg).ok_or("no grid")?;
            let i: usize = s["row"]
                .as_str()
                .and_then(|r| r.parse().ok())
                .ok_or("which seat?")?;
            let n = values["number"]
                .as_i64()
                .or_else(|| values["number"].as_str()?.trim().parse().ok());
            set_seat(&mut app.edits, cat, g, &t, i, None, n);
        }
        "boss" => {
            let t = team(cat, arg)?.clone();
            let pick = str_of("boss");
            let e = app.edits.teams.entry(t.label.clone()).or_default();
            e.boss = (!pick.is_empty() && pick != t.boss).then_some(pick);
            app.boss_pick = None;
        }
        "emblem-pick" => {
            app.emblem = str_of("emblem");
            return Ok(json!("reload"));
        }
        "emblem-name" => {
            let original = l
                .emblems
                .iter()
                .find(|e| e.ui_name == arg)
                .map(|e| e.name.clone())
                .unwrap_or_default();
            let value = str_of("value").trim().to_owned();
            let e = app.edits.emblems.entry(arg.to_owned()).or_default();
            e.name = (!value.is_empty() && value != original).then_some(value);
        }
        "emblem-opts" => {
            let e = app.edits.emblems.entry(arg.to_owned()).or_default();
            for c in &changed {
                match c.as_str() {
                    "size" => e.size = values["size"].as_f64().map(|v| (v.round() as u32).clamp(20, 100)),
                    "transparent" => e.opaque = !values["transparent"].as_bool().unwrap_or(true),
                    _ => {}
                }
            }
        }
        "logo-size" => {
            let (label, kind) = arg.split_once(':').ok_or("which logo?")?;
            team(cat, label)?;
            if let Some(v) = values["size"].as_f64() {
                let v = (v.round() as u32).clamp(20, 100);
                let e = app.edits.team_mut(label);
                if v == edits::FULL_SIZE {
                    e.sizes.remove(kind);
                } else {
                    e.sizes.insert(kind.to_owned(), v);
                }
            }
        }
        "driver-pick" => {
            let label = str_of("driver");
            let from = match &app.page {
                Page::Driver { from, .. } => from.clone(),
                _ => None,
            };
            app.page = Page::Driver { label, from };
            return Ok(json!("reload"));
        }
        "driver-season" => {
            let (label, grid) = arg.split_once(':').ok_or("which season?")?;
            let d = cat
                .drivers
                .get(label)
                .ok_or_else(|| format!("{label} isn't in the game with this profile's addons"))?
                .clone();
            let (g, t, i) = seasons_of(app, cat, label)
                .into_iter()
                .find(|(g, _, _)| g.label == grid)
                .map(|(g, t, i)| (g.clone(), t.clone(), i))
                .ok_or("he isn't on that grid")?;
            for c in &changed {
                match c.as_str() {
                    "number" => {
                        let n = values[c]
                            .as_i64()
                            .or_else(|| values[c].as_str()?.trim().parse().ok());
                        set_seat(&mut app.edits, cat, &g, &t, i, None, n);
                    }
                    "helmet" => {
                        let Some(look) = season_look(&d, &t.seats[i]) else {
                            continue;
                        };
                        let pick = str_of(c);
                        let e = app.edits.driver_mut(label);
                        // Back to what the kit has: no edit, unless an older edit for every
                        // season would put another helmet on it.
                        if pick == look.helmet && e.helmet.is_none() {
                            e.helmets.remove(&look.label);
                        } else if !pick.is_empty() {
                            e.helmets.insert(look.label.clone(), pick);
                        }
                    }
                    _ => {}
                }
            }
        }
        "driver" => {
            let d = cat
                .drivers
                .get(arg)
                .ok_or_else(|| format!("{arg} isn't in the game with this profile's addons"))?
                .clone();
            let e = app.edits.driver_mut(arg);
            let keep = |v: String, original: &str| {
                let v = v.trim().to_owned();
                (!v.is_empty() && v != original).then_some(v)
            };
            for c in &changed {
                match c.as_str() {
                    "first" => e.first = keep(str_of(c), &d.first),
                    "last" => e.last = keep(str_of(c), &d.last),
                    "code" => {
                        let code = str_of(c).trim().to_uppercase();
                        if !code.is_empty()
                            && (code.chars().count() != 3 || !code.chars().all(char::is_alphanumeric))
                        {
                            return Err(format!("{code} isn't a code: write three letters, such as NOR"));
                        }
                        e.code = keep(code, &d.code);
                    }
                    "number" => {
                        let n = values[c]
                            .as_f64()
                            .map(|n| n.round() as i64)
                            .or_else(|| str_of(c).trim().parse().ok());
                        e.number = n.map(|n| n.clamp(1, 99)).filter(|n| *n != d.number);
                    }
                    "nation" => e.nation = keep(str_of(c), &d.nation),
                    "helmet" => e.helmet = keep(str_of(c), d.helmet().unwrap_or_default()),
                    _ => {}
                }
            }
        }
        "apply-team" => {
            let t = team(cat, arg)?.clone();
            let slug = str_of("preset");
            if slug.is_empty() || slug == "null" {
                return Err("pick a team preset".into());
            }
            let preset = load_preset(app, &slug)?;
            let tp: TeamPreset = preset.teams.first().cloned().ok_or("the preset holds no team")?;
            let g = grid_of(cat, &app.grid, &t.label).ok_or("no grid")?.clone();
            let before = drivers_now(&app.edits, &g);
            edits::apply_team(&mut app.edits, &tp, &t);
            let fresh: HashSet<String> = t.seats.iter().map(|s| s.key.clone()).collect();
            keep_unique(&mut app.edits, cat, &g, &before, &fresh);
            app.save()?;
            app.team_form = None;
            return Ok(notice(format!("{} put on {}", preset.name, t.name("full"))));
        }
        "save-team" => {
            let t = team(cat, arg)?;
            let name = str_of("name").trim().to_owned();
            let name = if name.is_empty() {
                name_of(&app.edits, t, "full")
            } else {
                name
            };
            let preset = Preset {
                schema_version: 1,
                name: name.clone(),
                scope: "team".into(),
                grid: app.grid.clone(),
                teams: vec![edits::team_preset(&app.edits, t)],
            };
            edits::save_preset(&app.data_dir, &preset)?;
            app.team_form = None;
            return Ok(notice(format!("Saved {name} as a team preset")));
        }
        "export-team" => {
            let grid = app.grid.clone();
            let out = export(app, host, &grid, Some(arg), &str_of("name"))?;
            app.team_form = None;
            return Ok(out);
        }
        "save-grid" => {
            let label = if arg.is_empty() {
                str_of("grid")
            } else {
                arg.to_owned()
            };
            let grid = cat.grid(&label).ok_or("pick a grid")?;
            let name = str_of("name").trim().to_owned();
            let name = if name.is_empty() {
                format!("{} preset", grid.title)
            } else {
                name
            };
            let preset = Preset {
                schema_version: 1,
                name: name.clone(),
                scope: "grid".into(),
                grid: grid.label.clone(),
                teams: grid
                    .teams
                    .iter()
                    .map(|t| edits::team_preset(&app.edits, t))
                    .collect(),
            };
            edits::save_preset(&app.data_dir, &preset)?;
            app.grid_form = None;
            return Ok(notice(format!("Saved {name}")));
        }
        "export" => {
            let label = if arg.is_empty() {
                str_of("grid")
            } else {
                arg.to_owned()
            };
            let out = export(app, host, &label, None, &str_of("name"))?;
            app.grid_form = None;
            return Ok(out);
        }
        _ => return Err(format!("unknown form {form}")),
    }
    app.save()?;
    Ok(json!("reload"))
}

/// Writes a grid's edits, or only one team's of it, as an addon and installs it.
fn export(
    app: &mut App,
    host: &mut Host,
    grid_label: &str,
    team: Option<&str>,
    name: &str,
) -> Result<Value, String> {
    let l = app.loaded.as_ref().ok_or("the grids aren't read yet")?;
    let grid = l.catalog.grid(grid_label).ok_or("pick a grid")?;
    let fallback = match team.and_then(|t| grid.teams.iter().find(|x| x.label == t)) {
        Some(t) => name_of(&app.edits, t, "full"),
        None if team.is_some() => return Err("that team isn't on the open grid".into()),
        None => format!("{} grid", grid.title),
    };
    let name = if name.trim().is_empty() {
        fallback
    } else {
        name.trim().to_owned()
    };
    let id = format!("gridedit.{}", edits::slug(&name).replace('-', "_"));
    // Only this grid's edits go in, or the one team's.
    let mut only = Edits {
        schema_version: 1,
        ..Edits::default()
    };
    for t in grid.teams.iter().filter(|t| team.is_none_or(|x| t.label == x)) {
        if let Some(e) = app.edits.teams.get(&t.label) {
            only.teams.insert(t.label.clone(), e.clone());
        }
        for s in &t.seats {
            if let Some(e) = app.edits.seats.get(&s.key) {
                only.seats.insert(s.key.clone(), e.clone());
            }
            let driver = seat_view(&l.catalog, &app.edits, grid, s).label;
            if let Some(e) = app.edits.drivers.get(&driver) {
                only.drivers.insert(driver, e.clone());
            }
        }
    }
    let one = Catalog {
        grids: vec![grid.clone()],
        // An exported addon only names the game's own looks presets: an addon's could
        // be missing where it's turned on.
        drivers: l
            .catalog
            .drivers
            .iter()
            .map(|(k, d)| {
                let mut d = d.clone();
                d.looks.retain(|look| look.stock);
                (k.clone(), d)
            })
            .collect(),
        nations: Vec::new(),
        helmets: Vec::new(),
        principals: l.catalog.principals.clone(),
    };
    let out_dir = app.cache_dir.join("export").join(&id);
    let (dir, n) = ops::export(
        &out_dir,
        &id,
        &name,
        "Simplified Grid Editor",
        &one,
        &only,
        l.lib.as_ref(),
        &app.data_dir,
    )?;
    if !app.can("install_addon") {
        return Ok(json!({ "open": path_str(&dir) }));
    }
    host.call("install_addon", json!({ "path": path_str(&dir) }))?;
    let text = format!(
        "Exported {name} with {}; turn it on in a profile on the Addons page",
        plural(n, "change", "changes")
    );
    if app.can("move_addon") {
        Ok(json!({ "show_addon": { "id": id, "notice": text } }))
    } else {
        Ok(notice(text))
    }
}

// ── Build and summary ───────────────────────────────────────────────────────────

pub fn build(app: &mut App, host: &mut Host, params: &Value) -> Value {
    app.wait(host);
    let Some(l) = app.loaded.as_ref() else {
        return json!({ "ops": [], "warnings": [format!("Grid Editor: the grids can't be read, so no edits apply: {}",
            app.error.clone().unwrap_or_default())] });
    };
    let (live, _) = app.edits.live(&l.catalog);
    // The files go in the editor's cache folder, named from the launcher's cache root,
    // where the ops' sources are read from.
    let root = app.cache_root.clone();
    let base = match app.cache_dir.strip_prefix(&root) {
        Ok(dir) if !dir.as_os_str().is_empty() => format!("{}/", dir.to_string_lossy().replace('\\', "/")),
        _ => String::new(),
    };
    let _ = std::fs::remove_dir_all(root.join(format!("{base}build")));
    let mut out = ops::all(
        &l.catalog,
        &live,
        l.lib.as_ref(),
        &app.data_dir,
        &root,
        &format!("{base}build"),
        true,
        true,
    );
    ops::newer_heads(&l.catalog, &l.world, &mut out);
    if let Some(game) = app.game_dir.as_deref() {
        let (ops, warnings) = emblems::ops(
            &live,
            &l.emblems,
            game,
            &app.data_dir,
            &root,
            &format!("{base}build/emblems"),
            true,
        );
        for op in &ops {
            if let Some(k) = op["key"].as_str() {
                out.lang_keys.insert(k.to_owned());
            }
        }
        if !ops.is_empty() {
            out.lines.push(format!("{} My Team emblem change(s)", ops.len()));
        }
        out.ops.extend(ops);
        out.warnings.extend(warnings);
    }
    // Progressive Career's seats, where Simplified's DLL reads them in the game. Written
    // every time, so a seat put back on a grid leaves its season too.
    let seats = match &l.world.progressive {
        Some(p) => ops::progressive_seats(&l.catalog, &live, &l.world, p, &mut out),
        None => json!({ "seats": {} }),
    };
    let rel = format!("{base}build/progressive.json");
    let written = std::fs::create_dir_all(root.join(format!("{base}build")))
        .and_then(|_| std::fs::write(root.join(&rel), seats.to_string()));
    match written {
        Ok(()) => out.ops.push(
            json!({ "op": "file.put", "source": { "root": "cache", "path": rel },
            "target": format!("swofty/addons/{}/{}", app.id, world::PROGRESSIVE_SEATS) }),
        ),
        Err(e) => out
            .warnings
            .push(format!("Progressive Career's seats can't be written: {e}")),
    }
    app.note_applied(&out.lang_keys);
    ops::reply(out, &params["features"])
}

pub fn summary(app: &App) -> Value {
    let Some(l) = app.loaded.as_ref() else {
        return Value::Null;
    };
    let items: Vec<String> = l
        .catalog
        .grids
        .iter()
        .filter_map(|g| {
            let n = app.edits.count_in(g);
            (n > 0).then(|| format!("{} ({})", g.title, plural(n, "team", "teams")))
        })
        .collect();
    let mut items = items;
    if !app.edits.drivers.is_empty() {
        items.push(plural(app.edits.drivers.len(), "driver", "drivers"));
    }
    if !app.edits.emblems.is_empty() {
        items.push(plural(
            app.edits.emblems.len(),
            "My Team emblem",
            "My Team emblems",
        ));
    }
    if items.is_empty() {
        return Value::Null;
    }
    json!({ "label": "GRID EDITOR", "items": items })
}
