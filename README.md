# Simplified for Swofty

**Simplified for Swofty** is Team Simplified's addon for [Swofty Launcher for F1 25](https://github.com/Swofty-Developments/SwoftyLauncher). Its **Simplified Importer** turns any F1 25 mod into a standalone Swofty addon. The importer shows as **Simplified Importer** under **Simplified for Swofty** in the launcher's sidebar and as `swofty mods` on the command line.

- Works with any mod: drop its zip, 7z, rar or folder on the launcher, or choose it on the Simplified Importer page. The game folders can sit at the top or up to three folders down, or in a folder named like `F1 25`. Mods made with Simplified Studio (`.serps`) and community track zips import too.
- Each mod becomes a Swofty data addon on the **Addons** page, with its own switch, place in the load order and conflicts. Lower in the list wins.
- The addon holds what the mod changes from the game's own files: database records, text, archive resources, files and tracks.
- After an import the addon is on in the active profile, and the launcher switches to **Addons** with the addon highlighted.
- A mod whose parts are alternatives, such as tyre sets for different seasons, installs every part as an addon with one on. Pick another on **Addons**.
- Mods made with Simplified Studio and My Team packages load with Team Simplified's SERPs Base Files, which come with the addon.
- Nothing is kept besides the addon: the mod is unpacked only while it converts. To make an addon again, import the mod again; it keeps the same id.

How it works is in [docs/design.md](docs/design.md).

## The Simplified Importer page

- **Choose a mod…** picks zips, 7z and rar archives and folders, several at once. A drop anywhere on the window does the same.
- After an import, the launcher shows the new addon on **Addons**, or a notice with what went wrong. A mod that can't be made into an addon isn't kept.

## Mods imported before 0.7.0

The library converts into addons once, when the launcher starts, on the first import or on **Convert now** on the page, when the launcher has found the game. Each profile then turns the addons on in its own mod order, on where the mod was on, and puts them where the Simplified Importer loaded its mods in the load order. A profile that hasn't been handed over yet loads its mods through the build step, as before. Each mod's library copy is deleted once it is an addon. Mods that fail to convert stay in the library and keep loading through the build step. Track packages convert into track addons; the Tracks page is gone.

## Using it in Swofty Launcher

Simplified for Swofty is a launcher plugin: a helper program (`bin/simplified-helper`) that the launcher runs, packaged with its `swofty.json` as `teamsimplified.mods.swoftyaddon`. Each [release](https://github.com/TeamSimplified/simplified-swofty-addon/releases) attaches it, built for Windows and Linux, with its `SHA256SUMS` and its `swofty.json`.

Swofty Launcher 1.0.4 and later installs the addon itself, checks it against `SHA256SUMS`, and keeps it up to date. Launchers from 1.3.1 install the newest release whose `swofty.json` accepts them. Earlier launchers install the release GitHub marks as latest, which stays at one every launcher from 1.0.4 can install; new releases are published without that mark. To install it by hand, drop the `.swoftyaddon` on the launcher, or run:

```sh
swofty addons install teamsimplified.mods.swoftyaddon
swofty addons enable teamsimplified.mods
```

Its id is `teamsimplified.mods`, the same id the launcher's built-in copy used, so the mod library and each profile's mod list carry over. A launcher that still builds that copy in refuses to install this one ("teamsimplified.mods is built in").

The helper speaks the launcher's helper protocol through `swofty_addon::helper::run_helper`. Version 0.7.0 needs Swofty Launcher 1.3.0 or later: it installs, turns on and orders addons with the addon host calls (`install_addon`, `set_addon_enabled`, `addons`, `move_addon`) and asks the launcher to show an addon with the `show_addon` effect. The helper decides which dropped files and folders it imports, checking them by their contents.

`sections/mods.svg` gives the Simplified Importer page its sidebar icon, Team Simplified's mark.

The one-time move of tracks from the launcher's old `maps/` folder only runs in the built-in copy.

The crate still builds as a library, so the launcher can also build it in through `swofty-builtins`, as it did before: depend on it from the launcher's root `Cargo.toml` and patch the launcher crates it uses to the launcher's own copies.

```toml
[workspace.dependencies]
swofty-addon-simplified = { git = "https://github.com/TeamSimplified/simplified-swofty-addon", tag = "v0.5.1" }

[patch."https://github.com/Swofty-Developments/SwoftyLauncher"]
swofty-addon = { path = "packages/addon-api" }
swofty-formats = { path = "packages/formats" }
```

## Building on its own

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

To build the helper, run `cargo build --release --bin simplified-helper`. Pushing a `v*` tag builds it for Windows and Linux and attaches the `.swoftyaddon` to that release (`.github/workflows/release.yml`); the version in `swofty.json` must match the tag.

The toolchain is pinned in `rust-toolchain.toml`, to the same version as the launcher. The tests need no game files. When the launcher changes `swofty-addon` or `swofty-formats`, point the `tag` (or `rev`) in `Cargo.toml` at the release or commit that has the change.

## Licence

[GNU Affero General Public License, version 3 or later](LICENSE), like Swofty Launcher. Parts are ported from the Simplified Suite under the GNU General Public License version 3; see [THIRD_PARTY.md](THIRD_PARTY.md).

Based on Simplified Suite by Team Simplified - https://github.com/TeamSimplified/simplified-suite
