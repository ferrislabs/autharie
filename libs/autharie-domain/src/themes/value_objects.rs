use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::ThemeError;
use crate::webhooks::WebhookRealm;

pub const MAX_NAME_LENGTH: usize = 80;

pub type ThemeRealm = WebhookRealm;

pub fn parse_realm(value: &str) -> Result<ThemeRealm, ThemeError> {
    WebhookRealm::parse(value).map_err(|error| ThemeError::invalid(error.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, format = Uuid)]
pub struct ThemeId(pub Uuid);

impl fmt::Display for ThemeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "Autumn")]
pub struct ThemeName(String);

impl ThemeName {
    pub fn parse(value: &str) -> Result<Self, ThemeError> {
        let value = value.trim();
        let length = value.chars().count();

        if length == 0 || length > MAX_NAME_LENGTH {
            return Err(ThemeError::invalid(format!(
                "name: must hold between 1 and {MAX_NAME_LENGTH} characters"
            )));
        }

        if value.chars().any(char::is_control) {
            return Err(ThemeError::invalid("name: may not hold control characters"));
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn key(&self) -> String {
        self.0.to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed_and_between_1_and_80_characters() {
        assert_eq!(ThemeName::parse("  Autumn ").unwrap().as_str(), "Autumn");
        assert!(ThemeName::parse("a").is_ok());
        assert!(ThemeName::parse(&"é".repeat(80)).is_ok());
        assert!(ThemeName::parse("").is_err());
        assert!(ThemeName::parse("   ").is_err());
        assert!(ThemeName::parse(&"é".repeat(81)).is_err());
        assert!(ThemeName::parse("two\nlines").is_err());
    }

    #[test]
    fn a_name_error_names_the_field() {
        assert!(
            ThemeName::parse("")
                .unwrap_err()
                .to_string()
                .starts_with("name:")
        );
    }

    #[test]
    fn names_that_differ_by_case_share_a_key() {
        assert_eq!(
            ThemeName::parse("Autumn").unwrap().key(),
            ThemeName::parse("aUTUMN").unwrap().key()
        );
    }
}
