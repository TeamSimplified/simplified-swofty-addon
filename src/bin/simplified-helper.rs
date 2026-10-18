// The launcher runs this addon as a helper program (swofty.json's launcher block).
// `simplified-helper claims <path>` answers whether a dropped file or folder is a
// mod or a track, as yes, maybe or no, for launchers that ask before importing.
use swofty_addon::launcher::Claim;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let addon = swofty_addon_simplified::builtin().addon;
    if let [verb, path] = args.as_slice()
        && verb == "claims"
    {
        let claim = addon
            .importer()
            .map_or(Claim::No, |i| i.claims(std::path::Path::new(path)));
        let word = match claim {
            Claim::Yes => "yes",
            Claim::Maybe => "maybe",
            Claim::No => "no",
        };
        println!("{word}");
        return;
    }
    swofty_addon::helper::run_helper(addon);
}
