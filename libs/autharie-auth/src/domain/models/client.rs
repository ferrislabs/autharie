use serde::{Deserialize, Serialize};

use crate::domain::models::claims::Actor;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Client {
    pub id: String,
    pub client_id: String,
    pub roles: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub act: Option<Actor>,
}
