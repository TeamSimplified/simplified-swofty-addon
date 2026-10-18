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

The launcher builds this crate into `swofty-builtins`. It depends on two crates of the launcher, `swofty-addon` and `swofty-formats`, which this repository takes from a SwoftyLauncher release tag, or from a commit on its `main` when a change it needs is not released yet. In the launcher's root `Cargo.toml`, patch them to the launcher's own copies, so there is one of each:

```toml
[workspace.dependencies]
swofty-addon-simplified = { git = "https://github.com/TeamSimplified/simplified-swofty-addon", branch = "main" }

[patch."https://github.com/Swofty-Developments/SwoftyLauncher"]
swofty-addon = { path = "packages/addon-api" }
swofty-formats = { path = "packages/formats" }
```

Then remove `addons/simplified` from `members` and `default-members`, and delete the folder. `addons/builtins` keeps `swofty-addon-simplified.workspace = true` and `swofty_addon_simplified::builtin()` as they are.

Pin a tag or `rev` instead of `branch` for releases. To work on both repositories at once, point the dependency at a checkout instead: `{ path = "../simplified-swofty-addon" }`.

## Building on its own

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The toolchain is pinned in `rust-toolchain.toml`, to the same version as the launcher. The tests need no game files. When the launcher changes `swofty-addon` or `swofty-formats`, point the `tag` (or `rev`) in `Cargo.toml` at the release or commit that has the change.

## Licence

[GNU Affero General Public License, version 3 or later](LICENSE), like Swofty Launcher. Parts are ported from the Simplified Suite under the GNU General Public License version 3; see [THIRD_PARTY.md](THIRD_PARTY.md).

Based on Simplified Suite by Team Simplified - https://github.com/TeamSimplified/simplified-suite
