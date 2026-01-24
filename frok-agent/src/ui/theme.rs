use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThemeKind {
    NeonSunset,
    AcidMint,
    Infrared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Texture {
    None,
    Scanlines,
    Grid,
}

impl ThemeKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            ThemeKind::NeonSunset => "neon-sunset",
            ThemeKind::AcidMint => "acid-mint",
            ThemeKind::Infrared => "infrared",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "neon-sunset" | "neon" | "sunset" => Some(ThemeKind::NeonSunset),
            "acid-mint" | "acid" | "mint" => Some(ThemeKind::AcidMint),
            "infrared" | "ir" => Some(ThemeKind::Infrared),
            _ => None,
        }
    }
}

impl Texture {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Texture::None => "none",
            Texture::Scanlines => "scanlines",
            Texture::Grid => "grid",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "off" => Some(Texture::None),
            "scanlines" | "scanline" => Some(Texture::Scanlines),
            "grid" => Some(Texture::Grid),
            _ => None,
        }
    }
}

pub(crate) struct Theme {
    pub(crate) bg: Color,
    pub(crate) panel: Color,
    pub(crate) panel_border: Color,
    pub(crate) text: Color,
    pub(crate) muted: Color,
    pub(crate) accent: Color,
    pub(crate) accent_alt: Color,
    pub(crate) good: Color,
    pub(crate) warn: Color,
    pub(crate) danger: Color,
    pub(crate) selection_bg: Color,
    pub(crate) gradient_primary: [Color; 3],
    pub(crate) texture: Texture,
    pub(crate) texture_color: Color,
}

impl Theme {
    pub(crate) fn from_kind(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::NeonSunset => Self {
                bg: Color::Rgb(9, 12, 18),
                panel: Color::Rgb(16, 22, 32),
                panel_border: Color::Rgb(58, 68, 88),
                text: Color::Rgb(224, 232, 240),
                muted: Color::Rgb(138, 154, 170),
                accent: Color::Rgb(80, 204, 255),
                accent_alt: Color::Rgb(255, 189, 87),
                good: Color::Rgb(96, 220, 154),
                warn: Color::Rgb(255, 148, 92),
                danger: Color::Rgb(255, 98, 110),
                selection_bg: Color::Rgb(30, 24, 46),
                gradient_primary: [
                    Color::Rgb(80, 204, 255),
                    Color::Rgb(168, 92, 255),
                    Color::Rgb(255, 189, 87),
                ],
                texture: Texture::None,
                texture_color: Color::Rgb(14, 18, 26),
            },
            ThemeKind::AcidMint => Self {
                bg: Color::Rgb(8, 14, 16),
                panel: Color::Rgb(14, 22, 24),
                panel_border: Color::Rgb(54, 80, 78),
                text: Color::Rgb(214, 236, 230),
                muted: Color::Rgb(120, 160, 150),
                accent: Color::Rgb(110, 255, 182),
                accent_alt: Color::Rgb(198, 255, 120),
                good: Color::Rgb(110, 240, 178),
                warn: Color::Rgb(255, 184, 96),
                danger: Color::Rgb(255, 104, 120),
                selection_bg: Color::Rgb(20, 38, 34),
                gradient_primary: [
                    Color::Rgb(120, 255, 182),
                    Color::Rgb(72, 240, 210),
                    Color::Rgb(198, 255, 120),
                ],
                texture: Texture::None,
                texture_color: Color::Rgb(12, 20, 20),
            },
            ThemeKind::Infrared => Self {
                bg: Color::Rgb(14, 10, 10),
                panel: Color::Rgb(24, 14, 14),
                panel_border: Color::Rgb(80, 50, 50),
                text: Color::Rgb(238, 220, 220),
                muted: Color::Rgb(168, 132, 132),
                accent: Color::Rgb(255, 98, 110),
                accent_alt: Color::Rgb(255, 170, 100),
                good: Color::Rgb(255, 140, 120),
                warn: Color::Rgb(255, 170, 120),
                danger: Color::Rgb(255, 84, 90),
                selection_bg: Color::Rgb(46, 22, 22),
                gradient_primary: [
                    Color::Rgb(255, 98, 110),
                    Color::Rgb(255, 138, 96),
                    Color::Rgb(255, 200, 110),
                ],
                texture: Texture::None,
                texture_color: Color::Rgb(20, 14, 14),
            },
        }
    }
}
