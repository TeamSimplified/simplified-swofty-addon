// Placeholder until the conversion lands from feat/mod-importer-convert.
use std::path::Path;

use swofty_addon::launcher::GameFiles;
use swofty_addon::manifest::{Category, Kind};

use crate::library::{Library, Package};
use crate::{Result, err};

pub struct Converted {
    pub id: String,
    pub name: String,
    pub category: Category,
    pub kinds: Vec<Kind>,
    pub report: Vec<String>,
}

pub fn convert(
    _: &Library,
    p: &Package,
    _: Option<&dyn GameFiles>,
    _: &Path,
    _: &mut dyn FnMut(f32, &str),
) -> Result<Converted> {
    Err(err(format!(
        "{}: converting mods into addons is not built yet",
        p.name
    )))
}
