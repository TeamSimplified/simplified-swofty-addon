// The Simplified Importer page: Team Simplified's header and the import card. Each
// imported mod becomes an addon, listed on Addons.
use std::path::PathBuf;

use swofty_addon::launcher::Cx;
use swofty_addon::view::{
    Art, Block, Body, Button, ButtonKind, ButtonSize, Card, DropZone, Effect, Event, Hero, Icon, Notice,
    Shown, Style, Tone, View,
};

use crate::importer::{self, Convert, Done, Record};
use crate::{ENABLE, IMPORT, NoProgress, Result, err, plural};

pub const CONVERT: &str = "convert-library";
pub const TEAM: &str = "team-simplified";
pub const SHOW_ADDON: &str = "show-addon:";
pub const LINK: &str = "link:";
// The sidebar's mark (sections/mods.svg), drawn as the sidebar draws it.
const ART: &[u8] = include_bytes!("../art/simplified-mark.png");
const ART_FILE: &str = "simplified-mark.png";
pub const FOOTER: &str = "Based on Simplified Suite by Team Simplified.";
const ABOUT: &str = "Team Simplified make modding tools for F1 25: Simplified Studio, the SERPs Launcher, a track installer, swappers, editors and a database merger, open source in the Simplified Suite.";
const IMPORTER: &str = "Simplified for Swofty is their part of Swofty Launcher: its Simplified Importer turns any F1 25 mod into a Swofty addon.";
const FORMAT: [&str; 2] = [
    "A Swofty addon is a folder or .swoftyaddon file with a swofty.json manifest and the changes it makes: database records, text, archive resources, files and tracks.",
    "The launcher applies every addon that is on in the profile, in load order, and lower in the list wins.",
];

