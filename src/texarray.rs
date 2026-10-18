// The slice merge is ported from merge_resources.py of the Simplified Launcher,
// and the texture array layout from LogoBaker.cs and LogoSource.cs of the
// Simplified Team Editor.
// Based on Simplified Suite by Team Simplified - https://github.com/TeamSimplified/simplified-suite
// Copyright (C) 2026 Team Simplified. GPL-3.0; see THIRD_PARTY.md.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use swofty_addon::asset::{AssetOp, Root, Source};
use swofty_addon::launcher::GameFiles;
use swofty_addon::ops::ErpOp;
use swofty_formats::erp::{Erp, ErpFile, Resource};

use crate::{Result, err, io_err};

// Team logos sit in texture arrays that mods carry in the ui_texture shader
// archives. Each mod's arrays are compared with the game's own slice by slice, and
// the slices that differ are combined into one archive, later mods winning. It loads
// with the boot archive as an overlay of its own, apart from every mod's, because
// the build hook gives it the addon as its origin.
pub const UI_ERP: &str = crate::merge::BOOT_ERP;
pub const MERGED: &str = "ui_textures.erp";
const IMAGE: &str = ".texarray.image";
const STOCK: [&str; 8] = [
    "2025_asset_groups/ui_package/common_front_end.erp",
    "2025_asset_groups/ui_package/flow_frontend.erp",
    "2025_asset_groups/ui_package/flow_race.erp",
    "2025_asset_groups/ui_package/flow_race_intro.erp",
    "2025_asset_groups/ui_package/flow_boot.erp",
    "2025_asset_groups/ui_package/flow_career_hub.erp",
    "2025_asset_groups/ui_package/flow_post_race.erp",
    "2025_asset_groups/ui_package/flow_customisation.erp",
];

pub fn carrier(rel: &str) -> bool {
    crate::merge::carrier(rel)
        && rel
            .rsplit('/')
            .next()
            .is_some_and(|f| f.starts_with("ui_texture"))
}

pub fn merged_by_slice(uri: &str) -> bool {
    uri.ends_with(IMAGE) || uri.ends_with(".texarray")
}

pub struct Carried {
    pub item: String,
    pub erp: PathBuf,
    pub uris: Vec<String>,
}

#[derive(Default)]
pub struct Built {
    pub ops: Vec<AssetOp>,
    pub lines: Vec<String>,
    pub warnings: Vec<String>,
}

struct Layout {
    format: Vec<u8>,
    count: usize,
    slice: usize,
}

impl Layout {
    // Arrays built for an older game can hold fewer slices than the game's own.
    // The slices they do hold sit at the same places, so they merge as long as
    // every slice has the same format and size.
    fn fits(&self, stock: &Layout) -> bool {
        self.format == stock.format && self.slice == stock.slice
    }
}

// Fragment 0 of a surface holds its header: the image type at 8, the size at 12
// and 16, the mip count at 24 and the array size at 28. Fragment 1 holds every
// slice, one after another, all the same length.
fn layout(r: &Resource) -> Option<(Layout, Vec<u8>)> {
    let (f0, f1) = (r.fragments.first()?, r.fragments.get(1)?);
    let head = f0.data().ok()?;
    let count = u32::from_le_bytes(head.get(28..32)?.try_into().ok()?) as usize;
    let data = f1.data().ok()?;
    if count < 2 || data.is_empty() || data.len() % count != 0 {
        return None;
    }
    let l = Layout {
        format: head[8..28].to_vec(),
        count,
        slice: data.len() / count,
    };
    Some((l, data))
}

fn slices(data: &[u8], count: usize) -> Vec<&[u8]> {
    data.chunks(data.len() / count).collect()
}

fn short(uri: &str) -> &str {
    uri.rsplit('/').next().unwrap_or(uri)
}

struct Archives {
    open: HashMap<PathBuf, ErpFile>,
}

