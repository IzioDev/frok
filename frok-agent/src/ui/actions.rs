use super::input::fuzzy_match;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteActionId {
    AddRoute,
    DeleteRoute,
    CopyUrl,
    OpenUrl,
    ShowLogs,
    Connect,
    ClearLogs,
    OpenGithub,
    ThemeNeonSunset,
    ThemeAcidMint,
    ThemeInfrared,
    TextureNone,
    TextureScanlines,
    TextureGrid,
    Help,
    Quit,
}

#[derive(Debug, Clone)]
pub(crate) struct PaletteAction {
    pub(crate) id: PaletteActionId,
    pub(crate) label: &'static str,
    pub(crate) desc: &'static str,
    pub(crate) keywords: &'static [&'static str],
}

pub(crate) const PALETTE_ACTIONS: &[PaletteAction] = &[
    PaletteAction {
        id: PaletteActionId::AddRoute,
        label: "Arm route",
        desc: "Make local public",
        keywords: &["add", "route", "new", "create", "arm"],
    },
    PaletteAction {
        id: PaletteActionId::DeleteRoute,
        label: "Delete route",
        desc: "Disarm selected route",
        keywords: &["delete", "remove", "route"],
    },
    PaletteAction {
        id: PaletteActionId::CopyUrl,
        label: "Copy URL",
        desc: "Copy public link",
        keywords: &["copy", "url", "clipboard"],
    },
    PaletteAction {
        id: PaletteActionId::OpenUrl,
        label: "Open URL",
        desc: "Open public link",
        keywords: &["open", "url", "browser"],
    },
    PaletteAction {
        id: PaletteActionId::ShowLogs,
        label: "Logs",
        desc: "Open signal stream",
        keywords: &["logs", "signal", "events"],
    },
    PaletteAction {
        id: PaletteActionId::Connect,
        label: "Connect",
        desc: "Reconnect to edge",
        keywords: &["connect", "reconnect", "edge"],
    },
    PaletteAction {
        id: PaletteActionId::ClearLogs,
        label: "Clear logs",
        desc: "Purge log buffer",
        keywords: &["clear", "logs"],
    },
    PaletteAction {
        id: PaletteActionId::OpenGithub,
        label: "Open GitHub",
        desc: "Visit the frok repo",
        keywords: &["github", "repo", "source", "code"],
    },
    PaletteAction {
        id: PaletteActionId::ThemeNeonSunset,
        label: "Theme: Neon Sunset",
        desc: "Cyan > violet > amber",
        keywords: &["theme", "neon", "sunset", "gradient"],
    },
    PaletteAction {
        id: PaletteActionId::ThemeAcidMint,
        label: "Theme: Acid Mint",
        desc: "Mint > aqua > lime",
        keywords: &["theme", "acid", "mint", "gradient"],
    },
    PaletteAction {
        id: PaletteActionId::ThemeInfrared,
        label: "Theme: Infrared",
        desc: "Red > ember > heat",
        keywords: &["theme", "infrared", "red", "gradient"],
    },
    PaletteAction {
        id: PaletteActionId::TextureNone,
        label: "Texture: None",
        desc: "Clean background",
        keywords: &["texture", "none", "clean"],
    },
    PaletteAction {
        id: PaletteActionId::TextureScanlines,
        label: "Texture: Scanlines",
        desc: "Retro scanline wash",
        keywords: &["texture", "scanlines", "retro"],
    },
    PaletteAction {
        id: PaletteActionId::TextureGrid,
        label: "Texture: Grid",
        desc: "Faint grid overlay",
        keywords: &["texture", "grid", "retro"],
    },
    PaletteAction {
        id: PaletteActionId::Help,
        label: "Help",
        desc: "Show command help",
        keywords: &["help", "commands"],
    },
    PaletteAction {
        id: PaletteActionId::Quit,
        label: "Quit",
        desc: "Exit frok",
        keywords: &["quit", "exit", "close"],
    },
];

pub(crate) fn filter_actions(query: &str) -> Vec<&'static PaletteAction> {
    let trimmed = query.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return PALETTE_ACTIONS.iter().collect();
    }

    PALETTE_ACTIONS
        .iter()
        .filter(|action| matches_query(&trimmed, action))
        .collect()
}

fn matches_query(query: &str, action: &PaletteAction) -> bool {
    if fuzzy_match(query, action.label) {
        return true;
    }
    if fuzzy_match(query, action.desc) {
        return true;
    }
    action
        .keywords
        .iter()
        .any(|keyword| fuzzy_match(query, keyword))
}
