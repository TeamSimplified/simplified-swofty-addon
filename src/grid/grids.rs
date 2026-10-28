// The grids as the live database has them: each entrant list's teams, their names,
// colours and seats, and the drivers a seat can take.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::grid::db::Db;
use crate::grid::world::World;

pub const LISTS: &str = "entrants.entrant_lists";
pub const POOL: &str = "entrants.entrant_lists.entrants";
pub const TEAMS: &str = "entrants.teams";
pub const DRIVERS: &str = "entrants.drivers";

/// The grids the base game ships, in tab order.
pub const BASE: [(&str, &str); 5] = [
    ("f1_2026", "F1 2026"),
    ("f1_2025", "F1 2025"),
    ("f2_2026", "F2 2026"),
    ("f2_2025", "F2 2025"),
    ("f2_2024", "F2 2024"),
];

pub const NAME_FIELDS: [(&str, &str); 3] = [
    ("display_name", "full"),
    ("display_name_short", "short"),
    ("display_name_super_short", "super_short"),
];

pub const COLOUR_FIELDS: [(&str, &str); 4] = [
    ("ui_team_colour", "Menu"),
    ("primary_colour", "Primary"),
    ("secondary_colour", "Secondary"),
    ("tertiary_colour", "Tertiary"),
];

#[derive(Debug, Clone)]
pub struct NameField {
    pub field: &'static str,
    pub part: &'static str,
    pub key: String,
    pub value: String,
    /// Another team, or another name of this team, reads the same key, so an edit
    /// needs a key of its own.
    pub shared: bool,
}

#[derive(Debug, Clone)]
pub struct Seat {
    /// `<list>/<entrant label>`, or `<list>/#<index>` when the label doesn't name one row.
    pub key: String,
    pub select: String,
    pub row: String,
    pub position: i64,
    pub reference: String,
    pub override_driver: String,
    pub driver: String,
    pub car_number: i64,
    /// The looks preset the seat dresses its driver in: its own, else the one of the
    /// driver entry it names.
    pub look: String,
}

#[derive(Debug, Clone)]
pub struct Team {
    pub label: String,
    pub row: String,
    pub names: Vec<NameField>,
    pub timing: String,
    pub colours: Vec<(&'static str, Option<String>)>,
    pub seats: Vec<Seat>,
    /// The team principal, a character's label, as the profile's addons leave it.
    pub boss: String,
    /// The list of an optional team the game adds to a grid on request, such as
    /// Konnersport on F1 2025.
    pub optional: Option<String>,
}

impl Team {
    pub fn name(&self, part: &str) -> &str {
        self.names
            .iter()
            .find(|n| n.part == part)
            .map_or("", |n| n.value.as_str())
    }

    pub fn colour(&self, field: &str) -> Option<&str> {
        self.colours
            .iter()
            .find(|(f, _)| *f == field)
            .and_then(|(_, v)| v.as_deref())
    }
}

#[derive(Debug, Clone)]
pub struct Grid {
    pub label: String,
    pub title: String,
    pub origin: Option<String>,
    pub ui_path: String,
    pub teams: Vec<Team>,
    /// Last season's champion, who races as #1: the top of the standings the grid
    /// names, when that driver always takes the 1.
    pub champion: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Driver {
    pub label: String,
    pub first: String,
    pub last: String,
    pub number: i64,
    pub head: String,
    pub has_model: bool,
    pub pool: Option<String>,
    /// Takes #1 as the champion: a chance_to_use_number_1 of 100.
    pub number_one: bool,
    /// The addon that adds the driver or his head model; none for the game's own.
    pub origin: Option<String>,
    /// The row, and the text keys his names read.
    pub row: String,
    pub first_key: String,
    pub last_key: String,
    pub full_key: String,
    pub standard_key: String,
    /// The three-letter code on the timing tower.
    pub code: String,
    pub nation: String,
    /// He has a row of season details, whose preferred number follows his own.
    pub seasonal: bool,
    /// The looks presets linked to him, which give a seat his helmet, suit and gloves.
    pub looks: Vec<Look>,
}

/// One of a driver's looks presets: its selector, label and the helmet it gives.
#[derive(Debug, Clone)]
pub struct Look {
    pub select: String,
    pub label: String,
    pub helmet: String,
    /// The game has it without addons, so an exported addon can name it.
    pub stock: bool,
}

/// A helmet design: its label and a name to show, such as "Lando Norris · McLaren 2025".
#[derive(Debug, Clone)]
pub struct Helmet {
    pub label: String,
    pub name: String,
}

/// A nationality a driver can take: its label and its name.
#[derive(Debug, Clone)]
pub struct Nation {
    pub label: String,
    pub name: String,
}

impl Driver {
    pub fn name(&self) -> String {
        format!("{} {}", self.first, self.last).trim().to_owned()
    }

