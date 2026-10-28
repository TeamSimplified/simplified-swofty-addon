use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::UNIX_EPOCH;

use serde_json::{Value, json};

use crate::grid::edits::Edits;
use crate::grid::emblems::{self, Emblem};
use crate::grid::grids::{self, Catalog};
use crate::grid::host::Host;
use crate::grid::logos::{self, Library};
use crate::grid::world::{self, AddonRef, Inputs, World};

pub struct Loaded {
    pub world: World,
    pub catalog: Catalog,
    pub lib: Option<Library>,
    pub emblems: Vec<Emblem>,
    /// More Driver Models' driver portraits, by head name, when it's on.
    pub portraits: HashMap<String, PathBuf>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    /// The open grid's teams, a card each.
    Grid,
    /// One team of the open grid, by label.
    Team(String),
    /// One driver, by label, and the team the page was opened from.
    Driver {
        label: String,
        from: Option<String>,
    },
    Presets,
    /// The My Team Emblem.
    Emblem,
}

pub struct App {
    pub id: String,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_root: PathBuf,
    pub profile: String,
    pub vanilla: bool,
    pub enabled: bool,
    pub host_calls: Vec<String>,
    pub grid: String,
    pub page: Page,
    pub edits: Edits,
    pub loaded: Option<Loaded>,
    loading: Option<(String, Receiver<Result<Loaded, String>>)>,
    pub error: Option<String>,
    pub build_key: String,
    pub game_dir: Option<PathBuf>,
    /// The emblem open on the My Team Emblem tab, by ui_name.
    pub emblem: String,
    /// The name form open on the grid page: "save" or "export".
    pub grid_form: Option<String>,
    /// The form open in a team row's panel: the team's label and "save", "export" or
    /// "replace".
    pub team_form: Option<(String, String)>,
    /// The seat whose driver list is open on the team page: the team's label and the
    /// seat's place. The list is only built then, as it's long.
    pub seat_pick: Option<(String, usize)>,
    /// The team whose list of team principals is open.
    pub boss_pick: Option<String>,
    /// The season open on the driver page, by grid label.
    pub driver_season: Option<String>,
}

fn mtime(path: &Path) -> u64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

impl App {
    pub fn new(params: &Value) -> App {
        let path = |k: &str| PathBuf::from(params[k].as_str().unwrap_or_default());
        let cache_dir = path("cache_dir");
        // The launcher's data folder, else from cache_dir, which is <data>/cache/addons/<id>.
        let data_root = match params["data_root"].as_str() {
            Some(root) => PathBuf::from(root),
            None => cache_dir
                .ancestors()
                .nth(3)
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        };
        let data_dir = path("data_dir");
        App {
            id: params["addon"]
                .as_str()
                .unwrap_or("teamsimplified.grid-editor")
                .to_owned(),
            edits: Edits::load(&data_dir),
            data_dir,
            cache_dir,
            data_root,
            profile: params["profile"]["name"].as_str().unwrap_or("default").to_owned(),
            vanilla: params["profile"]["vanilla"].as_bool().unwrap_or(false),
            enabled: params["profile"]["enabled"].as_bool().unwrap_or(true),
            host_calls: params["host_calls"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c.as_str().map(str::to_owned))
                .collect(),
            grid: grids::BASE[0].0.to_owned(),
            page: Page::Grid,
            loaded: None,
            loading: None,
            error: None,
            build_key: String::new(),
            game_dir: None,
            emblem: String::new(),
            grid_form: None,
            team_form: None,
            seat_pick: None,
            boss_pick: None,
            driver_season: None,
        }
    }

    pub fn can(&self, call: &str) -> bool {
        self.host_calls.iter().any(|c| c == call)
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.edits.tidy();
        self.edits.save(&self.data_dir)
    }

    fn string(host: &mut Host, method: &str, params: Value) -> Option<String> {
        host.call(method, params).ok()?.as_str().map(str::to_owned)
    }

