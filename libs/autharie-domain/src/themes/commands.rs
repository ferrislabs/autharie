use serde_json::Value;

use super::{
    ThemeError,
    entities::ThemeState,
    tokens::ThemeTokens,
    value_objects::{ThemeName, ThemeRealm, parse_realm},
};

#[derive(Debug, Clone, PartialEq)]
pub struct NewTheme {
    pub name: ThemeName,
    pub tokens: ThemeTokens,
}

impl NewTheme {
    pub fn parse(name: &str, tokens: &Value) -> Result<Self, ThemeError> {
        Ok(Self {
            name: ThemeName::parse(name)?,
            tokens: ThemeTokens::parse(tokens)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThemeChange {
    pub name: Option<ThemeName>,
    pub tokens: Option<ThemeTokens>,
}

impl ThemeChange {
    pub fn parse(name: Option<&str>, tokens: Option<&Value>) -> Result<Self, ThemeError> {
        if name.is_none() && tokens.is_none() {
            return Err(ThemeError::invalid(
                "a change holds a name, tokens, or both",
            ));
        }

        Ok(Self {
            name: name.map(ThemeName::parse).transpose()?,
            tokens: tokens.map(ThemeTokens::parse).transpose()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeOutcome {
    pub revision: u64,
    pub realm: ThemeRealm,
    pub state: ThemeState,
}

impl ThemeOutcome {
    pub fn parse(
        revision: u64,
        realm: &str,
        items: Vec<(String, Option<String>)>,
    ) -> Result<Self, ThemeError> {
        let [(phase, message)] = <[_; 1]>::try_from(items).map_err(|items: Vec<_>| {
            ThemeError::invalid(format!(
                "a portal_theme report holds exactly one item, not {}",
                items.len()
            ))
        })?;

        let state = match phase.as_str() {
            "Pending" => ThemeState::Pending,
            "Applied" => ThemeState::Applied,
            "Failed" => ThemeState::Failed {
                message: message.unwrap_or_else(|| "the data plane gave no reason".to_string()),
            },
            other => {
                return Err(ThemeError::invalid(format!(
                    "'{other}' is not a theme phase"
                )));
            }
        };

        Ok(Self {
            revision,
            realm: parse_realm(realm)?,
            state,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_new_theme_needs_a_valid_name_and_valid_tokens() {
        let theme =
            NewTheme::parse(" Autumn ", &json!({ "colors": { "links": "#112233" } })).unwrap();

        assert_eq!(theme.name.as_str(), "Autumn");
        assert!(NewTheme::parse("", &json!({})).is_err());
        assert!(NewTheme::parse("Autumn", &json!({ "colors": { "links": "red" } })).is_err());
        assert!(NewTheme::parse("Autumn", &json!({ "nope": 1 })).is_err());
    }

    #[test]
    fn a_change_holds_something_and_what_it_holds_is_valid() {
        assert!(ThemeChange::parse(None, None).is_err());
        assert!(ThemeChange::parse(Some("  "), None).is_err());
        assert!(
            ThemeChange::parse(None, Some(&json!({ "spacing": { "fieldGap": 999 } }))).is_err()
        );

        let name_only = ThemeChange::parse(Some("Autumn"), None).unwrap();
        assert!(name_only.tokens.is_none());
        let tokens_only = ThemeChange::parse(None, Some(&json!({}))).unwrap();
        assert!(tokens_only.name.is_none());
        assert_eq!(tokens_only.tokens, Some(ThemeTokens::default()));
    }

    #[test]
    fn an_outcome_reads_its_single_item() {
        let applied =
            ThemeOutcome::parse(3, "master", vec![("Applied".to_string(), None)]).unwrap();
        assert_eq!(applied.state, ThemeState::Applied);
        assert_eq!(applied.revision, 3);

        let failed = ThemeOutcome::parse(
            3,
            "master",
            vec![("Failed".to_string(), Some("refused".to_string()))],
        )
        .unwrap();
        assert_eq!(
            failed.state,
            ThemeState::Failed {
                message: "refused".to_string()
            }
        );

        let pending =
            ThemeOutcome::parse(3, "master", vec![("Pending".to_string(), None)]).unwrap();
        assert_eq!(pending.state, ThemeState::Pending);
    }

    #[test]
    fn an_outcome_needs_one_item_a_known_phase_and_a_realm() {
        assert!(ThemeOutcome::parse(1, "master", vec![]).is_err());
        assert!(
            ThemeOutcome::parse(
                1,
                "master",
                vec![("Applied".to_string(), None), ("Applied".to_string(), None)]
            )
            .is_err()
        );
        assert!(ThemeOutcome::parse(1, "master", vec![("Exploded".to_string(), None)]).is_err());
        assert!(ThemeOutcome::parse(1, "bad realm", vec![("Applied".to_string(), None)]).is_err());
    }
}