    /// The helmet his newest plain preset gives him: the latest year, then the
    /// shortest label, so a team's regular kit wins over a one-race special.
    pub fn helmet(&self) -> Option<&str> {
        self.looks
            .iter()
            .filter(|l| !l.helmet.is_empty())
            .max_by_key(|l| (year_of(&l.label), std::cmp::Reverse(l.label.len())))
            .map(|l| l.helmet.as_str())
    }
}

/// The year a kit label starts with: 2025 for "2025-mclaren_lando_norris", 2016 for
/// Classic Seasons' "cs16-ferrari_charles_leclerc".
fn year_of(label: &str) -> u32 {
    let head = label.split('-').next().unwrap_or_default();
    let digits = head.trim_start_matches("cs");
    match digits.parse::<u32>() {
        Ok(y) if digits.len() == 2 => 2000 + y,
        Ok(y) => y,
        Err(_) => 0,
    }
}

pub struct Catalog {
    pub grids: Vec<Grid>,
    pub drivers: BTreeMap<String, Driver>,
    pub nations: Vec<Nation>,
    pub helmets: Vec<Helmet>,
    /// The characters who can lead a team, by name.
    pub principals: Vec<Principal>,
}

/// A character who can be a team principal.
#[derive(Debug, Clone)]
pub struct Principal {
    pub label: String,
    pub name: String,
    /// The language key of the character's name.
    pub key: String,
    /// The key reads another character's name, as the game gave a team's key its next
    /// principal's name. Picking him gives him a key of his own.
    pub renamed: bool,
}

impl Catalog {
    pub fn principal_name(&self, label: &str) -> String {
        self.principals
            .iter()
            .find(|p| p.label == label)
            .map_or_else(|| title_case(label), |p| p.name.clone())
    }

    pub fn grid(&self, label: &str) -> Option<&Grid> {
        self.grids.iter().find(|g| g.label == label)
    }

    pub fn team(&self, label: &str) -> Option<&Team> {
        self.grids
            .iter()
            .flat_map(|g| &g.teams)
            .find(|t| t.label == label)
    }

