# Parity with the Simplified Launcher

How the Simplified Importer's conversion compares with how Team Simplified's Simplified Launcher installs the same mod, and what the parity harness found on real mods.

## Sources

- **v3**: Simplified Launcher v3.2.13, the release players use. Its Python modules were recovered from the PyInstaller build. Citations are `module:line` in the recovered source, `actions.*` under `app/actions/` and `lib.*` under `app/lib/`. Its `app/original/` holds the clean database, the eleven language files, `2026_team_keys.lng`, `2026_team_names.json` and seven `ui_texarrays_*.erp` archives. The clean database and language files are byte-identical to the installed game's (build 25251167) and to the launcher's baseline.
- **v4**: the Simplified Launcher's C# source in the Simplified Suite, `Simplified Launcher/src/SimplifiedLauncher/`. Citations without a folder are under `Mods/`.
- **Ours**: this repository (`src/`) and the launcher ops that apply a converted addon (`packages/addon-api`, `packages/core` and `docs/reference/ops/` in SwoftyLauncher).

**identical** gives the same game state for the mod on its own; **different** says how; **missing** has no counterpart in ours. **fixed** marks rows this audit changed, with the commit subject.

## Summary

| | Rows |
|---|---|
| Identical before the audit | 19 |
| Different or missing, now fixed | 13 |
| Different by design | 10 |
| Not applicable to a converted addon | 5 |

## Matrix

### Import and staging

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 1 | Archive formats | v3: zip and `.serps` with `zipfile`, 7z with 7-Zip, rar with UnRAR then 7-Zip (`actions.prepare_mods:631-774`). v4: anything SharpCompress reads (`ModImport.cs:301-338`) | zip, 7z and rar in-process, 7-Zip or UnRAR as fallback (`package.rs:135-150`, `252-334`) | identical for zip, 7z, rar; v4's tar and gzip need 7-Zip |
| 2 | Unsafe entries | Absolute paths, `..` and `:` refused (`ModImport.cs:391-403`) | Same rule (`package.rs:381-393`) | identical |
| 3 | `mod.serps` | `mod_name` or `name`, `author`, `version`, `game_version` (`ModManifest.cs:24-82`, v3 `lib.api_mods`) | Same fields plus `studio_version` (`serps.rs`) | identical |
| 4 | Wrapper folder | Stepped into only when it holds `mod.serps` (`ModImport.cs:418-426`) | Same (`package.rs:395-405`) | identical |
| 5 | Game folder | v3: shallowest folder whose name starts with `f1 25`, else the parent of `shader_package_2025`, `audio` or `2025_asset_groups` (`actions.prepare_mods:232-265`). v4: a folder named exactly `F1 25`, else one holding game folders (`ModStaging.cs:74-107`) | Shallowest folder starting with `F1 25` that holds game folders, else one holding game folders, three levels down (`package.rs:482-497`) | identical |
| 6 | Loose files beside game folders at the mod's top | v3 copies them all, `mod.serps` and read-mes included. v4 gathers only the game folders (`ModStaging.cs:96-106`) | Every file was a game file | **fixed** (v4's rule, plus top-level files the game ships): `fix: leave out read-mes beside loose game folders and ea installer files` (`library.rs:358-380`) |
| 7 | Parts | v3: the parent of an `F1 25` folder or of `shader_package_2025`, `audio`, `2025_asset_groups` (`actions.list_mods:95-171`). v4 has none | The folder above any game folder or `F1 25` folder (`package.rs:419-463`) | identical. **fixed** a part folder named like the game, such as `F1 25 Engine Sounds for 2026 Cars/F1 25/audio`, which was taken for the game folder and failed the import: `fix: find a mod's parts when one is named like the game` (`package.rs` `parts`) |
| 8 | ` -c` and ` -x` | ` -c` names a category, ` -x` makes parts in one category exclusive (`actions.list_mods:195-236`) | Both left out of the part's name (`package.rs:465-480`); parts that write mostly the same things are variants with one on (`importer.rs:704`) | different: exclusivity comes from overlap, not ` -x` |
| 9 | Load order | v3: name order of the mods folder (`actions.list_mods:309-370`). v4: tick order | The profile's addon order; migration keeps each profile's mod order | identical in effect |
| 10 | Blocked files | `f1_25.exe` and its spellings, the anticheat launcher and `preloader_l.dll`, names starting `eaanticheat`, `eajavelin`, `preloader_`, except `eaanticheat.splash.png` (`actions.prepare_mods:30-52`). v4 blocks nothing | `.exe`, `.dll`, `.asi` and names starting `eaanticheat` were skipped | **fixed** (`eajavelin`, `preloader_` added): same commit as row 6 (`package.rs:32-38`). Other `.dll` and `.asi` files stay skipped by design (row 46) |
| 11 | Re-import | Replaces the library copy (`ModImport.cs:146-181`) | Same (`library.rs:336-353`) | identical |
| 12 | A folder with no game files | v4 imports it and installs nothing (`ModStaging.cs:88`) | Refused with a reason | different by design |

