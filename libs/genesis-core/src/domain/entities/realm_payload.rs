use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::error::GenesisError;

const MAX_LABEL: usize = 63;
const MAX_HOSTNAME: usize = 253;
const ADMINISTRATOR_REALM: &str = "master";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CellInstance {
    pub deployment_id: Uuid,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RealmPayloadV1 {
    pub deployment_id: Uuid,
    pub cell: CellInstance,
    pub realm: String,
    #[serde(default)]
    pub hostname: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantRoute {
    pub name: String,
    pub cell: IdentityInstanceRef,
}

impl RealmPayloadV1 {
    pub fn from_value(payload: &Value) -> Result<Self, GenesisError> {
        let parsed: Self =
            serde_json::from_value(payload.clone()).map_err(|e| invalid(e.to_string()))?;

        if !is_label(&parsed.realm) || parsed.realm == ADMINISTRATOR_REALM {
            return Err(invalid(format!(
                "'{}' is not a tenant realm name",
                parsed.realm
            )));
        }
        if !is_label(&parsed.cell.namespace) {
            return Err(invalid(format!(
                "'{}' is not a namespace",
                parsed.cell.namespace
            )));
        }
        if let Some(hostname) = &parsed.hostname {
            parsed.check_hostname(hostname)?;
        }

        Ok(parsed)
    }

    pub fn cell(&self) -> IdentityInstanceRef {
        IdentityInstanceRef::for_deployment(self.cell.deployment_id, self.cell.namespace.as_str())
    }

    pub fn route(&self) -> TenantRoute {
        TenantRoute {
            name: format!("realm-{}", self.deployment_id),
            cell: self.cell(),
        }
    }

    pub fn hostname(&self) -> Result<&str, GenesisError> {
        self.hostname
            .as_deref()
            .ok_or_else(|| invalid("a realm.create payload names its hostname".to_string()))
    }

    fn check_hostname(&self, hostname: &str) -> Result<(), GenesisError> {
        let well_formed = hostname.len() <= MAX_HOSTNAME && hostname.split('.').all(is_label);
        let first_label = hostname.split_once('.').map(|(label, _)| label);

        if well_formed && first_label == Some(self.realm.as_str()) {
            return Ok(());
        }
        Err(invalid(format!(
            "'{hostname}' is not a hostname of realm '{}'",
            self.realm
        )))
    }
}

fn is_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LABEL
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn invalid(message: String) -> GenesisError {
    GenesisError::InvalidPayload { message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TENANT: Uuid = Uuid::from_u128(1);
    const CELL: Uuid = Uuid::from_u128(2);

    fn payload(realm: &str, hostname: Option<&str>) -> Value {
        json!({
            "deployment_id": TENANT,
            "cell": { "deployment_id": CELL, "namespace": "cells-one" },
            "realm": realm,
            "hostname": hostname,
        })
    }

    #[test]
    fn a_create_payload_names_the_cell_instance_and_the_tenant_route() {
        let parsed =
            RealmPayloadV1::from_value(&payload("acme", Some("acme.cells.example"))).expect("ok");

        assert_eq!(
            parsed.cell(),
            IdentityInstanceRef::for_deployment(CELL, "cells-one")
        );
        assert_eq!(parsed.route().name, format!("realm-{TENANT}"));
        assert_eq!(parsed.route().cell.namespace, "cells-one");
        assert_eq!(parsed.hostname().expect("present"), "acme.cells.example");
    }

    #[test]
    fn a_delete_payload_needs_no_hostname() {
        let parsed = RealmPayloadV1::from_value(&payload("acme", None)).expect("ok");

        assert!(parsed.hostname().is_err());
        assert_eq!(parsed.realm, "acme");
    }

    #[test]
    fn the_administrator_realm_is_never_a_tenant() {
        let result = RealmPayloadV1::from_value(&payload("master", Some("master.cells.example")));

        assert!(matches!(result, Err(GenesisError::InvalidPayload { .. })));
    }

    #[test]
    fn a_realm_that_is_not_a_label_is_refused() {
        for realm in [
            "",
            "Acme",
            "-acme",
            "acme-",
            "ac me",
            "ac/me",
            &"a".repeat(64),
        ] {
            let result = RealmPayloadV1::from_value(&payload(realm, None));
            assert!(result.is_err(), "{realm}");
        }
    }

    #[test]
    fn a_hostname_of_another_realm_is_refused() {
        for hostname in [
            "other.cells.example",
            "acme",
            "acme..example",
            "ACME.cells.example",
            "acme.cells.example/x",
        ] {
            let result = RealmPayloadV1::from_value(&payload("acme", Some(hostname)));
            assert!(result.is_err(), "{hostname}");
        }
    }

    #[test]
    fn a_payload_without_a_cell_is_refused() {
        let result = RealmPayloadV1::from_value(&json!({
            "deployment_id": TENANT,
            "realm": "acme",
        }));

        assert!(result.is_err());
    }
}