    pub fn seat(&self, key: &str) -> Option<&Seat> {
        self.grids
            .iter()
            .flat_map(|g| &g.teams)
            .flat_map(|t| &t.seats)
            .find(|s| s.key == key)
    }
}

/// `#RRGGBB` from the signed ARGB integer text the database holds.
pub fn decode_colour(text: &str) -> Option<String> {
    let v: i64 = text.trim().parse::<f64>().ok()? as i64;
    let u = v as i32 as u32;
    Some(format!(
        "#{:02X}{:02X}{:02X}",
        (u >> 16) & 0xff,
        (u >> 8) & 0xff,
        u & 0xff
    ))
}

/// The reverse, opaque.
pub fn encode_colour(hex: &str) -> Option<String> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = u32::from_str_radix(h, 16).ok()?;
    Some(((0xff00_0000 | rgb) as i32).to_string())
}

/// A full name as a first and a last name: the last word, or from a particle such as
/// "de" in Nyck de Vries.
fn split_name(name: &str) -> (String, String) {
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.len() < 2 {
        return (String::new(), name.to_owned());
    }
    let particle = words[1..]
        .iter()
        .position(|w| {
            ["de", "van", "von", "da", "di", "der", "le", "la", "dos", "du"]
                .contains(&w.to_lowercase().as_str())
        })
        .map(|i| i + 1);
    let at = particle.unwrap_or(words.len() - 1);
    (words[..at].join(" "), words[at..].join(" "))
}

pub fn is_generic_head(head: &str) -> bool {
    let h = head.to_lowercase();
    h.contains("_malerig")
        || ["male_ls", "female_ls"].iter().any(|p| {
            h.strip_prefix(p)
                .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
}

fn title_case(s: &str) -> String {
    s.split(['_', ' ', '-'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn drivers(world: &World) -> BTreeMap<String, Driver> {
    let db = &world.db;
    let mut pool: HashMap<String, String> = HashMap::new();
    for r in db.rows(POOL, "entrant") {
        let driver = db.field(&r, "driver");
        if !driver.is_empty() {
            pool.entry(driver).or_insert_with(|| db.field(&r, "_metadata_"));
        }
    }
    let seasonal: std::collections::HashSet<String> = db
        .rows("entrants.drivers.seasonal_infos", "seasonal_info")
        .iter()
        .filter(|r| db.int(r, "preferred_car_number").is_some_and(|n| n != 255))
        .map(|r| db.field(r, "_metadata_"))
        .collect();
    let mut out = BTreeMap::new();
    for r in db.rows(DRIVERS, "driver") {
        let label = db.field(&r, "_metadata_");
        if label.is_empty() || label == "default_driver" {
            continue;
        }
        let (first_key, last_key) = (db.field(&r, "first_name"), db.field(&r, "last_name"));
        let first = world.text(&first_key).unwrap_or_default();
        let last = world.text(&last_key).unwrap_or_default();
        let (first, last) = if first.is_empty() && last.is_empty() {
            (String::new(), title_case(&label))
        } else {
            (first, last)
        };
        let head = db.field(&r, "head").to_lowercase();
        // A game driver wearing a head an addon brings for someone else, as Zenith's
        // reserves wear Sean Gelael's from More Driver Models, borrows it: he isn't
        // that addon's driver and counts as a generic face. One wearing his own, or one
        // an addon's build renamed, is the addon's.
        let by_build = world.build_drivers.contains(&label);
        let borrowed = !by_build
            && world.stock_drivers.contains(&label)
            && world.head_origin.contains_key(&head)
            && head != label.to_lowercase();
        let has_model = !borrowed && world.heads.contains(&head) && !is_generic_head(&head);
        let mut origin = world
            .driver_origin
            .get(&label)
            .or_else(|| (!borrowed).then(|| world.head_origin.get(&head)).flatten())
            .cloned();
        let mut has_model = has_model;
        let (mut first, mut last) = (first, last);
        let mut number = db.int(&r, "car_number").unwrap_or(0);
        // More Driver Models' drivers as its page has them now: on or off, with the
        // name and number it gives them, before a build has.
        if let Some((roster, d)) = world
            .roster
            .as_ref()
            .and_then(|r| Some((r, r.drivers.get(&label)?)))
        {
            has_model = d.on;
            origin = d.on.then(|| roster.addon.clone());
            if first.is_empty() || last.is_empty() {
                (first, last) = split_name(&d.name);
            }
            if let Some(n) = d.number {
                number = n;
            }
        }
        out.insert(
            label.clone(),
            Driver {
                pool: pool.get(&label).cloned(),
                seasonal: seasonal.contains(&label),
                looks: Vec::new(),
                label,
                first,
                last,
                number,
                number_one: db
                    .field(&r, "chance_to_use_number_1")
                    .trim()
                    .parse::<f64>()
                    .is_ok_and(|c| c >= 100.0),
                head,
                has_model,
                origin,
                first_key,
                last_key,
                full_key: db.field(&r, "full_name"),
                standard_key: db.field(&r, "last_name_standard"),
                code: db.field(&r, "initials"),
                nation: db.field(&r, "nationality"),
                row: r.clone(),
            },
        );
    }
    out
}

fn seats_of(db: &Db, list_row: &str, list: &str, pool: &HashMap<String, String>) -> Vec<(String, Seat)> {
    let rows = db.rows(list_row, "entrant");
    let labels: Vec<String> = rows.iter().map(|r| db.field(r, "_metadata_")).collect();
    let mut out = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let team = db.field(r, "team");
        if team.is_empty() {
            continue;
        }
        let label = &labels[i];
        let unique = !label.is_empty() && labels.iter().filter(|l| *l == label).count() == 1;
        let base = format!("{LISTS}.entrant_list[_metadata_={list}]");
        let (key, select) = if unique {
            (
                format!("{list}/{label}"),
                format!("{base}.entrant[_metadata_={label}]"),
            )
        } else {
            (format!("{list}/#{i}"), format!("{base}.entrant_{i}"))
        };
        let reference = db.field(r, "entrant_reference");
        let override_driver = db.field(r, "driver");
        let driver = if !override_driver.is_empty() {
            override_driver.clone()
        } else {
            pool.get(&reference).cloned().unwrap_or_default()
        };
        let mut look = db.field(r, "driver_customisation");
        if look.is_empty() {
            let entry = if reference.is_empty() { &driver } else { &reference };
            if let Some(p) = db.find(POOL, "entrant", entry) {
                look = db.field(&p, "driver_customisation");
            }
        }
        out.push((
            team,
            Seat {
                key,
                select,
                row: r.clone(),
                position: db.int(r, "team_position").unwrap_or(0),
                reference,
                override_driver,
                driver,
                car_number: db.int(r, "car_number").unwrap_or(255),
                look,
            },
        ));
    }
    out
}

pub fn catalog(world: &World) -> Catalog {
    let db = &world.db;
    let mut pool_driver: HashMap<String, String> = HashMap::new();
    for r in db.rows(POOL, "entrant") {
        pool_driver.insert(db.field(&r, "_metadata_"), db.field(&r, "driver"));
    }
    let mut team_rows: HashMap<String, String> = HashMap::new();
    let mut key_uses: HashMap<String, usize> = HashMap::new();
    for r in db.rows(TEAMS, "team") {
        team_rows.insert(db.field(&r, "_metadata_"), r.clone());
        for (field, _) in NAME_FIELDS {
            let key = db.field(&r, field);
            if !key.is_empty() {
                *key_uses.entry(key).or_default() += 1;
            }
        }
    }
    let lists: HashMap<String, String> = db
        .rows(LISTS, "entrant_list")
        .into_iter()
        .map(|r| (db.field(&r, "_metadata_"), r))
        .collect();

    let mut order: Vec<(String, String, Option<String>)> = BASE
        .iter()
        .filter(|(l, _)| lists.contains_key(*l))
        .map(|(l, t)| (l.to_string(), t.to_string(), None))
        .collect();
    let mut added: Vec<(&String, &String)> = world.list_origin.iter().collect();
    added.sort();
    for (label, addon) in added {
        let Some(row) = lists.get(label) else { continue };
        let title = world
            .text(&db.field(row, "display_name"))
            .unwrap_or_else(|| title_case(label));
        order.push((label.clone(), title, Some(addon.clone())));
    }

    let mut grids = Vec::new();
    for (label, title, origin) in order {
        let row = &lists[&label];
        let mut seats = Vec::new();
        let refs = db.field(row, "entrant_list_references");
        for r in refs.split(',').map(str::trim).filter(|r| !r.is_empty()) {
            if let Some(rrow) = lists.get(r) {
                seats.extend(seats_of(db, rrow, r, &pool_driver));
            }
        }
        seats.extend(seats_of(db, row, &label, &pool_driver));
        let mut teams: Vec<Team> = Vec::new();
        // The optional teams of a grid live in lists of their own that take the
        // grid by reference, such as f1_2025_with_konnersport.
        let mut optional: Vec<&String> = lists
            .keys()
            .filter(|l| {
                l.strip_prefix(label.as_str())
                    .is_some_and(|rest| rest.starts_with("_with_"))
            })
            .collect();
        optional.sort();
        let extra: Vec<(String, Seat, Option<String>)> = optional
            .into_iter()
            .flat_map(|o| {
                seats_of(db, &lists[o], o, &pool_driver)
                    .into_iter()
                    .map(move |(t, s)| (t, s, Some(o.clone())))
            })
            .collect();
        let all = seats.into_iter().map(|(t, s)| (t, s, None)).chain(extra);
        for (team_label, seat, optional) in all {
            if let Some(t) = teams.iter_mut().find(|t| t.label == team_label) {
                t.seats.push(seat);
                continue;
            }
            let Some(trow) = team_rows.get(&team_label) else {
                continue;
            };
            let names = NAME_FIELDS
                .iter()
                .map(|(field, part)| {
                    let key = db.field(trow, field);
                    NameField {
                        field,
                        part,
                        value: world.text(&key).unwrap_or_default(),
                        shared: key_uses.get(&key).copied().unwrap_or(0) > 1,
                        key,
                    }
                })
                .collect();
            teams.push(Team {
                label: team_label.clone(),
                row: trow.clone(),
                names,
                timing: db.field(trow, "timing_tower_texture_path"),
                colours: COLOUR_FIELDS
                    .iter()
                    .map(|(f, _)| (*f, decode_colour(&db.field(trow, f))))
                    .collect(),
                seats: vec![seat],
                boss: db.field(trow, "team_boss"),
                optional,
            });
        }
        for t in &mut teams {
            t.seats.sort_by_key(|s| s.position);
        }
        if teams.is_empty() {
            continue;
        }
        let standings = db.field(row, "previous_season_driver_standings");
        let top = db
            .find("entrants.standings", "standings", &standings)
            .and_then(|st| {
                db.rows(&st, "driver_reference")
                    .first()
                    .map(|r| db.field(r, "driver"))
            });
        grids.push(Grid {
            champion: top,
            ui_path: db.field(row, "ui_path"),
            label,
            title,
            origin,
            teams,
        });
    }
    let drivers = drivers(world);
    for g in &mut grids {
        if g.champion
            .as_ref()
            .is_some_and(|c| !drivers.get(c).is_some_and(|d| d.number_one))
        {
            g.champion = None;
        }
    }
    let mut nations: Vec<Nation> = db
        .rows("entrants.nationality", "nationality")
        .iter()
        .filter_map(|r| {
            let label = db.field(r, "_metadata_");
            (!label.is_empty() && label != "default").then(|| Nation {
                name: world
                    .text(&db.field(r, "language_string"))
                    .unwrap_or_else(|| title_case(&label)),
                label,
            })
        })
        .collect();
    nations.sort_by_key(|n| n.name.to_lowercase());
    let mut drivers = drivers;
    for folder in db.rows("entrants.driver_customisations", "folder") {
        let folder_label = db.field(&folder, "_metadata_");
        for r in db.rows(&folder, "preloaded_driver_customisation_properties") {
            let label = db.field(&r, "_metadata_");
            let Some(d) = drivers.get_mut(&db.field(&r, "linked_driver_stat_id")) else {
                continue;
            };
            if label.is_empty() {
                continue;
            }
            d.looks.push(Look {
                select: format!(
                    "entrants.driver_customisations.folder[_metadata_={folder_label}].preloaded_driver_customisation_properties[_metadata_={label}]"
                ),
                helmet: db.field(&r, "helmet"),
                stock: world.stock_looks.contains(&label),
                label,
            });
        }
    }
    let helmets = helmets(db, &drivers);
    let principals = principals(world, &grids);
    Catalog {
        grids,
        drivers,
        nations,
        helmets,
        principals,
    }
}

/// The characters whose job is running a team, and anyone a team has as its
/// principal. A character whose name key a later principal took, such as McLaren's
/// key now reading Andrea Stella for Andreas Seidl too, is named from his label.
fn principals(world: &World, grids: &[Grid]) -> Vec<Principal> {
    let db = &world.db;
    let bosses: HashSet<&str> = grids
        .iter()
        .flat_map(|g| &g.teams)
        .map(|t| t.boss.as_str())
        .collect();
    let rows: Vec<(String, String, String)> = db
        .rows("entrants.characters", "character")
        .iter()
        .filter_map(|r| {
            let label = db.field(r, "_metadata_");
            let role = db.field(r, "job_role");
            if label.is_empty()
                || !(role.contains("boss") || role.contains("principal") || bosses.contains(label.as_str()))
            {
                return None;
            }
            Some((label, db.field(r, "full_name"), role))
        })
        .collect();
    let texts: Vec<Option<String>> = rows
        .iter()
        .map(|(_, key, _)| {
            world
                .text(key)
                .map(|n| n.trim().to_owned())
                .filter(|n| !n.is_empty())
        })
        .collect();
    let mut name_uses: HashMap<&str, usize> = HashMap::new();
    for t in texts.iter().flatten() {
        *name_uses.entry(t.as_str()).or_default() += 1;
    }
    // Letters only, so "andreas_seidl" and "Andreas Seidl" match.
    let bare = |s: &str| {
        s.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_lowercase()
    };
    let mut out: Vec<Principal> = rows
        .iter()
        .zip(&texts)
        .map(|((label, key, _), text)| {
            let renamed = text
                .as_deref()
                .is_some_and(|t| name_uses[t] > 1 && bare(t) != bare(label));
            let name = match text {
                Some(t) if !renamed => t.clone(),
                _ => title_case(label),
            };
            Principal {
                label: label.clone(),
                name,
                key: key.clone(),
                renamed,
            }
        })
        .collect();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for p in &out {
        *seen.entry(p.name.clone()).or_default() += 1;
    }
    for p in &mut out {
        if seen[&p.name] > 1 {
            p.name = format!("{} · {}", p.name, title_case(&p.label));
        }
    }
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// Every helmet design, named after the driver and the team kit it belongs to.
fn helmets(db: &crate::grid::db::Db, drivers: &BTreeMap<String, Driver>) -> Vec<Helmet> {
    let mut out: Vec<Helmet> = db
        .rows("customisation.custom_avatar.helmets", "Helmet")
        .iter()
        .filter_map(|r| {
            let label = db.field(r, "_metadata_");
            if label.is_empty() {
                return None;
            }
            let name = match label
                .split_once('-')
                .and_then(|(y, rest)| Some((y, rest.split_once('_')?)))
            {
                Some((_, (team, who))) => {
                    let who = drivers.get(who).map_or_else(|| title_case(who), Driver::name);
                    let year = year_of(&label);
                    let year = if year > 0 {
                        format!(" {year}")
                    } else {
                        String::new()
                    };
                    format!("{who} · {}{year}", title_case(team))
                }
                None => title_case(&label),
            };
            Some(Helmet { label, name })
        })
        .collect();
    out.sort_by_key(|h| h.name.to_lowercase());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_split_at_the_last_word_or_a_particle() {
        let split = |n: &str| split_name(n);
        assert_eq!(
            split("Christian Lundgaard"),
            ("Christian".into(), "Lundgaard".into())
        );
        assert_eq!(split("Nyck de Vries"), ("Nyck".into(), "de Vries".into()));
        assert_eq!(
            split("Juan Pablo Montoya"),
            ("Juan Pablo".into(), "Montoya".into())
        );
        assert_eq!(split("Prost"), (String::new(), "Prost".into()));
    }

    #[test]
    fn colours_go_both_ways() {
        assert_eq!(decode_colour("-1997016").as_deref(), Some("#E18728"));
        assert_eq!(encode_colour("#e18728").as_deref(), Some("-1997016"));
        assert_eq!(decode_colour("-16723266.0").as_deref(), Some("#00D2BE"));
        assert_eq!(encode_colour("12345"), None);
    }

    #[test]
    fn generic_faces_are_told_apart_from_scanned_heads() {
        assert!(is_generic_head("male_ls144"));
        assert!(is_generic_head("female_ls493_malerig"));
        assert!(!is_generic_head("max_verstappen"));
        assert!(!is_generic_head("male_lsx"));
    }
}
