# Simplified for Swofty

Simplified for Swofty is the built-in addon, made by Team Simplified, that loads F1 25 mods in [Swofty Launcher for F1 25](https://github.com/Swofty-Developments/SwoftyLauncher). It shows as **Mods** in the launcher and as `swofty mods` on the command line.

- Works with any mod: drop its zip, 7z, rar or folder on the launcher, and its files go where copying them into the game folder would put them. The game folders can sit at the top or up to three folders down, or in a folder named like `F1 25`. Mods made with Simplified Studio (`.serps`) and community track zips import too.
- Keeps one library for every profile, and each profile's mods in load order. Lower in the list wins.
- Compares each mod with the game's own files and merges only what it changes: database records, text, archive resources and files. Several mods load together, and the Mods page shows where they overlap.
- Loads mods that Simplified Studio packed into the shader archives. Their resources load at boot, as they do with the Simplified Launcher.
- Merges team logos from every mod slice by slice into the game's logo texture arrays, including arrays built for an older game.
- Keeps team names a mod fills in from the game from undoing another mod's rename.

How it works is in [docs/design.md](docs/design.md).

## Using it in Swofty Launcher

Simplified for Swofty is a launcher plugin: a helper program (`bin/simplified-helper`) that the launcher runs, packaged with its `swofty.json` as `teamsimplified.mods.swoftyaddon`. Each [release](https://github.com/TeamSimplified/simplified-swofty-addon/releases) attaches it, built for Windows and Linux, with its `SHA256SUMS`.

To install it, drop the `.swoftyaddon` on the launcher, or run:

```sh
swofty addons install teamsimplified.mods.swoftyaddon
swofty addons enable teamsimplified.mods
```

Its id is `teamsimplified.mods`, the same id the launcher's built-in copy used, so the mod library and each profile's mod list carry over. A launcher that still builds that copy in refuses to install this one ("teamsimplified.mods is built in").

The helper speaks the launcher's helper protocol through `swofty_addon::helper::run_helper`. Until a launcher release includes [SwoftyLauncher#124](https://github.com/Swofty-Developments/SwoftyLauncher/pull/124), the plugin has three limits:

- All mods share one runtime overlay archive. Conflicts still list each mod's changes, but name only the addon.
- Only dropped files are claimed, by extension (`serps`, `zip`, `7z`, `rar`). To import a folder, use **Import** on the Mods page or `swofty mods import`.
- The one-time move of tracks from the launcher's old `maps/` folder only runs in the built-in copy.

The crate still builds as a library, so the launcher can also build it in through `swofty-builtins`, as it did before: depend on it from the launcher's root `Cargo.toml` and patch the launcher crates it uses to the launcher's own copies.

```toml
[workspace.dependencies]
swofty-addon-simplified = { git = "https://github.com/TeamSimplified/simplified-swofty-addon", tag = "v0.5.0" }

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