### Database

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 13 | Where the file sits | v3 routes `gamedatabases.2025.bin` from any folder (`actions.prepare_mods:321-401`, step 7). v4: any file with that name (`Merged.cs:32-38`) | Only `game_package/gamedatabases/` | **fixed**: `fix: merge a mod's database and language files wherever it keeps them` (`package.rs:58-60`, `79-81`) |
| 14 | One mod | v3 copies the mod's database whole (`actions.prepare_mods:590-597`). v4 applies its records and removals over the clean one (`DatabaseLayers.cs:28-74`) | Per record against the game's database (`diff.rs:156-186`), `put-record` with the mod's raw value (`merge.rs:84-100`) | identical |
| 15 | Removed records | Gone with the whole file (v3), removed (v4 `DatabaseLayers.cs:54-60`) | Removed only when `mod.serps` named the installed exe's version | **fixed**: `fix: apply a mod's removals as the simplified launcher does` (`diff.rs:179-185`); the report warns when `mod.serps` names another version |
| 16 | Two mods change one record | v3: neither applies, the clean value stays (`actions.team_palette:420-484`). v4: the later wins | The later addon wins | identical to v4; different from v3 by design |
| 17 | Another game's database | v4 fails (`Merged.cs:52-56`); v3 does not check | Fails the import (`library.rs:395-398`) | identical to v4 |

### Text

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 18 | Which files | v3: every `.lng` in the mod (`actions.prepare_mods:321-401`, step 2). v4: `.lng` files named like a game language, in any folder (`Merged.cs:32-38`, `ModStack.cs:241-246`) | Only `localisation/<dir>/language_*.lng` and `localisatioa/<dir>/language_*.lng` | **fixed** (`language_*.lng` in any folder): same commit as row 13 (`package.rs:61-63`). Other `.lng` names stay files, as in v4 |
| 19 | Unit | Entries changed or added against the clean file of the same name; a missing entry is not a removal (v3 `actions.merge_lng:196-364`, v4 `Merged.cs:154-160`) | Same (`diff.rs:256-281`) | identical |
| 20 | Languages a mod lacks | v3: every language gets the union of the mod's edits from all its files, the last file winning a key. v4: its own file, else its English, else its first by name (`Merged.cs:135-145`) | As v4 (`diff.rs:282-306`) | identical to v4; different from v3 by design (v3 gives every language the same text) |
| 21 | Untranslated filler | Applied like any change | A changed translation equal to the game's English text was left out | **fixed** (`lang.default`: applies unless another addon renames the key): `fix: apply untranslated text a mod fills in unless another addon renames it` (`diff.rs:326-337`) |
| 22 | Text a mod fills in for keys its database points at | Applied like any change | `lang.default` (`diff.rs:338-355`, `convert.rs:1025`) | identical for one mod; another addon's rename wins |
| 23 | Two mods change one key | v3: neither applies (`actions.merge_lng:196-364`). v4: the later wins | The later addon wins | identical to v4 |
| 24 | How the game reads it | Merged files go to `localisatioa/2025_release/` and only a patched exe reads them: ` tionf` → ` tioaf` (`lib.exe_patch:83-87`), applied in v3 only on Steam with the anticheat bypass and launch-after on (`lib.logic:257-273`, `actions.prepare_exe:32-100`); v4 holds the patch back (`ModStack.cs:150-153`) | `lang.set` and `lang.default` through the in-game loader (launcher `runtime.rs:240-250`, `394-403`) | different by design: ours needs no exe patch or anticheat bypass |
| 25 | `2026_team_keys.lng`, `2026_team_names.json` | Used only by the Team Editor path, not by mod installs (`actions.season_2026:245-296`, `lib.api_team_editor:558-580`) | none | not applicable |

### Archives

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 26 | A stock archive the mod ships | Replaced whole (v3 `actions.prepare_mods:321-401` step 6, v4 `ModStack.cs:205-234`) | Every changed or added resource went into an overlay on `vfx_package/common.erp`, which replaces the URI in every archive | **fixed**: `fix: overlay a mod's asset group archives on themselves, not on the boot archive` (`merge.rs:11-25`). Each archive under `2025_asset_groups/` now gets an overlay of its own; a layout change makes the launcher rewrite that archive, as v3 and v4 copy it |
| 27 | A resource two archives of one mod hold differently | Each archive keeps its own copy | One copy for both | **fixed**: `fix: keep a copy per archive of a resource a mod's archives hold differently`, then by row 26 |
| 28 | Resource change test | n/a (whole file) | Type, fragment layout and packed sizes, then digest or content hash (`diff.rs:188-212`) | identical |
| 29 | Resources the mod's archive lacks | A non-shader archive is replaced whole, so the resources it omits are gone; a shader archive is unioned with the game's, so the stock resources it omits are kept | A non-shader archive removes them (`diff.rs:232-241`); a shader archive keeps them | identical (shader archives remove nothing, row 30) |
| 30 | Shader archives (`shader_package_2025/*.erp`), including liveries Simplified Studio packs into them | The shipping launcher is PyInstaller Python; its shader merge is `app.lib.erp.merge_collected_shader_erps`, which **unions** the game's archive with the mod's (mod wins overlap, stock resources the mod omits are kept) and writes the result into the game folder. `stk.exe` is a separate .NET toolkit (the C# `ModStack.cs`) these mods do not use | Each resource the mod adds or changes goes into the mod's own overlay archive on `vfx_package/common.erp`, built at import and loaded with the game's archives; the game's shader archives are left untouched. The stock resources the mod omits stay | **different by choice**: Team Simplified keeps mods out of the game's shader archives. Swofty's `fix: keep shader-carried resources in their own shader archive` merged them into the stock archive instead, after the koenigsegg car mod delivered through the boot overlay hung the main menu at the My Team garage; that change is reverted |
| 31 | Simplified Studio's marker and shadow material in shader archives | Loaded with the shader archive | Left out | different by choice: loaded at boot, the shadow material would replace the game's vehicle shadow material |
| 47 | Generic and AMD shader archives (`<name>.sm51.erp`, `<name>.sm51_amd.erp`) | Both installed, each with the mod's own copy; the game loads the one for its GPU | A resource in both loads once, from the generic build; the report warns when the copies differ | different: AMD GPUs get the generic copy of a resource whose copies differ |
| 32 | Two mods ship one archive | v3: the first mod's archive with each later mod's resources over it, every resource the later mod carries winning, changed or not (`actions.prepare_mods:510-552`); shader archives likewise over the game's (`actions.prepare_mods:440-507`). v4: the later file wins whole | Each mod's changes over the game's, later addon wins per changed resource | different by design: a resource only the earlier mod changes keeps its change in ours, where the later mod's unchanged copy restores the game's in the Simplified Launcher. Two mods that change different cars in one archive both apply |
| 33 | Seven `ui_texarrays_*` kinds (car images, championship branding, drivers, engine branding, nationality flags, team branding, track images) | v3: the `.texarray.image` arrays in mods' `ui_texture.nefx2.sm51(_amd).erp`, the first mod's archive as the base, each slice compared with the `app/original/ui` arrays, later mod wins per slice, written to both shader archives (`actions.merge_resources:104-254`, `326-417`) | `erp.merge-slices` of each `.texarray.image` from `ui_texture*` shader archives into both `ui_texture.nefx2.sm51.erp` and `_amd.erp`; the launcher starts from the game's array in `ui_package` and writes the slices that differ over it (`convert.rs` `logos`, launcher `ops/erp.md#texture-arrays`); `.texarray` views whole into both | identical for arrays of the game's size. An array with fewer slices: v3 uses the mod's array as it is, ours merges its slices over the game's. Another format: both use it whole. Moved off the boot overlay with row 30 |
| 34 | Texture arrays elsewhere (`ui_package` archives) | Replaced with the archive | Per resource on that archive (row 26) | identical |
| 35 | `.texarray` views and other `ui_texture` resources | v3 keeps the first mod's whole archive | Views whole into both `ui_texture` archives; other resources in an overlay on their own `ui_texture` archive | identical |
| 36 | Archive the game does not ship | Copied | `file.put` (`package.rs:64-68`) | identical |
| 37 | SERPs overlays in `<launcher>/serps/` and `mod/F1 25` | v3 user folders merged into `abu_dhabi_roof` shader copies (`actions.prepare_serps:178-261`, `actions.prepare_mod`) | none | not applicable: not part of a mod |

### SERPs Base Files

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 38 | When they apply | Turned on whenever a selected mod has a shader archive with a resource named with `serps`, pinned at the top (`actions.archive_reader:365-470`, `webui/app.js` `autoEnableSerpsBasefiles`) | Converted like any mod, with no tie to the mods that need them | **fixed**: `feat: load simplified studio mods after the serps base files and turn them on` (`merge.rs:39-54`, `convert.rs:619-646`, `importer.rs:228-244`). Such a mod's `load_after` names the base files' addon, turning the mod on turns them on, and the report warns when the library lacks them |
| 39 | Their content | `vfx_package/common.erp`, both `teams/common.erp`, `EAAntiCheat.splash.png` | Converted per resource and file | identical (harness) |

### Files

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 40 | Other files | Copied over the game's, later mod wins | `file.put`; `.mipmaps` as `add-mipmaps` (`convert.rs:520-534`) | identical |
| 41 | `localisatioa/` files that are not language files | Copied to `localisatioa/` | Copied to `localisation/` (`package.rs:82-88`) | different: no game build reads either loose |
| 42 | Backup and restore | v3 backs up replaced files, the exe and anticheat files, removes `localisatioa/` and added files on restore (`actions.backup:25-152`, `actions.restore:214-403`); v4 likewise (`GameBackup.cs:84-239`) | The launcher's journal and `swofty restore` | identical in effect |
| 43 | Anticheat bypass | v3 copies `bypass.exe` into the game and runs it when language files need the patched exe (`actions.launch_pass:116-211`) | none | not applicable: text needs no patched exe (row 24) |

### Tracks

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 44 | Track packages (`manifest.json` with `track_code`) | v3 and v4 have no track install; the Simplified Track Installer builds track `.serps` mods | `track.add` with the whole package (`convert.rs:424-438`); Track Installer `.serps` convert like any mod | not applicable; the harness checks every package file rides in the addon |

### Programs and logs

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 45 | Converter log | none | One log per conversion in `logs/` of the addon's data folder, newest 20 kept (`log.rs`) | added: `feat: write a converter log for each conversion` |
| 46 | `.dll` and `.asi` plugins | Copied into the game (v3 blocks only named files) | Skipped and reported (`package.rs:32-38`) | different by design: an addon carries no programs |

## Harness

`examples/parity.rs` imports and converts each mod with this crate, applies the addon's database, text, archive and file ops with the launcher's crates (`swofty_addon::ops::apply_db_ops`, `swofty_core::erpbuild::ErpBuild`, the runtime's `lang.default` rule) to the game's own files, and compares the result with the Simplified Launcher's install of the same mod on a clean game:

- The database is the mod's records over the clean one, removals included, as `DatabaseLayers.Apply` and v3's whole copy give for one mod.
- Text is `Merged.Languages` over the eleven clean language files.
- Each archive the mod ships: for every resource URI in the mod's archive or the game's, the content the game reads after our ops equals what the Simplified Launcher installs. That is the mod's archive whole for an asset group archive, and for a shader archive the game's archive unioned with the mod's, the mod winning each URI and the stock resources it omits kept. Ours is read from where it ships: an asset group or shader archive's resources from the overlay on that archive, then the game's archive less removals; only other archives' resources from the boot overlay. Studio's marker is a no-op.
- Nothing a shader archive carries may reach the boot overlay: a shader-carried resource found there, or an added one missing from its own shader archive, is reported as different. This is the delivery that hung the main menu in game; the old harness treated the boot overlay as equivalent and called it identical. Run against the converter before the fix, the harness now reports koenigsegg and Merc26 different (192 of 194 and 898 of 907 resources).
- A resource the boot overlay replaces must not have copies in archives the mod does not ship, unless the mod ships the boot archive itself.
- Loose files compare by bytes and target. Track packages compare file by file.

Game files are read from the Steam folder and checked against Steam's depot manifests, or from the launcher's baseline objects where the launcher replaced them. Nothing is written to the game.

```sh
cargo run --release --example parity -- <mod>...
```

`PARITY_GAME`, `PARITY_DATA`, `PARITY_SCRATCH` override the game folder, Swofty's data folder and the scratch folder.

## Per-mod results

After the fixes, every F1 25 mod is identical. Run on 2026-10-22 against build 25251167, every F1 25 mod in the test set:

| Mod | Kind | Result |
|---|---|---|
| Aston Martin Honda 2027 Concept v2.0 | 2 variant parts (S1, S2), each a Studio-packed `object_auto_illuminate.nefx2.sm51.erp` and `_amd` (~114 MB each) carrying 798 livery textures and omitting the archive's base shader `object_auto_illuminate.nefx2` | identical, both parts and both GPU archives (799 resources each): the 798 textures in the overlay on each shader archive, the game's base shader kept; needs the SERPs Base Files |
| Merc26 | Mercedes 2026 livery Studio packed into `object_mask.nefx2.sm51.erp` and `_amd.erp` (731 KB and 767 KB in the game, 29 MB each in the mod) | identical: all 456 resources of each archive, the 450 the game lacks in the overlay on that archive; needs the SERPs Base Files |
| koenigsegg.zip | Studio car in `object_flag.nefx2.sm51.erp` and `_amd` (stock plus 96 added) | identical: 97 resources each, the 96 added in the overlay on each shader archive; needs the SERPs Base Files. The mod whose boot-overlay delivery hung the main menu in game |
| SERPs Base Files for F1 25 v4.1 | three `common.erp`, splash | identical. Before row 26: 6,761 copies in other archives were replaced |
| F1 2019 Cars for F1 25 | 22 asset group archives, 13 mip files | identical |
| F1 2021 Cars for F1 25 | 23 archives, 10 mip files | identical. Before row 26: 33,038 copies of its resources in archives it does not ship, such as story-mode cars, were replaced; before row 27: 2 resources of `common_fom.erp` |
| F1 25 with V10 Engines (V10.3) | Studio shader archives, 4 audio banks | identical; needs the SERPs Base Files |
| F1 25 Engine Sound Pack | 10 parts, each `<pack>/F1 25/audio/2025/*.bdl`, one named `F1 25 Engine Sounds for 2026 Cars` | identical, all 10 parts. Before `fix: find a mod's parts when one is named like the game`: the import failed, `holds no game files` |
| Pirelli Tyres Collection v1.0.6 | 12 parts, `teams/common.erp` and `tyrecompounds.erp` each | identical, every part |
| hockenheim.zip | track package, 1,189 files | identical package in `track.add` |
| sepang.zip | track package (`bahrain24`) | identical package in `track.add` |
| okayama-v0.34-swofty-ai.zip | track package | identical package in `track.add` |
| 1990s cars from 2019 and 2020.zip | 8 F1 2019 classic cars, `c_<team>_<yy>/wep/*.erp` and `.assetgroup` only; each assetgroup points at `2019_asset_groups/f1_2019_classic_vehicle_package/` | skipped: no F1 25 layout, game folder or `mod.serps`. The Simplified Launcher copies such files to the game folder's top, where nothing loads them |
| 05_Benetton B190.zip | loose DDS textures for another game | skipped: not an F1 25 mod |
| Williams_FW13B.zip | Assetto Corsa car skin | skipped: not an F1 25 mod |
| [IMG] F1 1990 v1.4.zip | Automobilista 2 livery pack | skipped: not an F1 25 mod |
| SCdesign_RSS1990_SAABERICSSONF1.7z | Assetto Corsa car skin | skipped: not an F1 25 mod |

Totals: 33 packages identical, 0 different, 5 archives skipped.

Before the fixes, V10, koenigsegg and Merc26 differed in the Studio shadow material (row 31), F1 2021 Cars and the SERPs Base Files in rows 26 and 27, and every Studio mod (Aston Martin, Merc26, koenigsegg, V10) in row 30, which the old harness missed. The Aston Martin 2027 livery drops its archive's base shader; the Simplified Launcher's Python shader merge keeps the game's, and an in-game boot test settled it: keeping the base shader boots to the frontend, dropping it crashes (ACCESS_VIOLATION in `.xtls`, loading `fom_ast`). Two in-game results back the shader rows: koenigsegg reaches the main menu only with its resources inside its own shader archive, and the Aston Martin livery boots only with the stock base shader kept. Whether each livery shows in game was not checked further. None of the real mods carry a database, language files or `ui_texture` arrays; those rows rest on the unit tests in `src/diff.rs`, `src/convert/tests.rs` and the launcher's gated logo merge test.

## Launcher ops still needed

Every row converts with ops the launcher has: `put-record`, `remove-record`, `lang.set`, `lang.default`, `erp.overlay` on any asset group archive (with the launcher's fallback to rewriting an archive whose layout changes) and on shader archives (written whole into the game files at build time), `remove-resource`, `merge-slices`, `add-mipmaps`, `file.put`, `track.add`, and `load_after` for the SERPs Base Files. One change keeps two converted mods from failing a build:

- `remove-record` and `remove-resource` fail when the key or resource is gone (`apply_db_op` in `addon-api/src/ops.rs:737-743`, `ErpBuild::apply` in `core/src/erpbuild.rs:396-401`). Two mods built from the same older database or archive both remove what it lacks, and the second addon's removal fails the build. Wanted: a removal of a key or resource an earlier op in the same build already removed is left out, with a line such as `erp: <addon>: eaid://… already removed by <earlier addon>`, as the Simplified Importer's build hook already does for library mods (`lib.rs` `without_repeats`).
- Launcher docs: `docs/reference/mods/serps.md:102` still says every resource goes into the `vfx_package/common.erp` overlay; asset group archives and shader archives now get overlays of their own (rows 26, 30), and the launcher writes shader archives whole into the game files at build time.

## Cannot be identical

- **Conflicts between mods** follow v4 and the launcher's load order: the later addon wins a record, a text key or a resource. v3 keeps the clean value when two mods change the same record or key (rows 16, 23). Mods that share an archive merge their changes; the Simplified Launcher lets the later archive restore the game's copy of everything it carries unchanged (row 32).
- **Text in every language** follows v4. v3 gives all eleven languages the union of a mod's edits, so a mod with its own translations shows its last file's text everywhere (row 20).
- **Text without an exe patch.** The loader adds text at runtime; there is no patched exe and no anticheat bypass (rows 24, 43).
- **Programs.** `.dll` and `.asi` files are never copied (row 46).
- **Short texture arrays** merge over the game's array instead of replacing it (row 33).
- **` -x` exclusivity** is inferred from what parts write (row 8).
