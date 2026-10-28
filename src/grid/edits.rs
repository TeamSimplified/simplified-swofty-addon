// The player's edits, kept in the addon's data folder so every profile shares
// them, and the presets saved from them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::grid::grids::{Catalog, Grid, Team};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TeamEdit {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colours: BTreeMap<String, String>,
    /// A picture for every place, by file name under `logos/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    /// A picture for one place, over `logo`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub logos: BTreeMap<String, String>,
    /// How much of its slot a place's picture fills, in percent.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sizes: BTreeMap<String, u32>,
    /// The team principal, a character's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boss: Option<String>,
}

/// How much of its slot a picture fills until the player changes it.
pub const FULL_SIZE: u32 = 100;

impl TeamEdit {
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
            && self.colours.is_empty()
            && self.logo.is_none()
            && self.logos.is_empty()
            && self.boss.is_none()
    }

    pub fn picture(&self, kind: &str) -> Option<&str> {
        self.logos.get(kind).or(self.logo.as_ref()).map(String::as_str)
    }

    pub fn size(&self, kind: &str) -> u32 {
        self.sizes.get(kind).copied().unwrap_or(FULL_SIZE).clamp(20, 100)
    }
}

/// A driver's own details: names, three-letter code, number and nationality.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DriverEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nation: Option<String>,
    /// A helmet design, by label, for every looks preset of his (before 0.4.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helmet: Option<String>,
    /// A helmet design for one season, by the label of that season's looks preset.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub helmets: BTreeMap<String, String>,
}

impl DriverEdit {
    pub fn is_empty(&self) -> bool {
        self.first.is_none()
            && self.last.is_none()
            && self.code.is_none()
            && self.number.is_none()
            && self.nation.is_none()
            && self.helmets.is_empty()
            && self.helmet.is_none()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SeatEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<i64>,
}

impl SeatEdit {
    pub fn is_empty(&self) -> bool {
        self.driver.is_none() && self.number.is_none()
    }
}

/// A My Team emblem the player made their own: a picture in place of the emblem's
/// colour mask, by file name under `emblems/`, how much of the square it fills,
/// whether its background is black rather than transparent, and a new name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EmblemEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub opaque: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Pictures per style layer, from 0.1.5 to 0.2.1; read to carry them over.
    #[serde(default, skip_serializing)]
    pub styles: BTreeMap<String, String>,
}

/// How much of the square a My Team logo fills until the player changes it.
pub const EMBLEM_SIZE: u32 = 80;

impl EmblemEdit {
    pub fn is_empty(&self) -> bool {
        self.picture.is_none() && self.name.is_none()
    }

    pub fn size(&self) -> u32 {
        self.size.unwrap_or(EMBLEM_SIZE).clamp(20, 100)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Edits {
    pub schema_version: u32,
    #[serde(default)]
    pub teams: BTreeMap<String, TeamEdit>,
    #[serde(default)]
    pub seats: BTreeMap<String, SeatEdit>,
    /// My Team emblems, by the emblem's ui_name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub emblems: BTreeMap<String, EmblemEdit>,
    /// Drivers' own details, by driver label.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub drivers: BTreeMap<String, DriverEdit>,
}

pub fn logos_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logos")
}

impl Edits {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("edits.json")
    }

    pub fn load(data_dir: &Path) -> Edits {
        let mut edits: Edits = std::fs::read(Self::path(data_dir))
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or(Edits {
                schema_version: 1,
                ..Edits::default()
            });
        // An emblem picture once went into a style layer; it goes into the emblem now.
        for e in edits.emblems.values_mut() {
            if e.picture.is_none() {
                e.picture = std::mem::take(&mut e.styles).into_values().next();
            }
        }
        edits
    }

