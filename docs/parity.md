# Parity with the Simplified Launcher

How the Mod Importer's conversion compares with how Team Simplified's Simplified Launcher installs the same mod.

## Sources

- **v4**: the Simplified Launcher's C# source, `Simplified Launcher/src/SimplifiedLauncher/` in the Simplified Suite. Paths below are relative to `Mods/` unless they name another project.
- **v3**: Simplified Launcher v3.2.13, the release players use. Its `app/original/` holds the clean database, the eleven language files, `2026_team_keys.lng`, `2026_team_names.json` and seven `ui_texarrays_*.erp` archives.
- **Ours**: this repository (`src/`) and the launcher ops that apply a converted addon (`packages/addon-api`, `packages/core` and `docs/reference/ops/` in SwoftyLauncher).

Status: **identical** gives the same game state for the mod on its own; **different** says how; **missing** has no counterpart in ours.

## Matrix

### Import

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 1 | Archive formats | Any format SharpCompress reads, by content (`ModImport.cs:301-338`) | zip, 7z, rar and folders; 7-Zip or UnRAR from `PATH` for the rest (`package.rs:127-142`, `252-334`) | different: tar and gzip need 7-Zip |
| 2 | Unsafe entries | Absolute paths, `..`, `:` refused and counted (`ModImport.cs:391-403`) | Same rule (`package.rs:373-385`) | identical |
| 3 | `mod.serps` | `mod_name` or `name`, `author`, `version`, `game`, `game_version`, BOM, comments, trailing commas (`ModManifest.cs:24-82`) | Same fields plus `studio_version` (`serps.rs:7-116`) | identical |
| 4 | Wrapper folder | Stepped into only when it holds `mod.serps` (`ModImport.cs:418-426`) | Same (`package.rs:387-397`) | identical |
| 5 | Game folder by name | Shallowest folder named exactly `F1 25`, three levels (`ModStaging.cs:76-85`, `139-141`) | Shallowest folder whose name starts with `F1 25` and holds game folders, else the first named so (`package.rs:474-489`) | identical for every mod named `F1 25`; ours also takes `F1 25 (name)` |
| 6 | Game folder by content | Markers `asset_groups`, `actionmaps`, `audio`, `videos`, `game_package`, `character_package`, `*_package`, `*_package_shared`, `*_asset_groups`, `shader_package_*` (`ModStaging.cs:57-64`, `147-166`) | Same markers plus `localisation` and `localisatioa` (`package.rs:19-30`) | identical; ours also finds a text-only mod |
| 7 | Loose game folders at the mod root | Only the marker folders move into `F1 25`; `mod.serps`, read-mes and other root files stay out (`ModStaging.cs:96-106`) | Every file under the root is a game file (`library.rs:349-389`) | different: root files become `file.put` |
| 8 | Unrecognised mod | Imported, installs nothing (`ModStaging.cs:88`, `ModStack.cs:102-106`) | Refused with a reason (`library.rs:282-289`) | different (deliberate) |
| 9 | Re-import | New copy staged, old moved aside, then deleted (`ModImport.cs:146-181`) | Same (`library.rs:330-347`) | identical |
| 10 | Programs | Laid like any file (`ModStack.cs:205-234`) | `.exe`, `.dll`, `.asi` and EA AntiCheat files other than the splash are skipped and reported (`package.rs:31-35`, `51-53`) | different (deliberate) |

### Database

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 11 | Unit | Per record against the clean database (`Merged.cs:46-64`, `DatabaseLayers.cs:28-74`) | Per record against `GameFiles::original` (`diff.rs:158-195`), `put-record` with the mod's raw value (`merge.rs:65-71`) | identical |
| 12 | Removed records | A stock record missing from the mod's database is removed (`DatabaseLayers.cs:54-60`) | Removed only when `mod.serps` `game_version` equals the installed exe's version; otherwise dropped with a warning (`diff.rs:110`, `187-193`) | different |
| 13 | Another game's database | Fails the install (`Merged.cs:52-56`) | Fails the import (`library.rs:367-370`) | identical |
| 14 | Order | Later ticked mod wins per record (`DatabaseLayers.cs:41-52`) | Later addon wins per record (launcher planner) | identical |

