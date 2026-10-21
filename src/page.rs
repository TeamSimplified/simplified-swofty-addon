// The Mod Importer page: Team Simplified's header, the import card, the last
// import's report and the mods made into addons from the library.
use std::path::PathBuf;

use swofty_addon::launcher::{AddonInfo, Cx};
use swofty_addon::view::{
    Art, Block, Body, Button, ButtonKind, Card, Columns, Effect, Event, Grid, Hero, Icon, Notice, Pill,
    Style, Tone, View,
};

use crate::importer::{self, Convert, Done, Entry, Made, Record};
use crate::library::Library;
use crate::{ENABLE, IMPORT, Loaded, NoProgress, REMOVE, Result, SHOW, date, err, library, plural, track};

pub const CONVERT: &str = "convert-library";
pub const TEAM: &str = "team-simplified";
pub const SHOW_ADDON: &str = "show-addon:";
pub const REIMPORT: &str = "reimport:";
pub const TOGGLE: &str = "toggle:";
pub const LINK: &str = "link:";
pub const DETAILS: &str = "details";
const ART: &[u8] = include_bytes!("../art/team-simplified.png");
const ART_FILE: &str = "team-simplified.png";
pub const FOOTER: &str = "Based on Simplified Suite by Team Simplified.";
const ABOUT: &str = "Team Simplified make modding tools for F1 25: Simplified Studio, the SERPs Launcher, a track installer, swappers, editors and a database merger, open source in the Simplified Suite.";
const IMPORTER: &str =
    "The Mod Importer is their part of Swofty Launcher: it turns any F1 25 mod into a Swofty addon.";
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
        ..Button::new(IMPORT, "Choose a mod…")
    }
}

