// The Simplified Importer page: the import card. Each imported mod becomes an addon,
// listed on Addons.
use swofty_addon::launcher::Cx;
use swofty_addon::view::{
    Block, Body, Button, ButtonKind, ButtonSize, DropZone, Effect, Event, Icon, Notice, Shown, Style, Tone,
    View,
};

use crate::importer::{self, Convert, Done, Record};
use crate::{ENABLE, IMPORT, NoProgress, Result, err, plural};

pub const CONVERT: &str = "convert-library";
pub const SHOW_ADDON: &str = "show-addon:";
const SUBTITLE: &str = "Adapt your copy and paste mods to work with the Swofty Launcher.";
pub const FOOTER: &str = "Based on the Simplified Suite by Team Simplified.";
pub fn choose_button() -> Button {
    Button {
        icon: Some(Icon::Upload),
        style: Style::Primary,
        kind: ButtonKind::Pick {
            title: "Choose a mod".into(),
            extensions: vec!["zip".into(), "7z".into(), "rar".into()],
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

fn choose() -> Block {
    Block::DropZone(DropZone {
        title: "Import a mod".into(),
        text: "Drop a mod's zip, 7z or RAR file, or its folder, anywhere on the window.".into(),
        button: choose_button(),
        icon: Some(Icon::Upload),
    })
}

pub fn view(cx: &Cx) -> Result<View> {
    let record = Record::load(cx.data_dir)?;
    let pending = importer::pending(cx, &record)?;
    let vanilla = cx.profile.vanilla;
    let mut blocks = Vec::new();
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
        subtitle: SUBTITLE.into(),
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

pub fn event(cx: &Cx, event: Event, conv: &dyn Convert) -> Result<Option<Effect>> {
    let effect = match event {
        Event::Startup {} => match importer::startup(cx, conv, &NoProgress, importer::BUDGET)? {
            Some(text) => Effect::Notice(text),
            None => Effect::None,
        },
        Event::Picked { id, paths } if id == IMPORT => {
            shown(importer::import(cx, &paths, &NoProgress, conv)?)?
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
        _ => return Ok(None),
    };
    Ok(Some(effect))
}