    /// The enabled addons in load order.
    fn addons(&self, host: &mut Host) -> Vec<AddonRef> {
        if self.can("addons")
            && let Ok(list) = host.call("addons", Value::Null)
        {
            let mut out: Vec<(usize, AddonRef)> = list
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .filter(|(_, a)| a["enabled"].as_bool() == Some(true))
                .map(|(i, a)| {
                    (
                        a["position"].as_u64().map_or(i, |p| p as usize),
                        AddonRef {
                            id: a["id"].as_str().unwrap_or_default().to_owned(),
                            name: a["name"].as_str().unwrap_or_default().to_owned(),
                            dir: PathBuf::from(a["dir"].as_str().unwrap_or_default()),
                        },
                    )
                })
                .collect();
            out.sort_by_key(|(p, _)| *p);
            return out.into_iter().map(|(_, a)| a).collect();
        }
        let profile: Value = std::fs::read(
            self.data_root
                .join("profiles")
                .join(format!("{}.json", self.profile)),
        )
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or(Value::Null);
        profile["addons"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| a["enabled"].as_bool().unwrap_or(true))
            .filter_map(|a| {
                let id = a["id"].as_str()?.to_owned();
                let dir = self.data_root.join("addons").join(&id);
                let name = std::fs::read(dir.join("swofty.json"))
                    .ok()
                    .and_then(|d| serde_json::from_slice::<Value>(&d).ok())
                    .and_then(|m| m["name"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| id.clone());
                Some(AddonRef { id, name, dir })
            })
            .collect()
    }

    fn fingerprint(&self, addons: &[AddonRef]) -> String {
        let mut parts = vec![self.profile.clone(), self.build_key.clone()];
        for a in addons.iter().filter(|a| a.id != self.id) {
            parts.push(format!(
                "{}:{}:{}",
                a.id,
                mtime(&a.dir.join("swofty.json")),
                mtime(&a.dir)
            ));
            // More Driver Models' switches and numbers change on its page, not in its folder.
            if a.id == world::MORE_DRIVER_MODELS {
                let data = self.data_root.join("addon-data").join(&a.id);
                for f in world::MORE_DRIVER_MODELS_FILES {
                    parts.push(mtime(&data.join(f)).to_string());
                }
            }
        }
        let built = world::latest_build(&self.data_root, &self.profile);
        parts.push(built.map_or(0, |b| mtime(&b.join("plan.json"))).to_string());
        parts.join("|")
    }

    fn own_lang(&self) -> HashSet<String> {
        std::fs::read(self.cache_dir.join("applied.json"))
            .ok()
            .and_then(|d| serde_json::from_slice::<Value>(&d).ok())
            .and_then(|v| serde_json::from_value(v["lang"].clone()).ok())
            .unwrap_or_default()
    }

    pub fn note_applied(&self, lang: &std::collections::BTreeSet<String>) {
        std::fs::create_dir_all(&self.cache_dir).ok();
        if let Ok(text) = serde_json::to_vec(&json!({ "lang": lang })) {
            std::fs::write(self.cache_dir.join("applied.json"), text).ok();
        }
    }

    /// Starts reading the world again when the profile, its addons or its last build
    /// changed since it was read.
    pub fn refresh(&mut self, host: &mut Host) {
        self.poll();
        if self.build_key.is_empty() {
            self.build_key = Self::string(host, "game_build_key", Value::Null).unwrap_or_default();
        }
        let addons = self.addons(host);
        let fp = self.fingerprint(&addons);
        let current = self.loaded.as_ref().is_some_and(|l| l.fingerprint == fp);
        let pending = self.loading.as_ref().is_some_and(|(f, _)| *f == fp);
        if current || pending || (self.error.is_some() && self.loaded.is_none() && self.loading.is_none()) {
            return;
        }
        let Some(game_dir) = Self::string(host, "game_dir", Value::Null).map(PathBuf::from) else {
            self.game_dir = None;
            self.error = Some("The launcher hasn't found F1 25. Locate the game on the Library page.".into());
            return;
        };
        let stock_db = match host.call("game_original", json!({ "rel": world::DB_REL })) {
            Ok(v) => match v.as_str() {
                Some(p) => PathBuf::from(p),
                None => {
                    self.error = Some("The launcher returned no game database.".into());
                    return;
                }
            },
            Err(e) => {
                self.error = Some(format!("The game database isn't available: {e}"));
                return;
            }
        };
        self.game_dir = Some(game_dir.clone());
        let stock_eng = Self::string(host, "game_original_lang", json!({ "code": "eng" })).map(PathBuf::from);
        let profile_heads = if self.can("profile_heads") {
            host.call("profile_heads", Value::Null)
                .ok()
                .and_then(|v| {
                    v["heads"].as_array().map(|a| {
                        a.iter()
                            .filter_map(|h| {
                                let label = h["label"].as_str()?.to_lowercase();
                                Some((label, h["addon_name"].as_str().map(str::to_owned)))
                            })
                            .collect()
                    })
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let inputs = Inputs {
            me: self.id.clone(),
            data_root: self.data_root.clone(),
            profile: self.profile.clone(),
            game_dir: Some(game_dir.clone()),
            stock_db,
            stock_eng,
            addons,
            profile_heads,
            own_lang: self.own_lang(),
        };
        eprintln!(
            "re-reading the grids: {}",
            if self.loaded.is_none() {
                "first read"
            } else {
                "the profile, its addons or its last build changed"
            }
        );
        let cache = self.cache_dir.clone();
        let key = self.build_key.clone();
        let (tx, rx) = channel();
        let fingerprint = fp.clone();
        std::thread::spawn(move || {
            tx.send(read_all(&inputs, &game_dir, &cache, &key, fingerprint))
                .ok();
        });
        self.error = None;
        self.loading = Some((fp, rx));
    }

    /// Takes a finished read; true when one came in.
    pub fn poll(&mut self) -> bool {
        let Some((_, rx)) = &self.loading else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(loaded)) => {
                self.loaded = Some(loaded);
                self.loading = None;
                true
            }
            Ok(Err(e)) => {
                self.error = Some(e);
                self.loading = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.error = Some("reading the grids stopped".into());
                self.loading = None;
                true
            }
        }
    }

    pub fn busy(&self) -> bool {
        self.loading.is_some()
    }

    /// Waits for a read in progress, for calls that need the world now.
    pub fn wait(&mut self, host: &mut Host) {
        self.refresh(host);
        if let Some((_, rx)) = self.loading.take() {
            match rx.recv() {
                Ok(Ok(l)) => self.loaded = Some(l),
                Ok(Err(e)) => self.error = Some(e),
                Err(_) => self.error = Some("reading the grids stopped".into()),
            }
        }
    }
}

/// The portraits More Driver Models ships, one PNG per head, when it's enabled.
fn portraits(addons: &[AddonRef]) -> HashMap<String, PathBuf> {
    let Some(mdm) = addons.iter().find(|a| {
        a.id == "mildtdesign.more-driver-models" || a.name.to_lowercase().contains("more driver models")
    }) else {
        return HashMap::new();
    };
    std::fs::read_dir(mdm.dir.join("portraits"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().to_lowercase(), p)))
        .collect()
}

fn read_all(
    inputs: &Inputs,
    game_dir: &Path,
    cache: &Path,
    key: &str,
    fingerprint: String,
) -> Result<Loaded, String> {
    let world = world::load(inputs)?;
    let catalog = grids::catalog(&world);
    let ui_paths: Vec<String> = catalog.grids.iter().map(|g| g.ui_path.clone()).collect();
    let arrays = logos::wanted(&ui_paths);
    let lib = logos::load(game_dir, world.built.as_deref(), cache, key, &arrays, &world.erp);
    Ok(Loaded {
        portraits: portraits(&inputs.addons),
        emblems: emblems::list(&world),
        world,
        catalog,
        lib: Some(lib),
        fingerprint,
    })
}