fn show_button(addon: &str) -> Button {
    Button {
        icon: Some(Icon::Puzzle),
        ..Button::new(format!("{SHOW_ADDON}{addon}"), "Show in Addons")
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

fn hero(cx: &Cx, converted: usize, open: bool) -> Block {
    Block::Hero(Hero {
        title: "Mod Importer".into(),
        subtitle: Some("By Team Simplified. Turns any F1 25 mod into a Swofty addon.".into()),
        art: art(cx),
        pills: vec![Pill {
            text: format!("{converted} converted"),
            tone: Tone::Muted,
            ..Pill::default()
        }],
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

fn choose(profile: &str) -> Block {
    Block::Card(Card {
        title: Some("Import a mod".into()),
        subtitle: Some(
            "Drop a mod's zip, 7z, RAR or .serps file, or its folder, anywhere on the window, or choose one."
                .into(),
        ),
        buttons: vec![choose_button()],
        blocks: vec![Block::Text {
            text: format!(
                "Each mod becomes a Swofty addon on the Addons page, turned on in {profile}. Mods made with Simplified Studio and track zips import too. Importing a mod again replaces its addon."
            ),
            tone: Tone::Muted,
        }],
        ..Card::default()
    })
}

fn kinds(e: &Entry) -> Vec<Pill> {
    let Some(made) = &e.made else {
        return Vec::new();
    };
    made.category
        .map(|c| Pill {
            text: c.label().into(),
            tone: Tone::Accent,
            ..Pill::default()
        })
        .into_iter()
        .chain(made.kinds.iter().map(|k| Pill {
            text: k.clone(),
            tone: Tone::Muted,
            ..Pill::default()
        }))
        .collect()
}

fn summary(made: &Made) -> String {
    let what = match made.summary.is_empty() {
        true => made.kinds.join(", "),
        false => made.summary.clone(),
    };
    [
        Some(format!("Became the addon {}", made.name)),
        made.category.map(|c| c.label().to_owned()),
        (!what.is_empty()).then_some(what),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

fn last(record: &Record, details: bool) -> Option<Block> {
    let last = record.last.as_ref()?;
    let mut blocks = Vec::new();
    for text in &last.errors {
        blocks.push(Block::Text {
            text: text.clone(),
            tone: Tone::Bad,
        });
    }
    for v in &last.variants {
        blocks.push(Block::Text {
            text: v.line(record),
            tone: Tone::Accent,
        });
    }
    let mut technical = Vec::new();
    let mut first = None;
    for id in &last.mods {
        let Some(e) = record.mods.get(id) else {
            continue;
        };
        if let Some(made) = &e.made {
            let off = last.variants.iter().any(|v| v.on != *id && v.mods.contains(id));
            blocks.push(Block::Text {
                text: match off {
                    true => format!("{} · off", summary(made)),
                    false => summary(made),
                },
                tone: if off { Tone::Muted } else { Tone::Good },
            });
            blocks.extend(made.warnings.iter().map(|w| Block::Text {
                text: format!("{}: {w}", made.name),
                tone: Tone::Warn,
            }));
            if !made.lines.is_empty() {
                technical.push(Block::Names {
                    label: made.name.clone(),
                    items: made.lines.clone(),
                });
            }
            if !off {
                first.get_or_insert(made.id.clone());
            }
        }
        if let Some(error) = &e.error {
            blocks.push(Block::Text {
                text: format!("{} was not converted: {error}", e.name),
                tone: Tone::Bad,
            });
        }
    }
    let mut buttons = Vec::new();
    if !technical.is_empty() {
        buttons.push(Button {
            icon: Some(Icon::Info),
            style: if details { Style::Primary } else { Style::Outline },
            ..Button::new(DETAILS, if details { "Hide details" } else { "Details" })
        });
    }
    buttons.extend(first.map(|id| show_button(&id)));
    if details && !technical.is_empty() {
        blocks.push(Block::Label("WHAT BECAME WHAT".into()));
        blocks.extend(technical);
    }
    if !buttons.is_empty() {
        blocks.push(Block::Buttons(buttons));
    }
    Some(Block::Card(Card {
        title: Some(match last.files.is_empty() {
            true => "Library conversion".into(),
            false => "Last import".into(),
        }),
        subtitle: Some(match last.files.is_empty() {
            true => format!("{} · the mods in your library", date(last.at)),
            false => format!("{} · {}", date(last.at), last.files.join(", ")),
        }),
        blocks,
        ..Card::default()
    }))
}

fn converted(record: &Record, installed: Option<&[AddonInfo]>, editable: bool) -> Option<Block> {
    let cards: Vec<Block> = record
        .mods
        .iter()
        .filter_map(|(id, e)| Some((id, e, e.made.as_ref()?)))
        .map(|(id, e, made)| {
            let present = installed.is_none_or(|list| list.iter().any(|a| a.id == made.id));
            let on = installed.and_then(|list| list.iter().find(|a| a.id == made.id).map(|a| a.enabled));
            let mut pills = kinds(e);
            match (present, on) {
                (false, _) => pills.push(Pill {
                    text: "Removed from Addons".into(),
                    tone: Tone::Warn,
                    ..Pill::default()
                }),
                (true, Some(on)) => pills.push(Pill {
                    text: if on { "On".into() } else { "Off".into() },
                    tone: if on { Tone::Good } else { Tone::Muted },
                    dot: true,
                    ..Pill::default()
                }),
                (true, None) => {}
            }
            let mut buttons = Vec::new();
            if present {
                buttons.push(show_button(&made.id));
            }
            if editable {
                buttons.push(Button {
                    icon: Some(Icon::Refresh),
                    ..Button::new(format!("{REIMPORT}{id}"), "Re-import")
                });
            }
            let mut blocks = vec![Block::Pills(pills)];
            if let Some(error) = &e.error {
                blocks.push(Block::Text {
                    text: format!("The last re-import failed: {error}"),
                    tone: Tone::Warn,
                });
            }
            blocks.push(Block::Buttons(buttons));
            Block::Card(Card {
                title: Some(e.name.clone()),
                subtitle: Some(format!("Converted {}", date(e.at))),
                blocks,
                ..Card::default()
            })
        })
        .collect();
    if cards.is_empty() {
        return None;
    }
    Some(Block::Card(Card {
        title: Some("Converted from your library".into()),
        subtitle: Some("The library keeps each mod as imported, so its addon can be made again.".into()),
        blocks: vec![Block::Grid(Grid {
            columns: Columns::Count(2),
            blocks: cards,
        })],
        ..Card::default()
    }))
}

fn failed(cx: &Cx, record: &Record) -> Result<Option<Block>> {
    let loaded = Loaded::read(cx)?;
    let mut cards = Vec::new();
    for (p, on) in &loaded.order {
        let Some(e) = record.mods.get(&p.id).filter(|e| e.made.is_none()) else {
            continue;
        };
        let mut buttons = vec![
            Button {
                icon: Some(Icon::Refresh),
                ..Button::new(format!("{REIMPORT}{}", p.id), "Convert again")
            },
            Button {
                icon: Some(Icon::FolderOpen),
                ..Button::new(format!("{SHOW}{}", p.id), "Show files")
            },
        ];
        if !cx.profile.vanilla {
            buttons.insert(
                1,
                Button::new(
                    format!("{TOGGLE}{}", p.id),
                    if *on { "Turn off" } else { "Turn on" },
                ),
            );
        }
        buttons.push(Button {
            icon: Some(Icon::Trash),
            style: Style::Danger,
            kind: ButtonKind::Confirm {
                title: format!("Remove {}?", p.name),
                text: "It is deleted from the library and turned off in every profile.".into(),
            },
            ..Button::new(format!("{REMOVE}{}", p.id), "Remove from library")
        });
        cards.push(Block::Card(Card {
            title: Some(p.name.clone()),
            subtitle: Some(format!(
                "{} in {} · loads through the Mod Importer's build step",
                if *on { "On" } else { "Off" },
                cx.profile.name
            )),
            blocks: vec![
                Block::Text {
                    text: e.error.clone().unwrap_or_default(),
                    tone: Tone::Bad,
                },
                Block::Buttons(buttons),
            ],
            ..Card::default()
        }));
    }
    if cards.is_empty() {
        return Ok(None);
    }
    Ok(Some(Block::Card(Card {
        title: Some("Not converted".into()),
        subtitle: Some(
            "These mods could not be made into addons. They load as before, in the library's order.".into(),
        ),
        blocks: cards,
        ..Card::default()
    })))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Panels {
    pub team: bool,
    pub details: bool,
}

pub fn view(cx: &Cx, open: bool, details: bool) -> Result<View> {
    let record = Record::load(cx.data_dir)?;
    let pending = importer::pending(cx, &record)?;
    let installed = match cx.addons() {
        Some(addons) => Some(addons.list()?),
        None => None,
    };
    let count = record.mods.values().filter(|e| e.made.is_some()).count();
    let vanilla = cx.profile.vanilla;
    let mut blocks = vec![hero(cx, count, open)];
    if !vanilla {
        blocks.push(choose(&cx.profile.name));
    }
    if open {
        blocks.push(team());
    }
    blocks.extend(last(&record, details));
    if !vanilla {
        blocks.extend(failed(cx, &record)?);
    }
    blocks.extend(converted(&record, installed.as_deref(), !vanilla));
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

fn files_dir(library: &Library, id: &str) -> PathBuf {
    let dir = library.dir(id);
    match dir.join(track::TRACK).is_dir() {
        true => dir.join(track::TRACK),
        false => dir.join(library::FILES),
    }
}

fn shown(done: Done) -> Result<Effect> {
    if done.failed {
        return Err(err(done.lines.join("\n")));
    }
    Ok(match done.show.filter(|_| !done.variants) {
        Some(id) => Effect::ShowAddon(id),
        None => Effect::Notice(done.lines.join("\n")),
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
        Event::Click { id } if id == DETAILS => {
            panels.details = !panels.details;
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
        Event::Click { id } if id.starts_with(REIMPORT) => {
            shown(importer::reimport(cx, &id[REIMPORT.len()..], conv)?)?
        }
        Event::Click { id } if id.starts_with(TOGGLE) => {
            let id = &id[TOGGLE.len()..];
            let mut loaded = Loaded::read(cx)?;
            let on = !loaded.order[loaded.find(id)?].1;
            loaded.set(id, on)?;
            loaded.save(cx)?;
            Effect::Reload
        }
        Event::Click { id } if id.starts_with(LINK) => {
            let url = LINKS
                .iter()
                .find(|(l, _, _)| *l == &id[LINK.len()..])
                .and_then(|(_, _, url)| *url)
                .ok_or_else(|| err(format!("no link {id}")))?;
            Effect::Open(PathBuf::from(url))
        }
        Event::Click { id } if id.starts_with(SHOW) => {
            Effect::Open(files_dir(&Library::new(cx.data_dir), &id[SHOW.len()..]))
        }
        Event::Click { id } if id.starts_with(REMOVE) => {
            let p = crate::remove(cx, &id[REMOVE.len()..])?;
            Effect::Notice(format!("Removed {} from the library", p.name))
        }
        _ => return Ok(None),
    };
    Ok(Some(effect))
}