impl Archives {
    fn load(&mut self, path: &Path, uri: &str) -> Result<Option<Resource>> {
        if !self.open.contains_key(path) {
            let file = ErpFile::open(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
            self.open.insert(path.to_path_buf(), file);
        }
        let file = &self.open[path];
        let Ok(found) = file.erp.find(uri) else {
            return Ok(None);
        };
        let r = file
            .load(found)
            .map_err(|e| err(format!("{}: {e}", path.display())))?;
        Ok(Some(r))
    }

    fn stock(&mut self, game: &dyn GameFiles, uri: &str) -> Result<Option<Resource>> {
        for rel in STOCK {
            let Ok(path) = game.original(rel) else { continue };
            if let Some(r) = self.load(&path, uri)? {
                return Ok(Some(r));
            }
        }
        Ok(None)
    }
}

pub fn build(game: &dyn GameFiles, carried: &[Carried], out: &Path) -> Result<Built> {
    let mut archives = Archives { open: HashMap::new() };
    let mut order: Vec<&str> = Vec::new();
    let mut versions: HashMap<&str, Vec<(&str, Resource)>> = HashMap::new();
    let mut header = None;
    for c in carried {
        for uri in &c.uris {
            let Some(r) = archives.load(&c.erp, uri)? else {
                continue;
            };
            if header.is_none() {
                let erp = &archives.open[&c.erp].erp;
                header = Some((erp.header, erp.version));
            }
            let v = versions.entry(uri).or_default();
            if v.is_empty() {
                order.push(uri);
            }
            v.push((c.item.as_str(), r));
        }
    }
    let mut built = Built::default();
    let mut counts: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut resources = Vec::new();
    for uri in order {
        let v = &versions[uri];
        let stock = match uri.ends_with(IMAGE) {
            true => archives.stock(game, uri)?,
            false => None,
        };
        let Some((stock, (want, base))) = stock.and_then(|s| layout(&s).map(|l| (s, l))) else {
            let (_, last) = v.last().expect("one version at least");
            resources.push(last.clone());
            continue;
        };
        let stock_slices = slices(&base, want.count);
        let mut merged: Vec<Vec<u8>> = stock_slices.iter().map(|s| s.to_vec()).collect();
        let mut owner: Vec<Option<&str>> = vec![None; want.count];
        let mut whole: Option<(&str, &Resource)> = None;
        for (item, r) in v.iter().map(|(i, r)| (*i, r)) {
            match layout(r) {
                Some((l, data)) if l.fits(&want) => {
                    if let Some((by, _)) = whole {
                        built.warnings.push(format!(
                            "{item}: its slices of {} left out; {by} replaces the whole array",
                            short(uri)
                        ));
                        continue;
                    }
                    if l.count > want.count {
                        built.warnings.push(format!(
                            "{item}: {} has {} slices, the game's {}; the slices past {} are left out",
                            short(uri),
                            l.count,
                            want.count,
                            want.count
                        ));
                    }
                    let mut n = 0;
                    let theirs = slices(&data, l.count).into_iter().take(want.count);
                    for (i, s) in theirs.enumerate() {
                        if s == stock_slices[i] {
                            continue;
                        }
                        if let Some(prev) = owner[i].filter(|p| *p != item && merged[i] != s) {
                            built.warnings.push(format!(
                                "{item} overrides slice {i} of {} from {prev}",
                                short(uri)
                            ));
                        }
                        merged[i] = s.to_vec();
                        owner[i] = Some(item);
                        n += 1;
                    }
                    if n > 0 {
                        let c = counts.entry(item).or_default();
                        c.0 += n;
                        c.1 += 1;
                    }
                }
                _ => {
                    built.warnings.push(format!(
                        "{item}: {} differs in size or format from the game's; the whole array is used",
                        short(uri)
                    ));
                    whole = Some((item, r));
                }
            }
        }
        match whole {
            Some((_, r)) => resources.push(r.clone()),
            None if owner.iter().any(Option::is_some) => {
                let mut r = stock.clone();
                r.set_fragment(1, &merged.concat())
                    .map_err(|e| err(format!("{uri}: {e}")))?;
                resources.push(r);
            }
            None => {}
        }
    }
    for (item, (n, arrays)) in counts {
        built.lines.push(format!(
            "{item}: {n} logo slice(s) in {arrays} texture array(s) merged"
        ));
    }
    if resources.is_empty() {
        if out.is_file() {
            std::fs::remove_file(out).map_err(io_err(out))?;
        }
        return Ok(built);
    }
    let (header, version) = header.expect("set with the first resource");
    built.ops = resources
        .iter()
        .map(|r| {
            AssetOp::Erp(ErpOp::PutResource {
                erp: UI_ERP.into(),
                resource: r.name.clone(),
                source: Source {
                    root: Root::Cache,
                    path: MERGED.into(),
                },
                rename: None,
            })
        })
        .collect();
    let mut erp = Erp {
        header,
        version,
        resources,
    };
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    let tmp = out.with_extension("erp.tmp");
    std::fs::write(&tmp, erp.to_bytes()).map_err(io_err(&tmp))?;
    std::fs::rename(&tmp, out).map_err(io_err(out))?;
    Ok(built)
}

#[cfg(test)]
mod tests {
    use swofty_formats::erp::{Fragment, HEADER_LEN, MAGIC};

