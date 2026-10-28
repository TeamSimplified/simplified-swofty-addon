// The emblems a player builds their My Team badge from. Each is a 512 by 512
// colour mask the game tints with the colours the player picks. A picture goes in
// place of the mask, and the emblem's colour_amount becomes 0 so the game draws it
// as it is and the badge editor offers no colours for it: the way the Team &
// Driver Editor addon does it.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use swofty_formats::erp::{ErpFile, Resource};
use swofty_formats::hash::fnv1a64;
use swofty_formats::texture::{self, Codec};

use crate::grid::edits::{Edits, EmblemEdit};
use crate::grid::image::{self, Image};
use crate::grid::logos;
use crate::grid::world::World;

const DIR: &str = "2025_asset_groups/ui_package/texture_arrays/customisation/cstm_badge_large";
const RES: &str = "eaid://ui_package/art/textures/customisation/cstm_badge_large";
const TABLE: &str = "customisation.custom_player_badges.emblems";

/// The emblem every player has from the start, the one a logo goes into first.
pub const FIRST: &str = "emblem_4";

#[derive(Debug, Clone)]
pub struct Emblem {
    pub label: String,
    pub ui_name: String,
    pub loc_tag: String,
    pub name: String,
}

/// The emblems in the live database, without the blank one.
pub fn list(world: &World) -> Vec<Emblem> {
    let db = &world.db;
    db.rows(TABLE, "BadgeEmblem")
        .iter()
        .filter_map(|r| {
            let ui_name = db.field(r, "ui_name");
            let label = db.field(r, "_metadata_");
            if ui_name.is_empty() || label == "blank" {
                return None;
            }
            let loc_tag = db.field(r, "loc_tag");
            Some(Emblem {
                name: world
                    .text(&loc_tag)
                    .unwrap_or_else(|| ui_name.trim_start_matches("emblem_").replace('_', " ")),
                label,
                ui_name,
                loc_tag,
            })
        })
        .collect()
}

/// The archive and resource of an emblem's picture.
pub fn place(ui_name: &str) -> (String, String) {
    (
        format!("{DIR}/emblem/{ui_name}.erp"),
        format!("{RES}/emblem/{ui_name}.tif.image"),
    )
}

fn read(game: &Path, ui_name: &str) -> Option<Resource> {
    let (erp, res) = place(ui_name);
    let file = ErpFile::open(&game.join(erp)).ok()?;
    file.load(file.erp.find(&res).ok()?).ok()
}

/// The emblem's picture size.
pub fn size(game: &Path, ui_name: &str) -> Option<(u32, u32)> {
    let h = texture::header(&read(game, ui_name)?).ok()?;
    Some((h.width, h.height))
}

/// A PNG of the game's own emblem, cached by name.
pub fn preview(game: &Path, cache: &Path, ui_name: &str) -> Option<PathBuf> {
    let path = cache.join("emblems").join(format!("{ui_name}.png"));
    if !path.is_file() {
        let img = texture::decode(&read(game, ui_name)?).ok()?;
        image::save_png(
            &Image {
                width: img.width,
                height: img.height,
                rgba: img.rgba,
            },
            &path,
        )
        .ok()?;
    }
    Some(path)
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("emblems")
}

/// The player's picture as the emblem will hold it: placed at its size on a
/// transparent or black square.
pub fn placed(edit: &EmblemEdit, data_dir: &Path, w: u32, h: u32) -> Result<Image, String> {
    let file = edit.picture.as_deref().ok_or("no picture")?;
    let picture = image::load_png(&dir(data_dir).join(file))?;
    Ok(image::place(
        &picture,
        w,
        h,
        edit.size(),
        edit.opaque.then_some([0, 0, 0]),
    ))
}

/// A PNG of the player's emblem as the game will show it, cached by its settings.
pub fn placed_preview(edit: &EmblemEdit, data_dir: &Path, cache: &Path, ui_name: &str) -> Option<PathBuf> {
    let file = edit.picture.as_deref()?;
    let path = cache.join("emblems").join(format!(
        "{ui_name}-{:016x}-{}-{}.png",
        fnv1a64(file),
        edit.size(),
        u8::from(edit.opaque)
    ));
    if !path.is_file() {
        image::save_png(&placed(edit, data_dir, 512, 512).ok()?, &path).ok()?;
    }
    Some(path)
}

/// The ops for the player's emblems: each picture encoded into its emblem, the
/// emblem's colours turned off, and new names. Sources are written under `root`.
pub fn ops(
    edits: &Edits,
    emblems: &[Emblem],
    game: &Path,
    data_dir: &Path,
    root: &Path,
    rel: &str,
    cached: bool,
) -> (Vec<Value>, Vec<String>) {
    let mut ops = Vec::new();
    let mut warnings = Vec::new();
    let source = |path: &str| {
        if cached {
            json!({ "root": "cache", "path": path })
        } else {
            json!(path)
        }
    };
    for (ui_name, edit) in &edits.emblems {
        let Some(emblem) = emblems.iter().find(|e| &e.ui_name == ui_name) else {
            continue;
        };
        if edit.picture.is_some() {
            let Some(res) = read(game, ui_name) else {
                warnings.push(format!("the game has no {ui_name} emblem"));
                continue;
            };
            let (Ok(h), Some(pixels)) = (texture::header(&res), res.fragments.get(1)) else {
                continue;
            };
            if h.codec() != Some(Codec::Bc7) {
                warnings.push(format!("{ui_name}: the emblem's format isn't BC7"));
                continue;
            }
            let data = placed(edit, data_dir, h.width, h.height)
                .and_then(|picture| logos::encode_slice(&picture, &h, pixels.size as usize));
            match data {
                Ok(data) => {
                    let (erp, resource) = place(ui_name);
                    let file = format!("{rel}/{ui_name}-{:016x}.bin", fnv1a64(&data));
                    if let Some(d) = root.join(&file).parent() {
                        std::fs::create_dir_all(d).ok();
                    }
                    if let Err(e) = std::fs::write(root.join(&file), &data) {
                        warnings.push(format!("{file}: {e}"));
                        continue;
                    }
                    ops.push(json!({ "op": "erp.replace-resource", "erp": erp, "resource": resource, "fragment": 1, "source": source(&file) }));
                    ops.push(json!({ "op": "db.set", "select": format!("{TABLE}.BadgeEmblem[_metadata_={}]", emblem.label),
                        "field": "colour_amount", "value": "0" }));
                }
                Err(e) => warnings.push(format!("{ui_name}: {e}")),
            }
        }
        if let Some(name) = edit.name.as_deref().map(str::trim).filter(|n| !n.is_empty())
            && !emblem.loc_tag.is_empty()
        {
            ops.push(json!({ "op": "lang.set", "language": "*", "key": emblem.loc_tag, "value": name }));
        }
    }
    (ops, warnings)
}
