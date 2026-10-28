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
  enabled addon adds, such as the Classic Seasons years. F1 2025 includes the optional
  Konnersport and APXGP teams; renaming them renames their menu entries too.
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
  off there leave the lists, without waiting for a Play. Last season's champion shows as #1, the number the
  game gives him.
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
and `erp.merge-slices` operations. The addon loads after every other addon, so its edits
win. An array that an earlier addon grew, as Classic Seasons does, gets the whole grown
array with the edited slices in it, since a slice merge would fall back to the game's
shorter array.

Addons can also leave one logo array differing between archives: Classic Seasons grows
the arrays with its teams, while a team mod merges its logo into a copy of the game's
size in `vfx_package/common.erp`, which the game then shows, so every Classic Seasons
team reads past its end and gets the last logo. On every Play the editor joins those
copies into one, matching teams by name, and writes it to each archive whose copy
differs, so both the added teams and the team mods keep their logos.

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