    pub fn save(&self, data_dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let path = Self::path(data_dir);
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn team_mut(&mut self, label: &str) -> &mut TeamEdit {
        self.teams.entry(label.to_owned()).or_default()
    }

    pub fn seat_mut(&mut self, key: &str) -> &mut SeatEdit {
        self.seats.entry(key.to_owned()).or_default()
    }

    pub fn driver_mut(&mut self, label: &str) -> &mut DriverEdit {
        self.drivers.entry(label.to_owned()).or_default()
    }

    pub fn tidy(&mut self) {
        self.teams.retain(|_, t| !t.is_empty());
        self.seats.retain(|_, s| !s.is_empty());
        self.emblems.retain(|_, e| !e.is_empty());
        self.drivers.retain(|_, d| !d.is_empty());
    }

    pub fn team_edited(&self, team: &Team) -> bool {
        self.teams.get(&team.label).is_some_and(|t| !t.is_empty())
            || team
                .seats
                .iter()
                .any(|s| self.seats.get(&s.key).is_some_and(|e| !e.is_empty()))
    }

    pub fn count_in(&self, grid: &Grid) -> usize {
        grid.teams.iter().filter(|t| self.team_edited(t)).count()
    }

    pub fn reset_team(&mut self, team: &Team) {
        self.teams.remove(&team.label);
        for s in &team.seats {
            self.seats.remove(&s.key);
        }
    }

    /// The edits that name a team or seat the profile has, and how many don't.
    pub fn live(&self, catalog: &Catalog) -> (Edits, usize) {
        let mut out = Edits {
            schema_version: 1,
            ..Edits::default()
        };
        out.emblems = self.emblems.clone();
        out.drivers = self.drivers.clone();
        let mut stale = 0;
        for (k, v) in &self.teams {
            match catalog.team(k) {
                Some(_) => {
                    out.teams.insert(k.clone(), v.clone());
                }
                None => stale += 1,
            }
        }
        for (k, v) in &self.seats {
            match catalog.seat(k) {
                Some(_) => {
                    out.seats.insert(k.clone(), v.clone());
                }
                None => stale += 1,
            }
        }
        (out, stale)
    }
}

/// A team's edits with its seats in order, to put on any team.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TeamPreset {
    pub team: TeamEdit,
    #[serde(default)]
    pub seats: Vec<SeatEdit>,
    /// What the preset was saved from, for its name in lists.
    #[serde(default)]
    pub from: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub schema_version: u32,
    pub name: String,
    /// `grid` or `team`.
    pub scope: String,
    #[serde(default)]
    pub grid: String,
    #[serde(default)]
    pub teams: Vec<TeamPreset>,
}

pub fn presets_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("presets")
}

pub fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        "preset".into()
    } else {
        s.chars().take(48).collect()
    }
}

pub fn team_preset(edits: &Edits, team: &Team) -> TeamPreset {
    TeamPreset {
        team: edits.teams.get(&team.label).cloned().unwrap_or_default(),
        seats: team
            .seats
            .iter()
            .map(|s| edits.seats.get(&s.key).cloned().unwrap_or_default())
            .collect(),
        from: team.label.clone(),
    }
}

/// Puts a team preset on a team, seat for seat.
pub fn apply_team(edits: &mut Edits, preset: &TeamPreset, team: &Team) {
    edits.reset_team(team);
    if !preset.team.is_empty() {
        edits.teams.insert(team.label.clone(), preset.team.clone());
    }
    for (seat, edit) in team.seats.iter().zip(&preset.seats) {
        if !edit.is_empty() {
            edits.seats.insert(seat.key.clone(), edit.clone());
        }
    }
}

/// Puts a grid preset on a grid: by team label where the grid has the team, and
/// team by team in order on another grid.
pub fn apply_grid(edits: &mut Edits, preset: &Preset, grid: &Grid) -> usize {
    let same = grid.label == preset.grid;
    let mut n = 0;
    for (i, tp) in preset.teams.iter().enumerate() {
        let team = if same {
            grid.teams.iter().find(|t| t.label == tp.from)
        } else {
            grid.teams.get(i)
        };
        if let Some(team) = team {
            apply_team(edits, tp, team);
            n += 1;
        }
    }
    n
}

pub fn list_presets(data_dir: &Path) -> Vec<(String, Preset)> {
    let mut out: Vec<(String, Preset)> = std::fs::read_dir(presets_dir(data_dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let slug = name.strip_suffix(".json")?.to_owned();
            let p: Preset = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
            Some((slug, p))
        })
        .collect();
    out.sort_by_key(|p| p.1.name.to_lowercase());
    out
}

pub fn save_preset(data_dir: &Path, preset: &Preset) -> Result<String, String> {
    let dir = presets_dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let base = slug(&preset.name);
    let mut name = base.clone();
    let mut n = 2;
    while dir.join(format!("{name}.json")).exists() {
        name = format!("{base}-{n}");
        n += 1;
    }
    let text = serde_json::to_vec_pretty(preset).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{name}.json")), text).map_err(|e| e.to_string())?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_safe_file_names() {
        assert_eq!(slug("My 2026 grid!"), "my-2026-grid");
        assert_eq!(slug("***"), "preset");
    }
}