### Text

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 15 | Which files | Any `.lng` named like one of the game's eleven, in any folder (`Merged.cs:32-38`, `ModStack.cs:241-246`); other `.lng` files are laid as files | `localisation/<dir>/language_*.lng` and `localisatioa/<dir>/language_*.lng` only (`package.rs:59-66`) | different: a game language file elsewhere becomes `file.put` |
| 16 | Unit | Entries changed or added against the clean file; a missing entry is not a removal (`Merged.cs:154-160`) | Same (`diff.rs:267-292`) | identical |
| 17 | Languages a mod lacks | Its own file, else its English, else its first by name (`Merged.cs:135-145`) | Same (`diff.rs:293-317`) | identical |
| 18 | Untranslated filler | A translation set to another language's text is applied (`Merged.cs:158`) | A changed translation equal to the game's English text is left out (`diff.rs:337-346`) | different |
| 19 | Text a mod fills in | Applied like any change | `lang.default` (`diff.rs:347-364`, `convert.rs:353-370`): applied unless an addon sets the key | identical for one mod; other mods' renames win |
| 20 | How the game reads it | v4 writes merged files beside the mod's (`Merged.cs:86-127`) and holds the exe patch back (`ModStack.cs:150-153`), so the game never reads them; v3: see row 31 | `lang.set` and `lang.default` through the in-game loader (launcher `runtime.rs:240-250`, `394-403`) | different: ours is read by the game |

### Archives

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 21 | A stock archive the mod ships | The mod's file replaces the game's (`ModStack.cs:205-234`, `GameInstall.cs:33-61`) | Each resource that differs from stock goes into an overlay on `2025_asset_groups/vfx_package/common.erp` (`merge.rs:78-100`, `convert.rs:383-436`) | identical per resource when the game resolves a URI from its boot overlay first |
| 22 | Resource change test | n/a (whole file) | Type, fragment layout and packed sizes, then digest or content hash (`diff.rs:197-218`) | identical |
| 23 | Resources the mod's archive lacks | Gone with the stock file | `remove-resource` only under the same `game_version` rule as row 12 (`diff.rs:246-252`) | different |
| 24 | A stock URI in several stock archives | Only the mod's archive changes | The overlay replaces it for every archive | different |
| 25 | Simplified Studio marker `eaid://tools_package/` | Loaded with the shader archive; does nothing | Left out (`merge.rs:29-38`) | identical |
| 26 | Studio's `f1_vehicle_shadow.material` in shader archives | Loaded with the shader archive | Left out (`merge.rs:30-38`) | different when it differs from the game's |
| 27 | Archive the game does not ship | Laid as a file | `file.put` (`package.rs:67-71`) | identical |
| 28 | Texture arrays | v4: laid whole. v3: see row 32 | `merge-slices` for `.texarray.image` in `shader_package_*/ui_texture*` archives, the `.texarray` views whole (`convert.rs:449-493`); arrays elsewhere whole like any resource | see row 32 |

### Files

| # | Behaviour | Simplified Launcher | Ours | Status |
|---|---|---|---|---|
| 29 | Other files | Copied over the game's (`ModStack.cs:205-234`) | `file.put`; `.mipmaps` files `add-mipmaps` (`convert.rs:408-420`) | identical |
| 30 | Backup and restore | Backup of every replaced file and a list of added ones, restored on exit (`GameBackup.cs:84-239`) | The launcher's journal and `swofty restore` | identical in effect |

### v3.2.13 only

| # | Behaviour | Simplified Launcher v3.2.13 | Ours | Status |
|---|---|---|---|---|
| 31 | Exe patch, `localisatioa/` | Being audited | `localisatioa/` is read as text (`package.rs:17`, `72-80`) | pending |
| 32 | Seven texture array kinds | Being audited | row 28 | pending |
| 33 | SERPs Base Files | Being audited | Converts like any mod | pending |
| 34 | `2026_team_keys.lng`, `2026_team_names.json` | Being audited | none | pending |
| 35 | Parts, ` -c` and ` -x` | Being audited | One library entry per part (`package.rs:411-472`) | pending |
| 36 | Tracks | Being audited | `track.add` with the package (`convert.rs:317-331`) | pending |

## Per-mod results

Pending the parity harness.

## Launcher ops still needed

Pending.

## Cannot be identical

Pending.
