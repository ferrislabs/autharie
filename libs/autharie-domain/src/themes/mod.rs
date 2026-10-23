use thiserror::Error;

pub mod commands;
pub mod entities;
pub mod payload;
pub mod ports;
pub mod service;
#[cfg(test)]
mod service_tests;
pub mod tokens;
pub mod value_objects;

pub use commands::{NewTheme, ThemeChange, ThemeOutcome};
pub use entities::{Theme, ThemeListing, ThemeState, ThemeSync, ThemeView};
pub use payload::PortalThemeActionPayload;
pub use tokens::{
    HexColour, ThemeBorders, ThemeColors, ThemeFontStyle, ThemeFonts, ThemeLinkFont,
    ThemeLinkStyle, ThemeShadow, ThemeSpacing, ThemeTokens,
};
pub use value_objects::{ThemeId, ThemeName, ThemeRealm, parse_realm};

pub const PORTAL_THEME_ACTION_TYPE: &str = "deployment.portal_theme";
pub const MAX_THEMES_PER_REALM: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ThemeError {
    #[error("{reason}")]
    Invalid { reason: String },

    #[error("no theme '{id}' in the realm '{realm}'")]
    NotFound { id: String, realm: String },

    #[error("a realm holds at most {MAX_THEMES_PER_REALM} themes")]
    ThemeLimit,

    #[error("the realm already has a theme named '{name}'")]
    NameTaken { name: String },

    #[error(
        "the active theme cannot be deleted: use FerrisKey's default or activate another first"
    )]
    ActiveTheme,

    #[error("themes are only available on FerrisKey deployments")]
    UnsupportedKind,
}

impl ThemeError {
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::Invalid {
            reason: reason.into(),
        }
    }
}
