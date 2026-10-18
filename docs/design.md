# Design

How Simplified for Swofty imports, stores, compares and merges `.serps` mods and tracks. These are sections 4 and 5 of the Swofty Launcher design notes, <https://f1.swofty.net/design/addons-and-mods>, which describe the Launcher API and Asset API this addon is built on. Links to other sections go there.

## 4. Simplified for Swofty

### 4.1 The addon

- The crate `addons/simplified` (`swofty-addon-simplified`) is Rust and AGPL-3.0-or-later, like the rest of the repository. Its `mod.serps` reader, package root, content root and entry path checks are ported from the Simplified Launcher (GPL-3.0, used with Team Simplified's permission) and carry the Simplified Suite notice. `THIRD_PARTY.md` credits Team Simplified for the `.serps` format and the Simplified Suite, and the Mods section shows the credit.
- Import, the library, profile state, the section and `swofty mods` are built (work item 13), and so are the diffs (work item 14), the merge and conflicts (work item 15).
- Manifest: id `teamsimplified.mods`, name `Simplified for Swofty`, author `Team Simplified`. No options and no native plugin.
- Built in and on by default in new profiles. It can be turned off per profile, which hides its section.
- It uses `swofty-addon` and `swofty-formats` only. Core has no code for it.

### 4.2 Packages

Any mod works, as copying it into the game folder would.

- A package is a zip archive under any name, `.serps` included, a 7z or RAR archive, or a folder.
- Dropped on the launcher, a folder or zip is claimed when it holds `mod.serps` at its root, a folder whose name starts with `F1 25`, or a game folder (see the content root below) at most three folders down. 7z and RAR archives are claimed by their extension.
- 7z archives are read by `sevenz-rust2`. RAR archives, and 7z archives that reader can't read, are unpacked by 7-Zip or UnRAR from `PATH` or where their installers put them. Without either, a RAR import fails and says what to install.
- `mod.serps` at the package root is JSON: `mod_name` or `name` (`mod_name` wins), `author`, `version`, `game` (for example `F1® 25`), `game_version` (for example `1.0.141.7865`), and `studio_version` from Simplified Studio. Every field is optional. The reader accepts a UTF-8 byte order mark, comments and trailing commas.
- `F1 25/` holds files in the game folder's layout. RM Designs' Scuderia AlphaTauri ORLEN package has the whole `game_package/gamedatabases/gamedatabases.2025.bin`, 11 whole language files under `localisatioa/2025_release/` and four whole shader archives under `shader_package_2025/`.
- `localisatioa/` is the folder Simplified's launcher points a patched exe at. Both `localisatioa/` and `localisation/` name the stock language file with the same file name.
- The content root is the first folder, at most three levels down, whose name starts with `F1 25`. Without one, it is the first folder, from the top down to three levels, with a game folder in it: `asset_groups`, `actionmaps`, `audio`, `videos`, `game_package`, `character_package`, `localisation`, `localisatioa`, or a folder ending in `_package`, `_package_shared` or `_asset_groups`, or starting with `shader_package_`.

### 4.3 Import

1. The package unpacks into `addon-data/teamsimplified.mods/.import-<n>/`. Entries with absolute paths or `..` are skipped and counted.
2. The importer reads `mod.serps` and finds the content root.
3. Each file gets a class:

| Path | Class |
|---|---|
| `game_package/gamedatabases/gamedatabases.2025.bin` | Database. It must parse as F1 25's database, or the import fails |
| `localisation/*/language_<code>.lng`, `localisatioa/*/language_<code>.lng` | Text |
| A `.erp` the game ships | Archive |
| Any other file the game ships | Replacement |
| A file the game does not ship | Addition |
| `.exe`, `.dll`, `.asi`, and EA AntiCheat files other than `EAAntiCheat.splash.png` | Skipped and named in the report |

4. The id is `<author>.<name>`, lowercased to `[a-z0-9-]`, or the file name when both are missing. Importing an id that exists replaces it: the new copy is staged, the old one moves aside, the new one moves in and the old one is deleted. Profiles keep the id's place and switch.
5. The importer diffs the package ([section 4.5](#45-diff)) and writes `package.json`.

The importer reads zip archives and folders. 7z is a later item. RAR is not supported.

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

- Removals count only when `game_version` equals the installed exe's version. Otherwise they are dropped, and the row says how many and for which version the package was made.
- A language the package lacks takes the changes of the package's `language_eng.lng`, or of its first language file by name.
- Archive diffs read the index only (`ErpFile::open`). The database diff parses both files once.
- Diffs are cached per game build key in `diff/<build key>.json`, with the changed and added records in `diff/<build key>.db`. Import computes the diff when a game is found. A mod without a diff for the installed build gets one at the next build, or from `Compare` in the section.
- Without a diff for the installed build, the expanded row lists the package's files by class.

### 4.6 Merge

The build hook emits ops for each enabled mod, in profile order, with origin `teamsimplified.mods` and the mod id:

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
- Every resource a mod adds or changes, in any archive, is put into `2025_asset_groups/vfx_package/common.erp`. Its overlay loads with the boot groups that list that archive, before any car or UI group, and overrides the game's copies, as Simplified Studio's shader archives do. A replacement may change a resource's size, which a replacement in its own archive can't at runtime. With a launcher that writes one overlay archive per origin, each mod has its own, numbered in load order.
- A write that repeats the value already in place, from an earlier mod, is left out.

#### Shader archive carriers

Simplified Studio packages carry a mod's textures and models in archives under `shader_package_2025/`, which the game loads at boot. No asset group the loader patches lists those archives, so an overlay targeting them is never read. This is built.

- Their resources load with `common.erp` like every other resource a mod adds or changes.
- Team logo texture arrays carried in `ui_texture*` archives are merged across every enabled mod. Each `.texarray.image` starts from the game's own array, from the first of eight `ui_package` archives that holds it. Fragment 1 is split into the array's slices, and each mod's slices that differ from the game's go in, later mods winning. An array built for an older game, with fewer slices of the same format and size, merges over the slices it has. An array of another format is used whole, and the build warns. The result is written to `cache/addons/teamsimplified.mods/ui_textures.erp` and put into `common.erp` with the addon as its origin, so it loads as an overlay of its own, apart from every mod's. Ported from the Simplified Launcher's `merge_resources.py` and the Team Editor's logo code.
- Simplified Studio's `eaid://tools_package/` marker is left out.

#### Text a package fills in

Team rename packages point the team name records at keys of their own, such as `lng_team_ast_title_26`, and give every team a value: the renamed team's new name, and the game's text for the rest. Those values would undo another package's rename.

- An added key whose text is the game's text of the key the stock record pointed at is a default, kept in the diff's `defaults`. English text counts too, since packages fill in untranslated names.
- A changed translation whose text is the game's English text of the same key is left out.
- Defaults go in once for each key no package renames, before every package's own changes. The diff has a `version`, and a cached diff of another version is computed again.

### 4.7 Conflicts

- The section computes conflicts between enabled mods from their cached diffs, in profile order. A mod counts once for each target. Simplified Studio's marker and the logo texture arrays merged slice by slice are not targets. A row shows `Overrides <n> in <mod>` when it wins and `<n> overridden by <mod>` when it loses. Its expanded body lists the targets under `CONFLICTS`: resource URIs with their archive, record keys in hex, text keys with their language, and files.
- The build reports conflicts with other addons and maps through the planner ([section 3.5](https://f1.swofty.net/design/addons-and-mods#_3-5-order-and-conflicts)).

### 4.8 Section

The sidebar entry `Mods` comes after `Addons`, with the library size as its count.

- **Header.** Title `Mods`. Subtitle `<n> in library · <k> on in <profile> · lower in the list wins`. Action `Import…`, a picker for `.zip`, `.7z`, `.rar` and `.serps` files and folders, several at once.
- **Notices.** The addon is off in this profile (button `Turn on`). The vanilla profile is active. Mods are not compared with the installed game build yet (button `Compare`).
- **Rows.** Reorderable, in load order. Title from `mod.serps`, subtitle `<author> · <version>`, tags `Database`, `Text`, `<n> archives`, `<n> files`, conflict tags and `Made for <game_version>` when it differs from the game. A switch turns the mod on in the profile.
- **Expanded row.** Stats: status, version, author, made for, Studio version, size, imported. `WHAT IT CHANGES`: records changed, added and removed; text entries by language; resources by archive; files. `CONFLICTS`. Buttons: Move up, Move down, Show files, and Remove from library with a confirmation.
- **Empty.** `No mods yet`, `Drop a mod on the window, or import its zip, 7z, rar or folder. Mods made with Simplified Studio import too.` and `Import…`.
- **Footer.** `Works with any F1 25 mod: its files go where copying them into the game folder would put them. Made by Team Simplified. Based on Simplified Suite by Team Simplified.`
- **Profile cards.** `summary` adds `MODS` with the enabled mods' names.
- **CLI.** `swofty mods list`, `import <path>…`, `remove <id>`, `enable <id>`, `disable <id>`, `move <id> <position>`.

## 5. Tracks are mods

Tracks are `.serps` mods in the Team Simplified library. There is no tracks addon and no Maps page. This is built.

- **Raw track zips.** A zip that holds a track package (`manifest.json` with `track_code`), or a download that holds one such zip, is claimed by Team Simplified. Import stores it as `library/<id>/` with `package.json` (`track` set to the code), a `mod.serps` written from the manifest, and the package under `track/`. The id is the track code, or `<code>-track` when a mod in the library already has that id. A track already in the library keeps its id and is replaced.
- **Track SERPs.** A package built by the Simplified Track Installer (the track archive in an invitational slot, mipmaps under `environment_package/tracks/<code>/mipmaps/`, the `track_info` shader archives and optionally a whole game database) imports, diffs and merges like any other `.serps` mod ([sections 4.3 to 4.6](#43-import)), with the slot as shipped.
- **Build.** For each enabled raw track, in profile order, the build hook emits `track.add` with `source` `{ "root": "data", "path": "library/<id>/track" }`. Core processes `track.add` ops with `activation/` before other database ops: the database rows, the archive, the menus and ERS. With `group` in the loader's `features.json`, the archive goes to its natural path `2025_asset_groups/environment_package/tracks/<code>/<code>.erp` with `group.register` ops and the natural resource-row values, and there is no limit on tracks. Otherwise the tracks take the invitational slots, at most eight.
- **Migration.** `Addon::migrate` runs from `Store::migrate` (`data_version` 3). Team Simplified moves each `$DATA/maps/<code>/` into the library and each profile's `maps` list into its state, enabled, after the mods already listed. The track's id is its code, or `<code>-track` when a library mod already has that id. Each track is staged under `library/.migrate-<folder>/`, and the next run finishes a staged track. A code that matches no track in the library is dropped. `Profile` still reads `maps` and no longer writes it.
- **Removed from core.** The Maps page and sidebar entry, `Profile.maps`, `Store::maps`, `import_map`, `remove_map`, the core track importer and the `swofty maps` commands. `swofty maps` is a hidden alias of `swofty mods`.
