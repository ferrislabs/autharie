use serde::Serialize;
use utoipa::ToSchema;

use crate::deployments::{Deployment, distribution::Distribution};

use super::{RealmName, TenantAdministrator};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct TenantRealm {
    pub name: RealmName,
    pub url: Option<String>,
    pub issuer: Option<String>,
    pub console: Option<String>,
    pub refusal: Option<String>,
    pub administrator: Option<TenantAdministrator>,
}

impl TenantRealm {
    pub fn of(
        deployment: &Deployment,
        zone: Option<&str>,
        refusal: Option<String>,
    ) -> Option<Self> {
        let Distribution::Pooled { realm, .. } = &deployment.distribution else {
            return None;
        };
        let url = zone.map(|zone| format!("https://{realm}.{zone}"));

        Some(Self {
            name: realm.clone(),
            issuer: url.as_ref().map(|url| format!("{url}/api/realms/{realm}")),
            console: url
                .as_ref()
                .map(|url| format!("{url}/realms/{realm}/overview")),
            url,
            refusal,
            administrator: None,
        })
    }

    pub fn with_administrator(mut self, administrator: Option<TenantAdministrator>) -> Self {
        self.administrator = administrator;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cells::CellId;

    fn pooled(realm: &str) -> Deployment {
        let mut deployment = crate::deployments::tests::a_deployment();
        deployment.distribution = Distribution::Pooled {
            cell_id: CellId(uuid::Uuid::nil()),
            realm: RealmName::try_from(realm).expect("valid"),
        };
        deployment
    }

    #[test]
    fn a_realm_is_reached_at_its_hostname_and_issues_tokens_under_its_own_path() {
        let realm = TenantRealm::of(&pooled("acme"), Some("autharie.fr"), None).expect("pooled");

        assert_eq!(realm.url.as_deref(), Some("https://acme.autharie.fr"));
        assert_eq!(
            realm.issuer.as_deref(),
            Some("https://acme.autharie.fr/api/realms/acme")
        );
    }

    #[test]
    fn a_realm_is_administered_from_the_console_of_its_own_hostname() {
        let realm = TenantRealm::of(&pooled("acme"), Some("autharie.fr"), None).expect("pooled");

        assert_eq!(
            realm.console.as_deref(),
            Some("https://acme.autharie.fr/realms/acme/overview")
        );
    }

    #[test]
    fn without_a_zone_a_realm_has_no_address_to_show() {
        let realm = TenantRealm::of(&pooled("acme"), None, None).expect("pooled");

        assert_eq!((realm.url, realm.issuer, realm.console), (None, None, None));
    }

    #[test]
    fn a_refused_realm_carries_its_reason() {
        let realm = TenantRealm::of(
            &pooled("acme"),
            Some("autharie.fr"),
            Some("the cell refused the realm".to_string()),
        )
        .expect("pooled");

        assert_eq!(realm.refusal.as_deref(), Some("the cell refused the realm"));
    }

    #[test]
    fn a_dedicated_deployment_has_no_tenant_realm() {
        let dedicated = crate::deployments::tests::a_deployment();

        assert_eq!(TenantRealm::of(&dedicated, Some("autharie.fr"), None), None);
    }
}