    use super::*;

    const LOGOS: &str =
        "eaid://ui_package/art/texture_arrays/overrides.uidb/overrides.team_logos.f1_2026.texarray";

    struct Game(PathBuf);

    impl GameFiles for Game {
        fn build_key(&self) -> String {
            "k".into()
        }

        fn exe_version(&self) -> Option<String> {
            None
        }

        fn is_stock(&self, rel: &str) -> bool {
            self.0.join(rel).is_file()
        }

        fn original(&self, rel: &str) -> Result<PathBuf> {
            let p = self.0.join(rel);
            match p.is_file() {
                true => Ok(p),
                false => Err(err(format!("{rel} is not a game file"))),
            }
        }

        fn original_lang(&self, code: &str) -> Result<PathBuf> {
            Err(err(code.into()))
        }
    }

    fn fragment(data: Vec<u8>) -> Fragment {
        Fragment {
            name: *b"temp",
            offset: 0,
            size: data.len() as u64,
            flags: 0x10,
            compression: 0,
            packed: data.len() as u64,
            raw: data,
        }
    }

    fn resource(name: &str, fragments: Vec<Vec<u8>>) -> Resource {
        let mut kind = [0u8; 16];
        kind[..13].copy_from_slice(b"GfxSurfaceRes");
        let mut r = Resource {
            name: name.into(),
            kind,
            unk: vec![0; 4],
            fragments: fragments.into_iter().map(fragment).collect(),
            hash: [0; 16],
        };
        r.hash = r.content_hash().unwrap();
        r
    }

    fn surface(slices: &[[u8; 4]]) -> Resource {
        surface_of(2, slices)
    }

    fn surface_of(width: u32, slices: &[[u8; 4]]) -> Resource {
        let mut head = vec![0u8; 38];
        head[12..16].copy_from_slice(&width.to_le_bytes());
        head[16..20].copy_from_slice(&2u32.to_le_bytes());
        head[28..32].copy_from_slice(&(slices.len() as u32).to_le_bytes());
        resource(&format!("{LOGOS}.image"), vec![head, slices.concat()])
    }

    fn write(path: &Path, resources: Vec<Resource>) {
        let mut header = [0u8; HEADER_LEN];
        header[..4].copy_from_slice(MAGIC);
        header[4..8].copy_from_slice(&3u32.to_le_bytes());
        let mut erp = Erp {
            header,
            version: 3,
            resources,
        };
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, erp.to_bytes()).unwrap();
    }

    fn slices_of(path: &Path) -> Vec<Vec<u8>> {
        let file = ErpFile::open(path).unwrap();
        let r = file
            .load(file.erp.find(&format!("{LOGOS}.image")).unwrap())
            .unwrap();
        r.fragments[1]
            .data()
            .unwrap()
            .chunks(4)
            .map(<[u8]>::to_vec)
            .collect()
    }

