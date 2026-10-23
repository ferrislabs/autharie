use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use super::{
    tokens::ThemeTokens,
    value_objects::{ThemeId, ThemeName, ThemeRealm},
};
use crate::deployments::DeploymentId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(tag = "phase")]
pub enum ThemeState {
    Pending,
    Applied,
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ThemeSync {
    #[serde(flatten)]
    pub state: ThemeState,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub id: ThemeId,
    pub deployment_id: DeploymentId,
    pub realm: ThemeRealm,
    pub name: ThemeName,
    pub tokens: ThemeTokens,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct ThemeView {
    pub id: ThemeId,
    pub name: ThemeName,
    pub tokens: ThemeTokens,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Theme> for ThemeView {
    fn from(theme: Theme) -> Self {
        Self {
            id: theme.id,
            name: theme.name,
            tokens: theme.tokens,
            active: theme.active,
            created_at: theme.created_at,
            updated_at: theme.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThemeListing {
    pub themes: Vec<Theme>,
    pub state: Option<ThemeSync>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_sync_is_flat_with_its_phase_message_and_revision() {
        assert_eq!(
            serde_json::to_value(ThemeSync {
                state: ThemeState::Pending,
                revision: 3
            })
            .unwrap(),
            json!({ "phase": "Pending", "revision": 3 })
        );
        assert_eq!(
            serde_json::to_value(ThemeSync {
                state: ThemeState::Failed {
                    message: "refused".to_string()
                },
                revision: 4
            })
            .unwrap(),
            json!({ "phase": "Failed", "message": "refused", "revision": 4 })
        );
    }
}
