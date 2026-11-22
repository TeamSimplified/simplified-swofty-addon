// The about section at the bottom of the Simplified Importer page: Team Simplified's mark,
// the addon's version, a short about Team Simplified and links to their projects.
use std::path::PathBuf;

use swofty_addon::launcher::Cx;
use swofty_addon::view::{Art, Block, Button, Effect, Fact, Hero};

use crate::{Result, err};

pub const LINK: &str = "link:";
pub const TITLE: &str = "Simplified for Swofty";
// The sidebar's mark (sections/mods.svg), drawn as the sidebar draws it.
const ART: &[u8] = include_bytes!("../art/simplified-mark.png");
const ART_FILE: &str = "simplified-mark.png";
const ABOUT: &str = "By Team Simplified, who make modding tools for F1 25: Simplified Studio, the SERPs Launcher, a track installer, swappers, editors and a database merger, open source in the Simplified Suite.";

// Links shown in the about section. None marks a placeholder for a link Team Simplified
// has not confirmed yet; it is not shown.
const LINKS: [(&str, &str, Option<&str>); 4] = [
    (
        "github",
        "Team Simplified on GitHub",
        Some("https://github.com/TeamSimplified"),
    ),
    (
        "suite",
        "Simplified Suite",
        Some("https://github.com/TeamSimplified/simplified-suite"),
    ),
    ("website", "Website", None),
    ("discord", "Discord", None),
];

fn art(cx: &Cx) -> Art {
    let path = cx.cache_dir.join(ART_FILE);
    let fresh = std::fs::metadata(&path).is_ok_and(|m| m.len() == ART.len() as u64);
    let written = fresh
        || std::fs::create_dir_all(cx.cache_dir)
            .and_then(|()| std::fs::write(&path, ART))
            .is_ok();
    match written {
        true => Art::Image(path),
        false => Art::Monogram("TS".into()),
    }
}

// The addon's version, as swofty.json gives it.
fn facts() -> Vec<Fact> {
    vec![Fact::new("Version", env!("CARGO_PKG_VERSION"))]
}

fn links() -> Vec<Button> {
    LINKS
        .iter()
        .filter(|(_, _, url)| url.is_some())
        .map(|(id, label, _)| Button::new(format!("{LINK}{id}"), *label))
        .collect()
}

fn hero(art: Art) -> Block {
    Block::Hero(Hero {
        title: TITLE.into(),
        subtitle: Some(ABOUT.into()),
        art,
        pills: Vec::new(),
        facts: facts(),
        buttons: links(),
    })
}

pub fn block(cx: &Cx) -> Block {
    hero(art(cx))
}

pub fn open(id: &str) -> Result<Effect> {
    let url = id
        .strip_prefix(LINK)
        .and_then(|link| LINKS.iter().find(|(l, _, _)| *l == link))
        .and_then(|(_, _, url)| *url)
        .ok_or_else(|| err(format!("no link {id}")))?;
    Ok(Effect::Open(PathBuf::from(url)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_opens_its_page_and_a_placeholder_is_not_shown() {
        let open = |id: &str| open(&format!("{LINK}{id}"));
        assert_eq!(
            open("github").unwrap(),
            Effect::Open(PathBuf::from("https://github.com/TeamSimplified"))
        );
        assert_eq!(
            open("suite").unwrap(),
            Effect::Open(PathBuf::from(
                "https://github.com/TeamSimplified/simplified-suite"
            ))
        );
        assert!(open("discord").is_err());
        let Block::Hero(hero) = hero(Art::Monogram("TS".into())) else {
            panic!("the about section is a hero")
        };
        let labels: Vec<&str> = hero.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["Team Simplified on GitHub", "Simplified Suite"]);
    }

    #[test]
    fn the_about_section_shows_the_addon_version() {
        let manifest: serde_json::Value = serde_json::from_str(crate::MANIFEST).unwrap();
        assert_eq!(
            facts(),
            [Fact::new("Version", manifest["version"].as_str().unwrap())]
        );
    }
}
