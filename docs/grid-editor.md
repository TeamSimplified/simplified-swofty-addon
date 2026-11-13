# Grid Editor

The **Grid Editor** page of Simplified for Swofty, by Team Simplified, below the About and
Simplified Importer pages. It has the grids down the left: DEFAULT (Formula 1 and Formula 2), OTHER (the grids
addons add) and MISC (Presets, Driver Profiles and My Team Emblem). Each grid's teams are
in a table, where clicking a team opens a panel with its status, **Save preset**,
**Export addon** and **Import preset**, its team selection logo with **Replace
logos** (the team's page, which holds its logos), and the team's name, car numbers, team
principal and colours to edit in place. Clicking a driver there offers **Edit driver** and **Replace
driver**, and clicking the principal **Replace team principal**; the long lists only
load then. It edits:

- **Grids:** F1 2026, F1 2025, F2 2026, F2 2025 and F2 2024, plus a tab for every grid an
  enabled addon adds, such as the Classic Seasons years. A season that defines its teams
  in the launcher's registry, such as the F1 2012 season, shows its teams as the game
  makes them at startup; their names, colours, logos and principal are that addon's, and
  their line-ups and drivers can be edited. F1 2025 includes the optional
  Konnersport and APXGP teams; renaming them renames their menu entries too.
  Progressive Career's seasons get no tabs of their own: it plays the Classic Seasons
  years and the game's 2025 and 2026 with those grids' teams, so they're edited on those
  tabs. Team names and logos reach it as they reach the grid. Drivers reach it through
  Simplified's DLL (`native/src/grid_editor.cpp`): the build puts the seats edited on
  those grids in `profile/grid-editor.json`, in Simplified's folder in the game, by year
  and career slot, finding each seat by the driver Progressive has where the grid had
  him; the DLL layers them on Progressive's `career.season` records for the profile that
  played. A new career races them from its first season, a running one from the next
  season it starts. Car numbers stay Progressive's. A driver Progressive has no look for
  in that year wears its default look for him, which takes the suit and gloves the
  seat's own driver wears that year and the helmet of his own nearest year.
  A grid an addon defines as a `content.package` record, as each Classic Seasons year
  does since Classic Seasons 0.7, is made by the game when it starts, after the build's
  database edits, so its drivers and car numbers can't be database edits. The build puts
  them in the same file instead, by record, team key and seat, each seat found by the
  driver the record gives it, and the DLL layers them on the record, one layer a seat. A
  driver is named by a record that resolves: `game/<label>` for the game's own, the
  record an addon registers him by (`swofty.classic-seasons/<label>`), else
  `person/<label>|game/<label>`. A new driver brings his own entrant's look and number,
  else wears the seat's look, which goes in the package's own looks rather than as a
  reference. An exported addon leaves those seats out and says so.
- **Suppliers:** each team's engine and tyre brand, in its panel, named with the year:
  Mercedes-AMG 2026, Pirelli 2026. The engines are the game's of the cars' era (the 2026
  ones for 2026 cars) and, with More Engine Suppliers on, those its last build added, from
  its `suppliers.json`, under its names for the game's engines. The build sets each of the
  team's cars' `engine`, and clears the `engine_name_override` the car gave its old engine,
  so the new one shows its own name; it builds after More Engine Suppliers, whose engine
  rows exist only once it builds. An exported addon leaves its engines out. A grid the game
  makes when it starts keeps its cars' engines. The tyres are Pirelli and, with More Tyre
  Suppliers on, its brands from its `brands.json`, showing the brand its own grid tabs give
  a car until one is picked here. The picked brand of each of the team's cars goes in
  `tyres.json` in the Grid Editor's data folder, by car archive, whenever the edits are
  saved and at build: More Tyre Suppliers (0.5 on) builds the brand onto the car from there,
  over its grid tabs.
- **Team names:** full, short and super short. Names are text
  in the game's language files. A name key that other teams or names share gets a key of
  its own, so an edit changes only that team.
- **Logos:** a PNG, JPEG or SVG of any size per place, or one for every place: timing tower
  (also used in standings and results), team selection, starting grid and showroom. The
  picture is fitted into the slot without stretching, at a **Logo size** of 20 to 100 %,
  encoded to BC7 and merged into the texture arrays slice by slice.
- **Team principal:** any character the game has running a team, past or present. One
  whose name the game gave to a later principal, such as Andreas Seidl, gets his own
  name back.
- **UI colours:** the menu colour and the primary, secondary and tertiary colours in the
  game database, set with the launcher's colour picker.
- **Line-ups:** in a team's panel each seat shows its driver and race number;
  **Replace driver** opens the list of drivers: the game's own with a head model, then
  the ones More Driver Models adds. Other addons' drivers only show where they
  already race. A driver races in one seat of a grid: the list starts with the drivers
  on it, by team, and picking one swaps the two seats. Presets and restores follow the
  same rule. More Driver Models' drivers follow its page as it is: the ones switched
  off there leave the lists, without waiting for a Play. A driver another addon gives an
  older head of its own, such as Classic Seasons' `cs_jolyon_palmer`, takes More Driver
  Models' head when he's switched on there, wherever he races: his row is pointed at the
  head its file updates, and where its build makes no head row of his label, one is
  copied from the older row with the model path its roster lists. Last season's champion
  shows as #1, the number the game gives him.
- **Drivers:** on the Driver Profiles page, or **Edit driver** in a team's panel. First
  and last name, initials and nationality go with the driver to every grid. Car number
  and helmet are per season, on a tab for each grid he races on, which says the team he
  drives for there: the number goes on his seat in that grid, the helmet on that
  season's kit preset. Names get keys of their own, so a name another driver shares
  stays his.
- **My Team logo:** choose a PNG, JPEG or SVG and it takes the place of one of the game's
  emblems (the first one every player has, unless you pick another), drawn as it is with
  the emblem's colours turned off. Set its size and whether its background is
  transparent or black, and rename the emblem.
- **Presets:** save a whole grid with **Save grid as preset** at the top of its page, or one team from its page, and put it
  back or on another grid or team. The Presets page lists grid and team presets apart.
- **Export:** write a grid's edits as an addon of its own and install it, with
  **Export grid as addon** at the top of the grid's page, or only one team's from its
  panel.
- **What changed what:** a field you edited shows in purple, one another addon changed in
  amber, and the table's Status column says whether a team is as its grid has it, edited
  by you, or changed by an addon.
- **Portraits:** with More Driver Models on, drivers show its portraits in the table and
  on their page.
- **Restore to default:** per field and per team in the team's panel, or a whole grid.
  It only takes your edits away: the field goes back to what the profile and its addons
  give it.

## How it works

The page reads the game as the active profile will load it: the stock database and
English text, with every other enabled addon's operations played over them in load
order. Rows an addon adds from its own build step come from the profile's last build.
Team mods therefore show with their names, colours and logos, and the page credits
the addon behind each change.

Edits are stored in the addon's data folder and shared by every profile. When the
profile builds, on **Play**, the addon's build hook returns them as `db.set`, `lang.set`
and `erp.merge-slices` operations. The addon builds after the season addons whose grids
it edits, Classic Seasons (`swofty.classic-seasons*`), Progressive Career, the F1 2014
(`msyeett.*`) and F1 2012 (`bydanix.*`) seasons, and the mods the importer made into
addons (`load_after` in `swofty.json`), so its edits win over theirs. Not after every
addon: More Driver Models requires Simplified and so builds after it, and `*` would make
a cycle the launcher refuses to launch. A season addon from another author needs its id
pattern added there.

Team logos are written only when a team has a logo set in the editor; with none, the
build leaves every logo array as the other addons made it. A logo goes into each
archive's own copy of the array, at its team's slice in that copy, with the copy's size
and every other slice kept. A copy of the game's size gets an `erp.merge-slices`. A copy
an earlier addon grew, as Classic Seasons does, or appended to (`erp.append-slices`, as
the F1 2014 season does), gets its picture resource back with the same slice count,
since a slice merge would fall back to the game's shorter array. The editor never
writes a whole array of another size, so the slices other addons added stay theirs.

Needs Swofty Launcher 1.5.25 or newer, for its colour picker and transparent pictures.

## Building

Windows, with Rust and the Visual Studio Build Tools:

```powershell
launcher\build.ps1
```

Linux, or both platforms with cargo-zigbuild: `launcher/build.sh`. The release ships the
Windows helper; add `"linux": "bin/grid-editor-helper"` to `launcher.program` in
`swofty.json` after building the Linux one.

The helper builds against the launcher's `swofty-core`, `swofty-addon` and
`swofty-formats` crates, so it sits in `addons/` of a Swofty Launcher checkout.

## Licence and credits

AGPL-3.0-or-later, as it links the launcher's AGPL crates.

Based on Simplified Suite by Team Simplified -
https://github.com/TeamSimplified/simplified-suite. The team colour encoding, name key
handling and logo placement follow the Simplified Team Editor and Grid Editor.
