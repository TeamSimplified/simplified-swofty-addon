// The Grid Editor page: the profile's F1 and F2 grids and the ones addons add, with
// team names, logos, colours, line-ups, principals and drivers edited over the
// profile's other addons, and the My Team emblem. It was its own addon,
// teamsimplified.grid-editor, and speaks the launcher's JSON: this module keeps its
// state between calls and turns the helper's typed calls into that JSON and back.
pub mod app;
pub mod bc7;
pub mod db;
pub mod edits;
pub mod emblems;
pub mod grids;
pub mod host;
pub mod image;
pub mod logos;
pub mod ops;
pub mod pages;
pub mod world;

use std::path::Path;
use std::sync::Mutex;

use serde_json::{Value, json};
use swofty_addon::asset::{AssetBatch, AssetOp, Op, Origin};
use swofty_addon::launcher::{BuildCx, Cx, Summary};
use swofty_addon::view::{Effect, Event, View};
use swofty_addon::{Error, Result};

use crate::ID;
use app::App;
use host::Host;

pub const SECTION: &str = "grids";
/// The Grid Editor's own folder in Simplified's data and cache folders.
const FOLDER: &str = "grid-editor";
/// The addon the Grid Editor was before, whose edits and presets move here.
const OLD_ID: &str = "teamsimplified.grid-editor";

#[derive(Default)]
pub struct Grid {
    app: Mutex<Option<App>>,
}

fn other(e: impl std::fmt::Display) -> Error {
    Error::Other(e.to_string())
}

/// The launcher's data folder: Simplified's cache folder is `<data>/cache/addons/<id>`.
fn data_root(cx: &Cx) -> std::path::PathBuf {
    cx.cache_dir
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dest)?;
        } else {
            std::fs::copy(e.path(), dest)?;
        }
    }
    Ok(())
}

/// Whether the profile still has the Grid Editor as its own addon on, which would
/// apply the same edits twice.
fn standalone_on(cx: &Cx) -> bool {
    cx.addons()
        .and_then(|s| s.list().ok())
        .is_some_and(|list| list.iter().any(|a| a.id == OLD_ID && a.enabled))
}

impl Grid {
    /// The editor for this profile, made on the first call and again when the profile
    /// changes. Edits and presets from the Grid Editor's own addon come along once.
    fn with<T>(&self, cx: &Cx, f: impl FnOnce(&mut App, &mut Host) -> T) -> T {
        let mut slot = self.app.lock().unwrap_or_else(|e| e.into_inner());
        let data_dir = cx.data_dir.join(FOLDER);
        let fresh = slot
            .as_ref()
            .is_none_or(|a| a.profile != cx.profile.name || a.data_dir != data_dir);
        if fresh {
            let root = data_root(cx);
            let old = root.join("addon-data").join(OLD_ID);
            if !data_dir.exists() && old.is_dir() {
                copy_dir(&old, &data_dir).ok();
            }
            *slot = Some(App::new(&json!({
                "addon": ID,
                "data_dir": data_dir,
                "cache_dir": cx.cache_dir.join(FOLDER),
                "data_root": root,
                "profile": { "name": cx.profile.name, "vanilla": cx.profile.vanilla, "enabled": cx.profile.enabled },
                "host_calls": Host::calls(cx),
            })));
        }
        let app = slot.as_mut().expect("made above");
        app.vanilla = cx.profile.vanilla;
        app.enabled = cx.profile.enabled;
        f(app, &mut Host::new(cx))
    }

    pub fn view(&self, cx: &Cx) -> Result<View> {
        let mut v = self.with(cx, |app, host| pages::view(app, host, SECTION));
        if standalone_on(cx)
            && let Some(notices) = v["notices"].as_array_mut()
        {
            notices.insert(0, json!({ "text": "The Simplified Grid Editor addon is on too and applies the same edits again. Turn it off or remove it on the Addons page: its edits and presets are here now.", "tone": "warn" }));
        }
        serde_json::from_value(v).map_err(other)
    }

    pub fn event(&self, cx: &Cx, event: Event) -> Result<Effect> {
        let event = serde_json::to_value(event).map_err(other)?;
        let effect = self
            .with(cx, |app, host| pages::event(app, host, &event))
            .map_err(Error::Other)?;
        serde_json::from_value(effect).map_err(other)
    }

    pub fn tick(&self, cx: &Cx) -> bool {
        self.with(cx, |app, _| app.poll())
    }

    /// The edits as ops after the rest of Simplified's, so they go over its mods too.
    pub fn build(&self, cx: &BuildCx, batch: &mut AssetBatch) -> Result<()> {
        if cx.cx.profile.vanilla {
            return Ok(());
        }
        if standalone_on(&cx.cx) {
            batch.warnings.push(
                "Grid Editor: the Simplified Grid Editor addon is on too, so its edits apply from there; turn it off to apply them from here"
                    .into(),
            );
            return Ok(());
        }
        let params = json!({ "features": cx.features });
        let built: Value = self.with(&cx.cx, |app, host| pages::build(app, host, &params));
        let origin = Origin {
            addon: ID.into(),
            item: Some(FOLDER.into()),
        };
        for op in built["ops"].as_array().into_iter().flatten() {
            let op: AssetOp = serde_json::from_value(op.clone()).map_err(other)?;
            batch.ops.push(Op {
                op,
                origin: origin.clone(),
            });
        }
        let strings = |k: &str| -> Vec<String> {
            built[k]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str().map(|s| format!("Grid Editor: {s}")))
                .collect()
        };
        batch.lines.extend(strings("lines"));
        batch.warnings.extend(strings("warnings"));
        if let Some(fields) = built["fields"].as_object() {
            for (k, v) in fields {
                if let Ok(names) = serde_json::from_value::<Vec<String>>(v.clone()) {
                    batch.fields.entry(k.clone()).or_default().extend(names);
                }
            }
        }
        Ok(())
    }

    pub fn summary(&self, cx: &Cx) -> Option<Summary> {
        let v = self.with(cx, |app, _| pages::summary(app));
        serde_json::from_value(v).ok()
    }
}
