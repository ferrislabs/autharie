use rand::Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::domain::entities::realm_payload::{RealmName, RealmPayloadV1};

pub const CONSOLE_CLIENT: &str = "security-admin-console";
pub const ADMINISTRATOR_USERNAME: &str = "admin";
pub const TENANT_ROLE: &str = "tenant-admin";
pub const TENANT_ROLE_DESCRIPTION: &str = "Administers this realm";
pub const TENANT_PERMISSIONS: [&str; 15] = [
    "manage_users",
    "view_users",
    "query_users",
    "manage_clients",
    "view_clients",
    "query_clients",
    "manage_roles",
    "view_roles",
    "manage_client_scopes",
    "view_client_scopes",
    "query_client_scopes",
    "view_realm",
    "view_events",
    "manage_organizations",
    "view_organizations",
];

const PASSWORD_LENGTH: usize = 24;
const SYMBOLS: &[u8] = b"-_.!@#%+=";
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!@#%+=";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct InitialAdministrator {
    username: String,
    password: String,
}

impl InitialAdministrator {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn password(&self) -> &str {
        &self.password
    }

    pub fn generate(username: &str) -> Self {
        let mut rng = rand::rng();
        loop {
            let mut candidate = String::with_capacity(PASSWORD_LENGTH);
            for _ in 0..PASSWORD_LENGTH {
                candidate.push(char::from(ALPHABET[rng.random_range(0..ALPHABET.len())]));
            }
            if covers_every_class(&candidate) {
                return Self::new(username, candidate);
            }
            candidate.zeroize();
        }
    }
}

fn covers_every_class(candidate: &str) -> bool {
    let bytes = candidate.as_bytes();
    bytes.iter().any(u8::is_ascii_uppercase)
        && bytes.iter().any(u8::is_ascii_lowercase)
        && bytes.iter().any(u8::is_ascii_digit)
        && bytes.iter().any(|byte| SYMBOLS.contains(byte))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantAccess {
    pub realm: RealmName,
    pub console_callback: String,
    pub administrator_id: Uuid,
}

impl TenantAccess {
    pub fn for_realm(payload: &RealmPayloadV1, hostname: &str) -> Self {
        Self {
            realm: payload.realm.clone(),
            console_callback: format!(
                "https://{hostname}/realms/{}/authentication/callback",
                payload.realm
            ),
            administrator_id: payload.deployment_id,
        }
    }
}

pub enum AccessGrant {
    Issued(InitialAdministrator),
    AlreadyIssued,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const TENANT: Uuid = Uuid::from_u128(1);

    fn payload() -> RealmPayloadV1 {
        RealmPayloadV1::from_value(&json!({
            "deployment_id": TENANT,
            "cell": { "deployment_id": Uuid::from_u128(2), "namespace": "cells-one" },
            "realm": "acme",
            "hostname": "acme.cells.example",
        }))
        .expect("a payload")
    }

    #[test]
    fn generated_passwords_are_24_characters_covering_every_class() {
        for _ in 0..200 {
            let admin = InitialAdministrator::generate("admin");
            let password = admin.password();
            assert_eq!(admin.username(), "admin");
            assert_eq!(password.chars().count(), 24);
            assert!(password.chars().any(|c| c.is_ascii_uppercase()));
            assert!(password.chars().any(|c| c.is_ascii_lowercase()));
            assert!(password.chars().any(|c| c.is_ascii_digit()));
            assert!(password.chars().any(|c| "-_.!@#%+=".contains(c)));
            assert!(password.bytes().all(|byte| ALPHABET.contains(&byte)));
        }
    }

    #[test]
    fn two_generated_passwords_differ() {
        let first = InitialAdministrator::generate("admin");
        let second = InitialAdministrator::generate("admin");

        assert_ne!(first.password(), second.password());
    }

    #[test]
    fn the_console_callback_is_the_realm_path_on_the_tenant_hostname() {
        let access = TenantAccess::for_realm(&payload(), "acme.cells.example");

        assert_eq!(access.realm.as_str(), "acme");
        assert_eq!(
            access.console_callback,
            "https://acme.cells.example/realms/acme/authentication/callback"
        );
        assert_eq!(access.administrator_id, TENANT);
    }

    #[test]
    fn the_tenant_role_never_manages_the_realm() {
        assert_eq!(TENANT_PERMISSIONS.len(), 15);
        assert!(!TENANT_PERMISSIONS.contains(&"manage_realm"));
        assert!(TENANT_PERMISSIONS.iter().all(|permission| {
            permission.starts_with("manage_")
                || permission.starts_with("view_")
                || permission.starts_with("query_")
        }));
        assert_eq!(TENANT_ROLE, "tenant-admin");
    }
}
