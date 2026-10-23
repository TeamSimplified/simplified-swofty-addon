// The About page: Team Simplified's header and the panel about Team Simplified, with
// links to their projects.
use std::path::PathBuf;

use swofty_addon::launcher::Cx;
use swofty_addon::view::{Art, Block, Body, Button, Card, Effect, Event, Hero, Tone, View};

use crate::page::FOOTER;
use crate::{Result, err};

pub const SECTION: &str = "about";
pub const LINK: &str = "link:";
// The sidebar's mark (sections/about.svg), drawn as the sidebar draws it.
const ART: &[u8] = include_bytes!("../art/simplified-mark.png");
const ART_FILE: &str = "simplified-mark.png";
const ABOUT: &str = "Team Simplified make modding tools for F1 25: Simplified Studio, the SERPs Launcher, a track installer, swappers, editors and a database merger, open source in the Simplified Suite.";
const IMPORTER: &str = "Simplified for Swofty is their part of Swofty Launcher: its Simplified Importer turns any F1 25 mod into a Swofty addon.";

// Links shown in the Team Simplified panel. None marks a placeholder for a link
// Team Simplified has not confirmed yet; it is not shown.
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

fn hero(cx: &Cx) -> Block {
    Block::Hero(Hero {
        title: "Simplified for Swofty".into(),
        subtitle: Some("By Team Simplified.".into()),
        art: art(cx),
        pills: Vec::new(),
        facts: Vec::new(),
        buttons: Vec::new(),
    })
}

fn team() -> Block {
    let mut blocks = vec![
        Block::Text {
            text: ABOUT.into(),
            tone: Tone::Neutral,
        },
        Block::Text {
            text: IMPORTER.into(),
            tone: Tone::Neutral,
        },
        Block::Label("LINKS".into()),
    ];
    blocks.push(Block::Buttons(
        LINKS
            .iter()
            .filter(|(_, _, url)| url.is_some())
            .map(|(id, label, _)| Button::new(format!("{LINK}{id}"), *label))
            .collect(),
    ));
    Block::Card(Card {
        title: Some("Team Simplified".into()),
        subtitle: Some("Modding tools for F1 25".into()),
        blocks,
        ..Card::default()
    })
}

pub fn view(cx: &Cx) -> Result<View> {
    Ok(View {
        subtitle: "Team Simplified's addon for Swofty Launcher.".into(),
        actions: Vec::new(),
        notices: Vec::new(),
        body: Body::Blocks(vec![
            hero(cx),
            team(),
            Block::Text {
                text: FOOTER.into(),
                tone: Tone::Muted,
            },
        ]),
        footer: None,
        tick: false,
    })
}

pub fn event(event: Event) -> Result<Effect> {
    match event {
        Event::Click { id } if id.starts_with(LINK) => {
            let url = LINKS
                .iter()
                .find(|(l, _, _)| *l == &id[LINK.len()..])
                .and_then(|(_, _, url)| *url)
                .ok_or_else(|| err(format!("no link {id}")))?;
            Ok(Effect::Open(PathBuf::from(url)))
        }
        _ => Ok(Effect::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_opens_its_page_and_a_placeholder_is_not_shown() {
        let open = |id: &str| {
            event(Event::Click {
                id: format!("{LINK}{id}"),
            })
        };
        assert_eq!(
            open("github").unwrap(),
            Effect::Open(PathBuf::from("https://github.com/TeamSimplified"))
        );
        assert!(open("discord").is_err());
        let Block::Card(card) = team() else {
            panic!("the team panel is a card")
        };
        let Some(Block::Buttons(links)) = card.blocks.last() else {
            panic!("links last")
        };
        assert_eq!(links.len(), 2);
    }
}