    fn setup(name: &str) -> (PathBuf, Game) {
        let root = std::env::temp_dir().join(format!("simplified-texarray-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let stock = [[0; 4], [1; 4], [2; 4], [3; 4]];
        write(
            &root.join("game/2025_asset_groups/ui_package/flow_frontend.erp"),
            vec![surface(&stock), resource(LOGOS, vec![b"srv".to_vec()])],
        );
        let game = Game(root.join("game"));
        (root, game)
    }

    fn carried(root: &Path, item: &str, slices: &[[u8; 4]]) -> Carried {
        carried_as(root, item, surface(slices))
    }

    fn carried_as(root: &Path, item: &str, image: Resource) -> Carried {
        let erp = root.join(format!("{item}/shader_package_2025/ui_texture.nefx2.sm51.erp"));
        write(
            &erp,
            vec![image, resource(LOGOS, vec![format!("srv {item}").into_bytes()])],
        );
        Carried {
            item: item.into(),
            erp,
            uris: vec![format!("{LOGOS}.image"), LOGOS.into()],
        }
    }

    #[test]
    fn an_array_without_slice_data_has_no_layout() {
        let mut head = vec![0u8; 38];
        head[28..32].copy_from_slice(&4u32.to_le_bytes());
        assert!(layout(&resource(LOGOS, vec![head, Vec::new()])).is_none());
    }

    #[test]
    fn changed_slices_from_every_mod_go_into_one_array() {
        let (root, game) = setup("merge");
        let mods = [
            carried(&root, "a", &[[0; 4], [9; 4], [2; 4], [3; 4]]),
            carried(&root, "b", &[[0; 4], [7; 4], [2; 4], [8; 4]]),
            carried(&root, "c", &[[0; 4], [1; 4], [2; 4], [3; 4]]),
        ];
        let out = root.join("cache").join(MERGED);
        let built = build(&game, &mods, &out).unwrap();
        assert_eq!(slices_of(&out), [vec![0; 4], vec![7; 4], vec![2; 4], vec![8; 4]]);
        assert_eq!(
            built.lines,
            [
                "a: 1 logo slice(s) in 1 texture array(s) merged",
                "b: 2 logo slice(s) in 1 texture array(s) merged",
            ]
        );
        assert_eq!(
            built.warnings,
            ["b overrides slice 1 of overrides.team_logos.f1_2026.texarray.image from a"]
        );
        let targets: Vec<(&str, &str)> = built
            .ops
            .iter()
            .map(|op| match op {
                AssetOp::Erp(ErpOp::PutResource { erp, source, .. }) => (erp.as_str(), source.path.as_str()),
                _ => panic!("only resource puts"),
            })
            .collect();
        assert_eq!(targets, [(UI_ERP, MERGED), (UI_ERP, MERGED)]);
        let file = ErpFile::open(&out).unwrap();
        let srv = file.load(file.erp.find(LOGOS).unwrap()).unwrap();
        assert_eq!(srv.fragments[0].data().unwrap(), b"srv c");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_array_from_an_older_game_merges_the_slices_it_has() {
        let (root, game) = setup("older");
        let mods = [
            carried(&root, "a", &[[0; 4], [1; 4], [2; 4], [9; 4]]),
            carried(&root, "b", &[[6; 4], [1; 4]]),
            carried(&root, "c", &[[0; 4], [1; 4], [2; 4], [3; 4], [7; 4]]),
        ];
        let out = root.join("cache").join(MERGED);
        let built = build(&game, &mods, &out).unwrap();
        assert_eq!(slices_of(&out), [vec![6; 4], vec![1; 4], vec![2; 4], vec![9; 4]]);
        assert_eq!(
            built.warnings,
            [
                "c: overrides.team_logos.f1_2026.texarray.image has 5 slices, the game's 4; the slices past 4 are left out"
            ]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_array_of_another_format_is_used_whole() {
        let (root, game) = setup("whole");
        let mods = [
            carried(&root, "a", &[[0; 4], [9; 4], [2; 4], [3; 4]]),
            carried_as(&root, "b", surface_of(4, &[[5; 4], [5; 4]])),
        ];
        let out = root.join("cache").join(MERGED);
        let built = build(&game, &mods, &out).unwrap();
        assert_eq!(slices_of(&out), [vec![5; 4], vec![5; 4]]);
        assert_eq!(
            built.warnings,
            [
                "b: overrides.team_logos.f1_2026.texarray.image differs in size or format from the game's; the whole array is used"
            ]
        );
        let none = build(&game, &[], &out).unwrap();
        assert!(none.ops.is_empty() && !out.exists());
        std::fs::remove_dir_all(&root).ok();
    }
}
