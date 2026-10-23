# Design

How Simplified for Swofty, Team Simplified's addon, uses its Simplified Importer to import, store and compare mods and tracks and turn each into a Swofty addon. These are sections 4 and 5 of the Swofty Launcher design notes, <https://f1.swofty.net/design/addons-and-mods>, which describe the Launcher API and Asset API this addon is built on. Links to other sections go there.

## 4. Simplified for Swofty

### 4.1 The addon

- The crate `addons/simplified` (`swofty-addon-simplified`) is Rust and AGPL-3.0-or-later, like the rest of the repository. Its `mod.serps` reader, package root, content root and entry path checks are ported from the Simplified Launcher (GPL-3.0, used with Team Simplified's permission) and carry the Simplified Suite notice. `THIRD_PARTY.md` credits Team Simplified for the `.serps` format, the Simplified Suite and the logo, and the Simplified Importer page shows the credit.
- Import, the library, profile state, the section and `swofty mods` are built (work item 13), and so are the diffs (work item 14), the merge and conflicts (work item 15).
- Manifest: id `teamsimplified.mods`, name `Simplified for Swofty`, author `Team Simplified`, version 0.7.0, `compat.launcher` `>=1.3.0`. No options and no native plugin.
- Ships as a launcher plugin: `teamsimplified.mods.swoftyaddon` holds `swofty.json` and the helper program `bin/simplified-helper`, which hands the addon to `swofty_addon::helper::run_helper`. `swofty.json` declares the one section, id `mods`, titled `Simplified Importer`, the `mods` command, the import extensions and the build hook, and the addon reads its sections and commands from it. `sections/mods.svg`, Team Simplified's mark, is the section's sidebar icon. The launcher can still build the crate in through `swofty-builtins`; built in, it is on by default in new profiles. Either way it can be turned off per profile, which hides its section.
- It uses `swofty-addon` and `swofty-formats` only. Core has no code for it.

### 4.2 Packages

Any mod works, as copying it into the game folder would.

- A package is a zip archive, a 7z or RAR archive, or a folder. A `.serps` file is refused: it is Simplified Studio's description of a mod, not the mod.
- Dropped on the launcher, a folder or zip is claimed when it holds `mod.serps` at its root, a folder whose name starts with `F1 25`, or a game folder (see the content root below) at most three folders down. 7z and RAR archives are claimed by their extension.
- Zip archives may use Deflate, Deflate64, BZip2, LZMA, XZ, PPMd or Zstandard. 7z archives are read by `sevenz-rust2`, and RAR archives of every version, RAR 1.3 to 7, by `rars`. An archive either reader can't read is unpacked by 7-Zip or UnRAR from `PATH` or where their installers put them, when one is installed.
- `mod.serps` at the package root is JSON: `mod_name` or `name` (`mod_name` wins), `author`, `version`, `game` (for example `F1® 25`), `game_version` (for example `1.0.141.7865`), and `studio_version` from Simplified Studio. Every field is optional. The reader accepts a UTF-8 byte order mark, comments and trailing commas.
- `F1 25/` holds files in the game folder's layout. RM Designs' Scuderia AlphaTauri ORLEN package has the whole `game_package/gamedatabases/gamedatabases.2025.bin`, 11 whole language files under `localisatioa/2025_release/` and four whole shader archives under `shader_package_2025/`.
- `localisatioa/` is the folder Simplified's launcher points a patched exe at. Both `localisatioa/` and `localisation/` name the stock language file with the same file name.
- The content root is the first folder, at most three levels down, whose name starts with `F1 25`. Without one, it is the first folder, from the top down to three levels, with a game folder in it: `asset_groups`, `actionmaps`, `audio`, `videos`, `game_package`, `character_package`, `localisation`, `localisatioa`, or a folder ending in `_package`, `_package_shared` or `_asset_groups`, or starting with `shader_package_`.
- **Parts.** A mod can come in parts: folders that each hold game files of their own, such as `2026 Broadcast graphics/shader_package_2025/` beside `Realistic Tyre colours/shader_package_2025/`. A part is the folder above a game folder or a folder named like `F1 25`, unless it sits inside another part's game files; a folder beside the game folders, such as `UI/F1 25/`, is a part of its own. A folder named like `F1 25` counts as the game folder only when it holds the game's own folders, so in `F1 25 Engine Sounds for 2026 Cars/F1 25/audio/` the outer folder is the part. A mod with two or more parts imports as one library entry per part, `<mod id>.<part>` named `<mod name> › <part>`, each with its own switch, place in the load order, diff and conflicts. ` -c` and ` -x` at the end of a folder name are left out of the part's name, as the Simplified Launcher shows them. Ported from the Simplified Launcher's `list_mods.py`.

### 4.3 Import

1. The package unpacks into `addon-data/teamsimplified.mods/.import-<n>/`. Entries with absolute paths or `..` are skipped and counted.
2. The importer reads `mod.serps` and finds the content root.
3. Each file gets a class:

| Path | Class |
|---|---|
| `gamedatabases.2025.bin` in any folder, which goes to `game_package/gamedatabases/` | Database. It must parse as F1 25's database, or the import fails |
| `language_<code>.lng` in any folder, such as `localisatioa/2025_release/` | Text |
| A `.erp` the game ships | Archive |
| Any other file the game ships | Replacement |
| A file the game does not ship | Addition |
| `.exe`, `.dll`, `.asi`, files whose names start with `EAAntiCheat` other than `EAAntiCheat.splash.png`, `EAJavelin` or `preloader_` | Skipped and named in the report |
| With game folders loose at the content root: a file at the top that the game does not ship, or in a folder that is not a game folder, such as a read-me | Skipped and named in the report |

4. The id is `<author>.<name>`, lowercased to `[a-z0-9-]`, or the file name when both are missing. Importing an id that exists replaces it: the new copy is staged, the old one moves aside, the new one moves in and the old one is deleted. Profiles keep the id's place and switch.
5. The importer diffs the package ([section 4.5](#45-diff)) and writes `package.json`.

The importer reads zip, 7z and RAR archives and folders. Encrypted archives are not supported.

### 4.4 Library

```
$DATA/addon-data/teamsimplified.mods/library/<id>/
  package.json          name, author, version, game_version, studio_version, source name, sha256, import time, file list
  mod.serps             as shipped
  files/                the content root, by game path
  diff/<build key>.json the diff for one game build
  diff/<build key>.db   its database records
```

The library is shared by every profile. Each profile's state lists the mods it uses, in load order:

```json
"state": {
  "teamsimplified.mods": {
    "mods": [{ "id": "rm-designs.scuderia-alphatauri-orlen", "enabled": true }]
  }
}
```

A mod not in the list is off and shows after the list, by name. Removing a mod from the library removes it from every profile.

### 4.5 Diff

Each mod is compared with clean files from `GameFiles`, never with the game folder as modded:

| Class | Unit | Changed | Added | Removed |
|---|---|---|---|---|
| Database | record key | same key, other tag or bytes | key not in stock | stock key missing from the package |
| Archive | resource URI | type, fragment layout or sizes differ, or else the index digest (fragment data when a digest is empty). Mod tools keep the game's digest on resources they rewrite | URI not in stock | stock URI missing from the package |
| Text | language and key | value differs | key not in stock | never |
| Replacement, Addition | file | whole file | whole file | never |

- Removals apply only when `mod.serps` names the installed game version; otherwise the report says how many were left out. A database exported from an older version lacks every record later updates added, and the game crashes at boot when it reads one (a 1.0.138 database on 1.0.141 removed 14,618 records and crashed in `Sess_Apply_ContentAvailability`). A shader archive removes nothing: the Simplified Launcher's Python shader merge (`app.lib.erp.merge_collected_shader_erps`) unions the game's archive with the mod's, keeping the stock resources the mod omits. A boot test on real F1 25 confirmed it: dropping the base shader a Studio livery omits crashes the game (ACCESS_VIOLATION in `.xtls` while loading `fom_ast` textures), while keeping it boots to the frontend.
- A language the package lacks takes the changes of the package's `language_eng.lng`, or of its first language file by name.
- Archive diffs read the index only (`ErpFile::open`). The database diff parses both files once.
- Diffs are cached per game build key in `diff/<build key>.json`, with the changed and added records in `diff/<build key>.db`. Import computes the diff when a game is found. A mod without a diff for the installed build gets one at the next build, or from `Compare` in the section.
- Without a diff for the installed build, the expanded row lists the package's files by class.

### 4.6 Merge

The build hook merges only the mods a profile has not handed over to their addons ([section 4.9](#49-conversion)): mods that failed to convert, and every mod of a profile not handed over yet. It emits ops for each such enabled mod, in profile order, with origin `teamsimplified.mods` and the mod id:

| Class | Ops |
|---|---|
| Database | `db.put-record`, `db.remove-record` |
| Archive | `erp.put-resource` from the mod's archive, `erp.remove-resource` |
| Text | `lang.set` |
| Replacement, Addition | `file.put` |

- The planner applies them after map activation and earlier addons. A package's database changes touch only the records it changed, and archives merge per resource.
- A special livery row a package adds is reported as owned through `owned_items` ([section 3.4](https://f1.swofty.net/design/addons-and-mods#_3-4-deliveries)), and the livery appears in the picker.
- The ops take the planner's routes ([section 3.4](https://f1.swofty.net/design/addons-and-mods#_3-4-deliveries)). Without the in-game loader, text ops wait and the build reports how many entries wait.
- Resource removals are `erp.remove-resource` ops on the archive the mod removed them from, and put every `erp` op on the build route. A removal of a record or resource an earlier mod already removed is left out.
- Database records are written as raw `db.put-record` values from `diff/<build key>.db`. Archive and file sources are `{ "root": "data", "path": "library/<id>/files/<game path>" }`.
- A resource a mod adds or changes in an archive under `2025_asset_groups/` goes into an overlay on that archive, as the mod's archive replaces the game's there and nowhere else. Its stock copies in other archives stay. A replacement that changes a resource's layout makes the launcher rewrite that archive into the game files, as the Simplified Launcher copies the mod's archive over the game's.
- A resource from a shader archive, or from any other archive no asset group lists, goes into `2025_asset_groups/vfx_package/common.erp`. Its overlay loads with the boot groups that list that archive, before any car or UI group. With a launcher that writes one overlay archive per origin, each mod has its own, numbered in load order.
- A write that repeats the value already in place, from an earlier mod, is left out.

#### Shader archive carriers

Simplified Studio packages carry a mod's textures and models in archives under `shader_package_2025/`, which the game loads at boot. No asset group the loader patches lists those archives, so an overlay targeting them is never read. Team Simplified keeps them out of the game's shader archives: the mod's resources go into an overlay archive of its own, built at import, which loads with the game's archives.

- Their resources load with `common.erp`. The game's stock shader archive stays in place, so the stock resources a mod's archive omits, such as an archive's base shader (`object_auto_illuminate.nefx2` and the like), are still loaded.
- A shader archive comes as a generic build, `<name>.sm51.erp`, and an AMD build, `<name>.sm51_amd.erp`, and the game loads the one for its GPU. A resource a mod carries in both loads once, from the generic build. When the two copies differ, the report warns that AMD GPUs get the generic copy.
- Team logo texture arrays carried in `ui_texture*` archives are merged across every enabled mod; anything else those archives add loads with the mod's own archive. Each `.texarray.image` starts from the game's own array, from the first of eight `ui_package` archives that holds it. Fragment 1 is split into the array's slices, and each mod's slices that differ from the game's go in, later mods winning. An array built for an older game, with fewer slices of the same format and size, merges over the slices it has. An array of another format is used whole, and the build warns. The result is written to `cache/addons/teamsimplified.mods/ui_textures.erp` and put into `common.erp` with the addon as its origin, so it loads as an overlay of its own, apart from every mod's. Ported from the Simplified Launcher's `merge_resources.py` and the Team Editor's logo code.
- A mod that loads resources from a shader package, as Simplified Studio and My Team mods do, needs Team Simplified's SERPs Base Files. Simplified Studio's marker and vehicle shadow material don't count. Its record says so (`made.base`). Every converted addon's `origin.tool` is `teamsimplified.mods`, which the launcher tags Simplified; 0.10.0 gave the others `teamsimplified.mods/copy-paste`, which startup switches back. Such a mod's description says what its shader packages replace, from Team Simplified's list in `merge::REPLACES` (`object_flag` is the Ferrari team, `ui_texture` the UI), and "Made using the Simplified Studio." when `mod.serps` gives a Studio version; every description then lists the mod's counts. At startup, mods converted before 0.10.0 are flagged from their import report and lose the old "Converted from …" prefix. While such an addon is on, while such an addon is on, the build hook adds the three SERPs Base Files v4.1 archives the addon ships under `base/` as overlays in place of the game's `vfx_package/common.erp`, `f1_2025_vehicle_package/teams/common.erp` and `f1_2026_vehicle_package/teams/common.erp`, as the Simplified Launcher installs them. My Team mods hang on the loading screen without them.
- Simplified Studio's `eaid://tools_package/` marker is left out, and so is the `…/shared/shadows/materials/f1_vehicle_shadow.material` it adds to every shader package, which would otherwise replace the game's vehicle shadow material.

#### Text a package fills in

Team rename packages point the team name records at keys of their own, such as `lng_team_ast_title_26`, and give every team a value: the renamed team's new name, and the game's text for the rest. Those values would undo another package's rename.

- An added key whose text is the game's text of the key the stock record pointed at is a default, kept in the diff's `defaults`. English text counts too, since packages fill in untranslated names.
- A changed translation whose text is the game's English text of the same key is a default too: it applies, as in the Simplified Launcher, unless a package renames the key.
- Defaults go in once for each key no package renames, before every package's own changes. The diff has a `version`, and a cached diff of another version is computed again.

### 4.7 Conflicts

- Conflicts between converted mods are conflicts between their addons, which the launcher reports on the **Addons** page.
- The build reports conflicts of the mods that still load through the build hook with other addons through the planner ([section 3.5](https://f1.swofty.net/design/addons-and-mods#_3-5-order-and-conflicts)).

### 4.8 Simplified Importer page

The section `mods`, titled `Simplified Importer`, is one of the launcher's big sidebar sections. Its body is blocks:

- **Header.** Subtitle `Adapt your copy and paste mods to work with the Swofty Launcher.` Action `Choose a mod…`, a picker for `.zip`, `.7z` and `.rar` files and folders, several at once.
- **Notices.** The addon is off in this profile (button `Turn on`). The vanilla profile is active. The launcher has no addon host calls. Library mods are not converted yet and convert on the next start, or this profile is not handed over yet (button `Convert now`).
- **Import a mod.** A card with `Choose a mod…`: a drop anywhere on the window imports too.
- **Credit.** The last block is `Based on the Simplified Suite by Team Simplified.`, part of the page, not a fixed footer.
- **Vanilla.** The page shows no `Import a mod`, only a notice to switch profiles to import.
- **Profile cards.** `summary` adds `MODS` with the enabled mods that still load through the build hook, and nothing when there are none.
- **CLI.** `swofty mods list` shows each library mod with its addon or why it was not converted. `import <path>…` converts as the page does. `remove <id>` deletes a mod from the library and leaves its addon. `enable`, `disable` and `move` change the mods that load through the build hook.

### 4.9 Conversion

Every mod becomes a standalone Swofty data addon. `src/convert.rs` writes one addon folder for one library mod; `src/importer.rs` installs it and keeps the record.

1. A drop on the window (`import`), `Choose a mod…` (a `picked` event) or `swofty mods import` unpacks each path into the library and diffs it ([section 4.3](#43-import)). The library copy is only for converting: it is deleted once the addon is installed, and when the mod can't convert.
2. Each library entry converts into `addon-data/teamsimplified.mods/.convert/<mod id>/`, which `install_addon` installs. The staging folder is deleted after.
3. Outside the vanilla profile, `set_addon_enabled` turns the addon on in the active profile and the profile records the mod as handed over.
4. A `picked` event returns `{ "show_addon": "<addon id>" }` for the first addon, and the launcher switches to **Addons** with it highlighted. An import with no addon made returns a notice, and one where every file failed returns an error.
5. Importing a mod again converts it again and installs it over the same addon id. The addon keeps its switch.
6. Converting compares a mod with the game's own files. Without a game, or on a launcher with no addon host calls, a mod is not imported. Track packages convert without a game. Mods left in the library from before (section "Mods imported before 0.7.0" in the README) convert at startup or on `Convert now`, and their copies are deleted once converted.
7. Report lines that start with `warning:` show as warnings on the page.
8. A mod already converted keeps its addon id, also when the importer now names its parts differently.
9. Each conversion writes `logs/convert-<UTC time>-<addon id>.log` in the addon's data folder, one fact per line: the source file, its size and SHA-256, the game build, the file classes, the diff, the ops written per archive and language, each file, resource or removal left out with its reason, warnings, errors and timings, and the addon made. The newest 20 logs are kept, and the home folder shows as `~`.

#### Names and ids

- A mod's name comes from `mod.serps`, else from the zip or folder name. A version at the end of that name, such as `v1.0.6` or `(V10.3)`, becomes the version, and `- By <author>` the author, when `mod.serps` gives none.
- A part is named by the folders that tell it apart, innermost first, then the mod: `Pirelli Season 2011 · Default Weather Grip Track — Pirelli Tyres Collection`. Folders every part sits in and folders named like the mod are left out.
- A part's library id is the mod's id, a dot and a slug of those folders, at most 32 characters, cut at a word with a 6-digit hash of the part's folder after it. Addon ids are `mod.<library id>`, at most 64 characters. A part imported under the old id, the mod id and a slug of its whole folder, keeps that id.
- A track zip's `origin.sha256` is the zip's SHA-256.

#### Variants

Parts of one import that write mostly the same things are variants. Two parts are variants when the records, resources, text entries and files they both write are at least half of everything the two write. All variants install. The one on is the variant that was on before a re-import, else the first whose part name has the word `Default` or `Standard`, else the first; the rest install off. Parts that are not variants all turn on. The import notice and `Last import` say how many variants installed and which is on. A `Choose a mod…` import with variants returns `show_addon` with that notice, so the launcher highlights the first addon and shows the notice.

#### What the addon holds

- Database records are `put-record` and `remove-record`. Archive resources go into overlay archives with `erp.overlay`, and removals are `remove-resource`. Loose files are `file.put`, and `.mipmaps` files `add-mipmaps`. A track is `track.add` with its package.
- Text is `lang.set`. Text a mod fills in with the game's own text is `lang.default`, which never undoes another addon's rename.
- Team logo texture arrays from `ui_texture*` archives ship in `logos/ui_textures.erp` as the mod has them, with one `erp.merge-slices` each into `shader_package_2025/ui_texture.nefx2.sm51.erp` and `_amd.erp`; the launcher starts from the game's array in `ui_package` and writes only the slices that differ from it. Texture array views (`.texarray`) go in whole with `put-resource`.
- `kind` lists `database`, `erp`, `text` and `files` as the addon has them.
- `category` comes from what the mod changes. A track package or mostly track resources is Tracks. 1000 or more records with car resources or team and driver text is Seasons. Mostly car resources is Cars, or Liveries when they are all textures and materials and not tyres. Then Audio, Drivers & teams, Database, UI and Other. The name decides only between Database and Seasons (`season`, `calendar`) and towards Liveries (`livery`).

The record is `addon-data/teamsimplified.mods/importer.json`:

```json
{
  "mods": {
    "rm-designs.scuderia-alphatauri-orlen": {
      "name": "Scuderia AlphaTauri ORLEN", "at": 1792000000,
      "made": { "id": "mod.rm-designs.scuderia-alphatauri-orlen", "name": "Scuderia AlphaTauri ORLEN",
                "category": "liveries", "kinds": ["Database", "Archives"], "lines": ["…"] }
    }
  },
  "last": { "at": 1792000000, "files": ["AlphaTauri.zip"], "mods": ["rm-designs.scuderia-alphatauri-orlen"] }
}
```

- `made` is the addon made from the mod. `error` is why the last conversion failed. A failed conversion of a mod converted before keeps the old `made`.
- Each profile's state lists the mods it has handed over in `addons`. The build hook leaves out a mod that has `made` and is handed over.

#### Migration

Libraries from before 0.7.0 convert once, on the first launcher start, the first import or `Convert now`, and each profile is handed over on its own.

- `launcher.startup` in `swofty.json` asks the launcher for a `startup` event on each start. With nothing pending it returns `none` at once. Otherwise it runs the migration and returns a notice: `Converted <n> mods into addons`, the mods not converted, and how many are left.
- A startup conversion starts no mod it expects to end past 270 seconds, at 40 MB a second, after the first. The rest convert on the next start or with `Convert now`.
- While the migration runs, at startup or from `Convert now`, the launcher shows a checklist (the `tasks` host call): `Migrating your mods to Swofty addons`, one row per mod to convert, waiting, running, done with its category, or failed with the reason. A full list goes out each time a row changes, mods past the time budget stay waiting with `converts on next start`, and nothing goes out when nothing is pending. A launcher without the call shows nothing.

1. Each library mod without a record entry converts and installs, in the active profile's mod order, then the rest. The record is saved after each mod, and a later run converts only what is left. An import that runs the migration does not convert a mod the migration just made from the same files again.
2. For the active profile, the addons of the mods it lists turn on or off as each mod was, in the profile's mod order, and append to the profile's addon list in that order. With the `move_addon` host call, each then moves to just after `teamsimplified.mods`, or after the addon of the mod before it, where the build hook loaded the mods. The current order comes from each addon's `position` in the `addons` host call. A launcher before 1.3.0 gives no `position`, and the profile file, `profiles/<profile>.json` two folders above the addon's data folder, is read instead. A launcher without `move_addon` leaves them appended.
3. A profile not handed over yet loads its mods through the build hook, as before, and the page offers `Convert now`.
4. Mods that fail to convert stay in the library and in each profile's mod list, and the build hook keeps loading them.

The library is never deleted: it is the source each addon is made from again.

## 5. Tracks are mods

Tracks are mods in the Team Simplified library, and each converts into a track addon. There is no Maps page and no Tracks page.

- **Raw track zips.** A zip that holds a track package (`manifest.json` with `track_code`), or a download that holds one such zip, is claimed by Team Simplified. Import stores it as `library/<id>/` with `package.json` (`track` set to the code), a `mod.serps` written from the manifest, and the package under `track/`. The id is the track code, or `<code>-track` when a mod in the library already has that id. A track already in the library keeps its id and is replaced.
- **Track SERPs.** A package built by the Simplified Track Installer (the track archive in an invitational slot, mipmaps under `environment_package/tracks/<code>/mipmaps/`, the `track_info` shader archives and optionally a whole game database) imports, diffs and merges like any other `.serps` mod ([sections 4.3 to 4.6](#43-import)), with the slot as shipped.
- **Build.** For each enabled raw track the profile has not handed over to its addon, in profile order, the build hook emits `track.add` with `source` `{ "root": "data", "path": "library/<id>/track" }`. Core processes `track.add` ops with `activation/` before other database ops: the database rows, the archive, the menus and ERS. With `group` in the loader's `features.json`, the archive goes to its natural path `2025_asset_groups/environment_package/tracks/<code>/<code>.erp` with `group.register` ops and the natural resource-row values, and there is no limit on tracks. Otherwise the tracks take the invitational slots, at most eight.
- **Migration.** `Addon::migrate` runs from `Store::migrate` (`data_version` 3). Team Simplified moves each `$DATA/maps/<code>/` into the library and each profile's `maps` list into its state, enabled, after the mods already listed. The track's id is its code, or `<code>-track` when a library mod already has that id. Each track is staged under `library/.migrate-<folder>/`, and the next run finishes a staged track. A code that matches no track in the library is dropped. `Profile` still reads `maps` and no longer writes it.
- **Track addons.** A track package converts into a track addon like any other mod ([section 4.9](#49-conversion)): `track.add` with the package inside the addon, category `tracks`. Track addons show on the **Addons** page under Tracks, and the Tracks page of 0.6.0 is gone.
- **Removed from core.** The Maps page and sidebar entry, `Profile.maps`, `Store::maps`, `import_map`, `remove_map`, the core track importer and the `swofty maps` commands. `swofty maps` is a hidden alias of `swofty mods`.
