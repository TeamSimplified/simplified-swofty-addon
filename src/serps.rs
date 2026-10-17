// Ported from ModManifest.cs of the Simplified Launcher.
// Based on Simplified Suite by Team Simplified - https://github.com/TeamSimplified/simplified-suite
// Copyright (C) 2026 Team Simplified. GPL-3.0; see THIRD_PARTY.md.

use serde::{Deserialize, Serialize};

pub const FILE: &str = "mod.serps";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModInfo {
    #[serde(default, alias = "Mod_Name", alias = "ModName", alias = "modName")]
    pub mod_name: Option<String>,
    #[serde(default, alias = "Name")]
    pub name: Option<String>,
    #[serde(default, alias = "Author")]
    pub author: Option<String>,
    #[serde(default, alias = "Version")]
    pub version: Option<String>,
    #[serde(default, alias = "Game")]
    pub game: Option<String>,
    #[serde(default, alias = "Game_Version", alias = "GameVersion", alias = "gameVersion")]
    pub game_version: Option<String>,
    #[serde(
        default,
        alias = "Studio_Version",
        alias = "StudioVersion",
        alias = "studioVersion"
    )]
    pub studio_version: Option<String>,
}

impl ModInfo {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let text = std::str::from_utf8(bytes).ok()?;
        serde_json::from_str(&relax(text)).ok()
    }

    pub fn title(&self) -> Option<&str> {
        text(&self.mod_name).or(text(&self.name))
    }

    pub fn author(&self) -> Option<&str> {
        text(&self.author)
    }

    pub fn version(&self) -> Option<&str> {
        text(&self.version)
    }

    pub fn game_version(&self) -> Option<&str> {
        text(&self.game_version)
    }

    pub fn studio_version(&self) -> Option<&str> {
        text(&self.studio_version)
    }
}

fn text(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

fn relax(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let mut chars = json.chars().peekable();
    let mut string = false;
    while let Some(c) = chars.next() {
        if string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            }
            (',', _) => {
                let rest = chars.clone().find(|c| !c.is_whitespace());
                if !matches!(rest, Some('}' | ']')) {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hand_edited_files() {
        let json = "\u{FEFF}{\n  // made by hand\n  \"name\": \"Livery\", /* old */\n  \"mod_name\": \" Scuderia \",\n  \"author\": \"RM // Designs\",\n  \"version\": \"\",\n  \"game_version\": \"1.0.141.7865\",\n}\n";
        let info = ModInfo::parse(json.as_bytes()).unwrap();
        assert_eq!(info.title(), Some("Scuderia"));
        assert_eq!(info.author(), Some("RM // Designs"));
        assert_eq!(info.version(), None);
        assert_eq!(info.game_version(), Some("1.0.141.7865"));
        assert_eq!(
            ModInfo::parse(b"{\"Name\": \"x\", \"GameVersion\": \"1\"}")
                .unwrap()
                .title(),
            Some("x")
        );
        assert!(ModInfo::parse(b"not json").is_none());
    }
}
