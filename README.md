# Simplified for Swofty

Simplified for Swofty is Team Simplified's **Mod Importer** for [Swofty Launcher for F1 25](https://github.com/Swofty-Developments/SwoftyLauncher). It turns any F1 25 mod into a standalone Swofty addon. It shows as **Mod Importer** in the launcher's sidebar and as `swofty mods` on the command line.

- Works with any mod: drop its zip, 7z, rar, `.serps` or folder on the launcher, or choose it on the Mod Importer page. The game folders can sit at the top or up to three folders down, or in a folder named like `F1 25`. Mods made with Simplified Studio (`.serps`) and community track zips import too.
- Each mod becomes a Swofty data addon on the **Addons** page, with its own switch, place in the load order and conflicts. Lower in the list wins.
- The addon holds what the mod changes from the game's own files: database records, text, archive resources, files and tracks.
- After an import the addon is on in the active profile, and the launcher switches to **Addons** with the addon highlighted.
- The library keeps every mod as imported. **Re-import** makes its addon again, under the same id.

How it works is in [docs/design.md](docs/design.md).

## The Mod Importer page

- Team Simplified's logo and a short about. The **Team Simplified** button opens a panel about Team Simplified and the Swofty addon format, with links to their GitHub, the Simplified Suite and the SERPs Launcher.
- **Choose a mod…** picks zips, 7z and rar archives, `.serps` files and folders, several at once. A drop anywhere on the window does the same.
- **Last import** lists each mod of the last import: the addon it became, its category and kinds, what became what, and warnings.
- **Not converted** lists the mods that could not be made into addons, with the reason. They load through the addon's build step, as mods did before 0.7.0, and each has **Convert again**, **Turn on** or **Turn off**, **Show files** and **Remove from library**.
- **Converted from your library** lists each mod with its addon, **Show in Addons** and **Re-import**.

## Mods imported before 0.7.0

The library converts into addons once, on the first import or on **Convert now** on the page, when the launcher has found the game. Each profile then turns the addons on in its own mod order, on where the mod was on, and puts them where Simplified for Swofty loaded its mods in the load order. A profile that hasn't been handed over yet loads its mods through the build step, as before. Mods that fail to convert stay in the library and keep loading through the build step. The library is never deleted. Track packages convert into track addons; the Tracks page is gone.

## Using it in Swofty Launcher

Simplified for Swofty is a launcher plugin: a helper program (`bin/simplified-helper`) that the launcher runs, packaged with its `swofty.json` as `teamsimplified.mods.swoftyaddon`. Each [release](https://github.com/TeamSimplified/simplified-swofty-addon/releases) attaches it, built for Windows and Linux, with its `SHA256SUMS`.

Swofty Launcher 1.0.4 and later installs the addon itself from the latest release, checks it against `SHA256SUMS`, and keeps it up to date. To install it by hand, drop the `.swoftyaddon` on the launcher, or run:

```sh
swofty addons install teamsimplified.mods.swoftyaddon
swofty addons enable teamsimplified.mods
```

Its id is `teamsimplified.mods`, the same id the launcher's built-in copy used, so the mod library and each profile's mod list carry over. A launcher that still builds that copy in refuses to install this one ("teamsimplified.mods is built in").

The helper speaks the launcher's helper protocol through `swofty_addon::helper::run_helper`. Version 0.7.0 needs Swofty Launcher 1.3.0 or later: it installs, turns on and orders addons with the addon host calls (`install_addon`, `set_addon_enabled`, `addons`, `move_addon`) and asks the launcher to show an addon with the `show_addon` effect. The helper decides which dropped files and folders it imports, checking them by their contents.

`sections/mods.svg` gives the Mod Importer its sidebar icon, Team Simplified's mark.

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
