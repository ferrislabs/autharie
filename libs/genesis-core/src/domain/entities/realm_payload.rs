use std::fmt;

use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::error::GenesisError;

const MAX_LABEL: usize = 63;
const ADMINISTRATOR_REALM: &str = "master";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct RealmName(String);

impl RealmName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RealmName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for RealmName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if is_label(&value) && value != ADMINISTRATOR_REALM {
            Ok(Self(value))
        } else {
            Err(format!("'{value}' is not a tenant realm name"))
        }
    }
}

impl TryFrom<&str> for RealmName {
    type Error = GenesisError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_string()).map_err(invalid)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CellInstance {
    pub deployment_id: Uuid,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RealmPayloadV1 {
    pub deployment_id: Uuid,
    pub cell: CellInstance,
    pub realm: RealmName,
    #[serde(default)]
    pub hostname: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantRoute {
    pub name: String,
    pub cell: IdentityInstanceRef,
    pub deployment_id: Uuid,
    pub realm: RealmName,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteOwner {
    pub deployment: Option<String>,
    pub realm: Option<String>,
}

impl TenantRoute {
    pub fn is_owned_by(&self, owner: &RouteOwner) -> bool {
        owner.deployment.as_deref() == Some(self.deployment_id.to_string().as_str())
            && owner.realm.as_deref() == Some(self.realm.as_str())
    }
}

impl RealmPayloadV1 {
    pub fn from_value(payload: &Value) -> Result<Self, GenesisError> {
        let parsed: Self =
            serde_json::from_value(payload.clone()).map_err(|e| invalid(e.to_string()))?;

        if !is_label(&parsed.cell.namespace) {
            return Err(invalid(format!(
                "'{}' is not a namespace",
                parsed.cell.namespace
            )));
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
            deployment_id: self.deployment_id,
            realm: self.realm.clone(),
        }
    }

    pub fn hostname_in(&self, zone: &str) -> Result<&str, GenesisError> {
        let expected = format!("{}.{zone}", self.realm);
        match self.hostname.as_deref() {
            Some(hostname) if !zone.is_empty() && hostname == expected => Ok(hostname),
            Some(hostname) => Err(invalid(format!(
                "'{hostname}' is not the hostname of realm '{}' in zone '{zone}'",
                self.realm
            ))),
            None => Err(invalid(
                "a realm.create payload names its hostname".to_string(),
            )),
        }
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
    const ZONE: &str = "cells.example";

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
        let route = parsed.route();
        assert_eq!(route.name, format!("realm-{TENANT}"));
        assert_eq!(route.cell.namespace, "cells-one");
        assert_eq!(route.deployment_id, TENANT);
        assert_eq!(route.realm.as_str(), "acme");
        assert_eq!(
            parsed.hostname_in(ZONE).expect("present"),
            "acme.cells.example"
        );
    }

    #[test]
    fn a_delete_payload_needs_no_hostname() {
        let parsed = RealmPayloadV1::from_value(&payload("acme", None)).expect("ok");

        assert!(parsed.hostname_in(ZONE).is_err());
    }

    #[test]
    fn the_administrator_realm_is_never_a_tenant() {
        let result = RealmPayloadV1::from_value(&payload("master", Some("master.cells.example")));

        assert!(matches!(result, Err(GenesisError::InvalidPayload { .. })));
        assert!(RealmName::try_from("master").is_err());
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
            "../x",
            &"a".repeat(64),
        ] {
            let result = RealmPayloadV1::from_value(&payload(realm, None));
            assert!(result.is_err(), "{realm}");
        }
    }

    #[test]
    fn the_hostname_is_exactly_the_realm_under_the_zone() {
        for (hostname, zone) in [
            ("other.cells.example", ZONE),
            ("acme.other.example", ZONE),
            ("acme.eu.cells.example", ZONE),
            ("acme.cells.example.", ZONE),
            ("ACME.cells.example", ZONE),
            ("acme.", ""),
        ] {
            let parsed = RealmPayloadV1::from_value(&payload("acme", Some(hostname))).expect("ok");
            assert!(parsed.hostname_in(zone).is_err(), "{hostname} in {zone}");
        }
    }

    #[test]
    fn a_route_belongs_to_the_deployment_and_realm_on_its_labels() {
        let route = RealmPayloadV1::from_value(&payload("acme", None))
            .expect("ok")
            .route();
        let owner = |deployment: &str, realm: &str| RouteOwner {
            deployment: Some(deployment.to_string()),
            realm: Some(realm.to_string()),
        };

        assert!(route.is_owned_by(&owner(&TENANT.to_string(), "acme")));
        assert!(!route.is_owned_by(&owner(&TENANT.to_string(), "other")));
        assert!(!route.is_owned_by(&owner(&CELL.to_string(), "acme")));
        assert!(!route.is_owned_by(&RouteOwner::default()));
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