// Links shown in the Team Simplified panel. None marks a placeholder for a link
// Team Simplified has not confirmed yet; it is not shown.
const LINKS: [(&str, &str, Option<&str>); 5] = [
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
    (
        "serps-launcher",
        "SERPs Launcher",
        Some("https://github.com/TeamSimplified/SERPs-Launcher"),
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

pub fn choose_button() -> Button {
    Button {
        icon: Some(Icon::Upload),
        style: Style::Primary,
        kind: ButtonKind::Pick {
            title: "Choose a mod".into(),
            extensions: vec!["zip".into(), "7z".into(), "rar".into(), "serps".into()],
            folders: true,
            multiple: true,
        },
        size: ButtonSize::Large,
        ..Button::new(IMPORT, "Choose a mod…")
    }
}

fn notices(cx: &Cx, pending: &importer::Pending) -> Vec<Notice> {
    let mut out = crate::notices(cx);
    if cx.addons().is_none() {
        out.push(Notice {
            text: importer::UPDATE.into(),
            tone: Tone::Warn,
            action: None,
        });
        return out;
    }
    if cx.game.is_none() && pending.convert > 0 {
        out.push(Notice {
            text: format!(
                "{} in your library {} not addons yet. {}",
                plural(pending.convert, "mod", "mods"),
                if pending.convert == 1 { "is" } else { "are" },
                importer::NO_GAME
            ),
            tone: Tone::Warn,
            action: None,
        });
    }
    let text = match (pending.convert, pending.place, cx.game.is_some()) {
        (n, _, true) if n > 0 => format!(
            "{} in your library {} not addons yet. They convert on the next start, or now: each becomes an addon, on where the mod was on.",
            plural(n, "mod", "mods"),
            if n == 1 { "is" } else { "are" }
        ),
        (_, n, _) if n > 0 => format!(
            "{} in {} {} addons now. Turn them on in the profile's mod order.",
            plural(n, "mod", "mods"),
            cx.profile.name,
            if n == 1 { "has" } else { "have" }
        ),
        _ => return out,
    };
    out.push(Notice {
        text,
        tone: Tone::Accent,
        action: Some(Button {
            icon: Some(Icon::Refresh),
            style: Style::Primary,
            ..Button::new(CONVERT, "Convert now")
        }),
    });
    out
}

fn hero(cx: &Cx, open: bool) -> Block {
    Block::Hero(Hero {
        title: "Simplified Importer".into(),
        subtitle: Some(
            "By Team Simplified. Mods convert into Swofty addons using the Simplified Importer.".into(),
        ),
        art: art(cx),
        pills: Vec::new(),
        facts: Vec::new(),
        buttons: vec![Button {
            icon: Some(Icon::Info),
            style: if open { Style::Primary } else { Style::Outline },
            ..Button::new(TEAM, "Team Simplified")
        }],
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
        Block::Label("THE SWOFTY ADDON FORMAT".into()),
        Block::Text {
            text: FORMAT.join(" "),
            tone: Tone::Muted,
        },
        Block::Label("LINKS".into()),
    ];
    let links: Vec<(&str, &str)> = LINKS
        .iter()
        .filter(|(_, _, url)| url.is_some())
        .map(|(id, label, _)| (*id, *label))
        .collect();
    blocks.push(Block::Buttons(
        links
            .iter()
            .map(|(id, label)| Button::new(format!("{LINK}{id}"), *label))
            .collect(),
    ));
    Block::Card(Card {
        title: Some("Team Simplified".into()),
        subtitle: Some("Modding tools for F1 25".into()),
        blocks,
        ..Card::default()
    })
}

fn choose() -> Block {
    Block::DropZone(DropZone {
        title: "Import a mod".into(),
        text: "Drop a mod's zip, 7z, RAR or .serps file, or its folder, anywhere on the window.".into(),
        button: choose_button(),
        icon: Some(Icon::Upload),
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Panels {
    pub team: bool,
}

pub fn view(cx: &Cx, open: bool) -> Result<View> {
    let record = Record::load(cx.data_dir)?;
    let pending = importer::pending(cx, &record)?;
    let count = record.mods.values().filter(|e| e.made.is_some()).count();
    let vanilla = cx.profile.vanilla;
    let mut blocks = vec![hero(cx, open)];
    if open {
        blocks.push(team());
    }
    if !vanilla {
        blocks.push(choose());
    }
    blocks.push(Block::Text {
        text: FOOTER.into(),
        tone: Tone::Muted,
    });
    let notices = match vanilla {
        true => crate::notices(cx),
        false => notices(cx, &pending),
    };
    Ok(View {
        subtitle: format!("{} converted into addons", plural(count, "mod", "mods")),
        actions: Vec::new(),
        notices,
        body: Body::Blocks(blocks),
        footer: None,
        tick: false,
    })
}

fn shown(done: Done) -> Result<Effect> {
    if done.failed {
        return Err(err(done.lines.join("\n")));
    }
    Ok(match (done.show, done.variants) {
        (Some(id), true) => Effect::ShowAddon(Shown::new(id, done.lines.join("\n"))),
        (Some(id), false) => Effect::ShowAddon(id.into()),
        (None, _) => Effect::Notice(done.lines.join("\n")),
    })
}

pub fn event(cx: &Cx, event: Event, conv: &dyn Convert, panels: &mut Panels) -> Result<Option<Effect>> {
    let effect = match event {
        Event::Startup {} => match importer::startup(cx, conv, &NoProgress, importer::BUDGET)? {
            Some(text) => Effect::Notice(text),
            None => Effect::None,
        },
        Event::Picked { id, paths } if id == IMPORT => {
            shown(importer::import(cx, &paths, &NoProgress, conv)?)?
        }
        Event::Click { id } if id == TEAM => {
            panels.team = !panels.team;
            Effect::Reload
        }
        Event::Click { id } if id == CONVERT => {
            let lines = importer::migrate(cx, conv, &NoProgress)?;
            match lines.is_empty() {
                true => Effect::Reload,
                false => Effect::Notice(lines.join("\n")),
            }
        }
        Event::Click { id } if id == ENABLE => Effect::SetAddonEnabled(true),
        Event::Click { id } if id.starts_with(SHOW_ADDON) => Effect::ShowAddon(id[SHOW_ADDON.len()..].into()),
        Event::Click { id } if id.starts_with(LINK) => {
            let url = LINKS
                .iter()
                .find(|(l, _, _)| *l == &id[LINK.len()..])
                .and_then(|(_, _, url)| *url)
                .ok_or_else(|| err(format!("no link {id}")))?;
            Effect::Open(PathBuf::from(url))
        }
        _ => return Ok(None),
    };
    Ok(Some(effect))
}
