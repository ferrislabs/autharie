use std::collections::BTreeSet;

use autharie_domain::dataplane::{
    cloud_provider::{CredentialVerifier, Provider},
    credential::{CredentialError, ScopeCheck, SecretString},
};
use chrono::Utc;

use crate::{
    config::ScalewayConfig,
    error::{ApiError, ScalewayError},
    http::{Http, Session},
    models::{ApiKey, PolicyList, Project, RuleList},
};

pub const REQUIRED_PERMISSION_SETS: [&str; 4] = [
    "KubernetesFullAccess",
    "InstancesReadOnly",
    "PrivateNetworksFullAccess",
    "ProjectReadOnly",
];

pub const INSPECTION_PERMISSION_SET: &str = "IAMReadOnly";

pub struct ScalewayVerifier {
    http: Http,
}

impl ScalewayVerifier {
    pub fn new(config: &ScalewayConfig) -> Result<Self, ScalewayError> {
        Ok(Self {
            http: Http::new(config)?,
        })
    }

    async fn granted(&self, session: &Session<'_>) -> Result<BTreeSet<String>, ApiError> {
        let key: ApiKey = session
            .get(
                &format!("/iam/v1alpha1/api-keys/{}", session.access_key),
                &[],
            )
            .await?;
        let project: Project = session
            .get(&format!("/account/v3/projects/{}", session.project_id), &[])
            .await?;

        let (principal_filter, principal_id) = match (key.application_id, key.user_id) {
            (Some(application_id), _) => ("application_ids", application_id),
            (None, Some(user_id)) => ("user_ids", user_id),
            (None, None) => return Ok(BTreeSet::new()),
        };

        let policies: PolicyList = session
            .get(
                "/iam/v1alpha1/policies",
                &[
                    ("organization_id", project.organization_id.as_str()),
                    (principal_filter, principal_id.as_str()),
                    ("page_size", "100"),
                ],
            )
            .await?;

        let mut granted = BTreeSet::new();
        for policy in &policies.policies {
            let rules: RuleList = session
                .get(
                    "/iam/v1alpha1/rules",
                    &[("policy_id", policy.id.as_str()), ("page_size", "100")],
                )
                .await?;
            granted.extend(
                rules
                    .rules
                    .into_iter()
                    .flat_map(|rule| rule.permission_set_names),
            );
        }
        Ok(granted)
    }
}

impl CredentialVerifier for ScalewayVerifier {
    async fn verify(
        &self,
        _provider: Provider,
        secret: &SecretString,
    ) -> Result<ScopeCheck, CredentialError> {
        let session = Session::open(&self.http, secret)?;

        let granted = match self.granted(&session).await {
            Ok(granted) => granted,
            Err(error) if error.is_denied() && error.has_kind("permissions_denied") => {
                return Err(CredentialError::MissingPermissions {
                    missing: vec![INSPECTION_PERMISSION_SET.to_string()],
                });
            }
            Err(error) if error.is_denied() => return Err(CredentialError::Invalid),
            Err(error) => return Err(CredentialError::Store(error.to_string())),
        };

        let missing: Vec<String> = REQUIRED_PERMISSION_SETS
            .iter()
            .filter(|required| !granted.contains(**required))
            .map(|required| required.to_string())
            .collect();
        if !missing.is_empty() {
            return Err(CredentialError::MissingPermissions { missing });
        }

        let extra: Vec<String> = granted
            .iter()
            .filter(|name| {
                name.as_str() != INSPECTION_PERMISSION_SET
                    && !REQUIRED_PERMISSION_SETS.contains(&name.as_str())
            })
            .cloned()
            .collect();
        if !extra.is_empty() {
            return Err(CredentialError::ExcessPermissions { extra });
        }

        Ok(ScopeCheck {
            checked_at: Utc::now(),
        })
    }
}
