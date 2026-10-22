use std::collections::BTreeMap;

use autharie_auth::Identity;
use chrono::Utc;
use serde_json::{Map, Value, json};

use crate::{
    CoreError,
    action::{
        Action, ActionPayload, ActionSource, ActionTarget, ActionType, ActionVersion, TargetKind,
        commands::RecordActionCommand,
        ports::{ActionRepository, ActionService},
        service::ActionServiceImpl,
    },
    audit::{
        AuditAction, AuditChange, AuditEntry, AuditEntryId, AuditTarget, AuditTargetKind,
        ports::AuditRepository, service::audit_actor,
    },
    dataplane::{
        credential::SecretString, herald_identity::speaking_for, ports::DataPlaneRepository,
    },
    deployments::{Deployment, DeploymentId, DeploymentKind, ports::DeploymentRepository},
    generate_uuid_v7,
    organisation::{OrganisationId, ports::OrganisationRepository},
    user::ports::UserRepository,
    webhooks::{
        RETRY_DELIVERY_ACTION_TYPE, ROTATE_SECRET_ACTION_TYPE, WEBHOOK_CREATED_AUDIT,
        WEBHOOK_DELETED_AUDIT, WEBHOOK_REVEALED_AUDIT, WEBHOOK_ROTATED_AUDIT,
        WEBHOOK_UPDATED_AUDIT, WEBHOOKS_ACTION_TYPE, WebhookError,
        commands::{WebhookInput, WebhooksOutcome},
        delivery::{DeliveryQuery, MAX_DELIVERIES, WebhookDelivery, WebhookDeliveryIndex},
        entities::{
            RequestKind, RequestPhase, RevealedSecret, SecretRecord, StoredWebhook, Webhook,
            WebhookRequest, WebhookState, WebhookView,
        },
        ports::{
            SecretVault, WebhookOutcomeService, WebhookPolicy, WebhookRepository, WebhookService,
        },
        value_objects::{DEFAULT_REALM, HeaderInput, WebhookId, WebhookRealm, parse_delivery_id},
    },
};

const REQUESTS_SHOWN_PER_WEBHOOK: usize = 5;

fn headers_context(id: WebhookId) -> String {
    format!("webhook-headers:{id}")
}

fn secret_context(id: WebhookId, version: u64) -> String {
    format!("webhook-secret:{id}:{version}")
}

fn without_query(endpoint: &str) -> &str {
    endpoint.split(['?', '#']).next().unwrap_or(endpoint)
}

fn summary(webhook: &Webhook) -> Value {
    json!({
        "deployment_id": webhook.deployment_id.0,
        "realm": webhook.realm.as_str(),
        "name": webhook.name.as_str(),
        "endpoint": without_query(webhook.endpoint.as_str()),
        "events": webhook.events.iter().map(|event| event.name()).collect::<Vec<_>>(),
        "header_names": webhook.headers.iter().map(|header| header.as_str()).collect::<Vec<_>>(),
    })
}

pub struct WebhookServiceImpl<D, O, W, A, AU, U, V, P>
where
    A: ActionRepository,
{
    deployments: D,
    organisations: O,
    webhooks: W,
    actions: ActionServiceImpl<A>,
    audit: AU,
    users: U,
    vault: V,
    policy: P,
}

impl<D, O, W, A, AU, U, V, P> WebhookServiceImpl<D, O, W, A, AU, U, V, P>
where
    A: ActionRepository,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        deployments: D,
        organisations: O,
        webhooks: W,
        actions: A,
        audit: AU,
        users: U,
        vault: V,
        policy: P,
    ) -> Self {
        Self {
            deployments,
            organisations,
            webhooks,
            actions: ActionServiceImpl::new(actions),
            audit,
            users,
            vault,
            policy,
        }
    }
}

impl<D, O, W, A, AU, U, V, P> WebhookServiceImpl<D, O, W, A, AU, U, V, P>
where
    D: DeploymentRepository,
    O: OrganisationRepository,
    W: WebhookRepository,
    A: ActionRepository,
    AU: AuditRepository,
    U: UserRepository,
    V: SecretVault,
    P: WebhookPolicy,
{
    async fn deployment(
        &self,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Deployment, CoreError> {
        self.deployments
            .get_by_id(deployment_id)
            .await?
            .filter(|deployment| deployment.organisation_id == organisation_id)
            .filter(|deployment| deployment.deleted_at.is_none())
            .ok_or(CoreError::DeploymentNotFound {
                id: deployment_id.0,
            })
    }

    async fn managed_deployment(
        &self,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Deployment, CoreError> {
        let deployment = self.deployment(organisation_id, deployment_id).await?;

        if deployment.kind != DeploymentKind::Ferriskey {
            return Err(WebhookError::UnsupportedKind.into());
        }

        Ok(deployment)
    }

    async fn stored(
        &self,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> Result<StoredWebhook, CoreError> {
        self.webhooks
            .get(deployment_id, webhook_id)
            .await?
            .ok_or_else(|| WebhookError::NotFound { id: webhook_id.0 }.into())
    }

    async fn check_realm(
        &self,
        deployment_id: DeploymentId,
        realm: &WebhookRealm,
    ) -> Result<(), CoreError> {
        let sync = self.webhooks.sync_state(deployment_id).await?;

        let known = match sync.realms.filter(|realms| !realms.is_empty()) {
            Some(realms) => realms.iter().any(|known| known == realm.as_str()),
            None => realm.as_str() == DEFAULT_REALM,
        };

        if known {
            return Ok(());
        }

        Err(WebhookError::invalid(format!(
            "the realm '{}' does not exist on this deployment",
            realm.as_str()
        ))
        .into())
    }

    async fn open_headers(
        &self,
        stored: &StoredWebhook,
    ) -> Result<BTreeMap<String, String>, CoreError> {
        let Some(sealed) = &stored.sealed_headers else {
            return Ok(BTreeMap::new());
        };

        let opened = self
            .vault
            .open(&headers_context(stored.webhook.id), sealed)
            .await?;

        serde_json::from_str(opened.expose()).map_err(|_| {
            CoreError::InternalError("the stored header values cannot be read".to_string())
        })
    }

    async fn seal_headers(
        &self,
        id: WebhookId,
        values: &BTreeMap<String, String>,
    ) -> Result<Option<crate::dataplane::credential_repository::SealedSecret>, CoreError> {
        if values.is_empty() {
            return Ok(None);
        }

        let text = serde_json::to_string(values)
            .map_err(|error| CoreError::InternalError(error.to_string()))?;

        Ok(Some(
            self.vault
                .seal(&headers_context(id), &SecretString::new(text))
                .await?,
        ))
    }

    async fn desired_payload(
        &self,
        deployment: &Deployment,
        revision: u64,
    ) -> Result<Value, CoreError> {
        let stored = self.webhooks.list(deployment.id).await?;
        let mut webhooks = Vec::with_capacity(stored.len());
        let mut headers = Map::new();

        for entry in &stored {
            let webhook = &entry.webhook;
            webhooks.push(json!({
                "id": webhook.id.0,
                "realm": webhook.realm.as_str(),
                "name": webhook.name.as_str(),
                "description": webhook.description,
                "endpoint": webhook.endpoint.as_str(),
                "events": webhook.events.iter().map(|event| event.name()).collect::<Vec<_>>(),
                "header_names": webhook.headers.iter().map(|header| header.as_str()).collect::<Vec<_>>(),
            }));

            let values = self.open_headers(entry).await?;
            if !values.is_empty() {
                headers.insert(webhook.id.0.to_string(), json!(values));
            }
        }

        Ok(json!({
            "deployment_id": deployment.id.0,
            "namespace": deployment.namespace,
            "revision": revision,
            "webhooks": webhooks,
            "headers": headers,
        }))
    }

    async fn record(
        &self,
        deployment: &Deployment,
        action_type: &str,
        data: Value,
    ) -> Result<Action, CoreError> {
        self.actions
            .record_action(RecordActionCommand::new(
                deployment.id,
                deployment.dataplane_id,
                ActionType(action_type.to_string()),
                ActionTarget {
                    kind: TargetKind::Deployment,
                    id: deployment.id.0,
                },
                ActionPayload { data },
                ActionVersion(1),
                ActionSource::System,
            ))
            .await
    }

    async fn record_desired_set(
        &self,
        deployment: &Deployment,
        revision: u64,
    ) -> Result<(), CoreError> {
        let payload = self.desired_payload(deployment, revision).await?;
        self.record(deployment, WEBHOOKS_ACTION_TYPE, payload)
            .await?;

        Ok(())
    }

    async fn write_audit(
        &self,
        identity: &Identity,
        organisation_id: OrganisationId,
        action: &str,
        webhook_id: WebhookId,
        change: Option<AuditChange>,
    ) -> Result<(), CoreError> {
        self.audit
            .append(AuditEntry::record(
                AuditEntryId(generate_uuid_v7()),
                organisation_id,
                audit_actor(identity, &self.users).await?,
                AuditAction(action.to_string()),
                AuditTarget {
                    kind: AuditTargetKind::Custom("webhook".to_string()),
                    id: webhook_id.0,
                },
                change,
                Utc::now(),
            ))
            .await
    }

    async fn view(
        &self,
        deployment_id: DeploymentId,
        stored: StoredWebhook,
    ) -> Result<WebhookView, CoreError> {
        let id = stored.webhook.id;
        let requests = self
            .webhooks
            .recent_requests(deployment_id, REQUESTS_SHOWN_PER_WEBHOOK)
            .await?
            .into_iter()
            .filter(|request| request.webhook_id == id)
            .collect();

        Ok(WebhookView {
            webhook: stored.webhook,
            requests,
        })
    }

    fn values_for_create(headers: &[HeaderInput]) -> Result<BTreeMap<String, String>, CoreError> {
        headers
            .iter()
            .map(|header| {
                header
                    .value
                    .as_ref()
                    .map(|value| (header.name.as_str().to_string(), value.expose().to_string()))
                    .ok_or_else(|| {
                        WebhookError::invalid(format!(
                            "the header '{}' needs a value",
                            header.name.as_str()
                        ))
                        .into()
                    })
            })
            .collect()
    }

    fn values_for_update(
        headers: &[HeaderInput],
        current: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, CoreError> {
        headers
            .iter()
            .map(|header| {
                let value = match &header.value {
                    Some(value) => value.expose().to_string(),
                    None => current
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(header.name.as_str()))
                        .map(|(_, value)| value.clone())
                        .ok_or_else(|| {
                            WebhookError::invalid(format!(
                                "the header '{}' needs a value",
                                header.name.as_str()
                            ))
                        })?,
                };

                Ok((header.name.as_str().to_string(), value))
            })
            .collect()
    }
}

impl<D, O, W, A, AU, U, V, P> WebhookService for WebhookServiceImpl<D, O, W, A, AU, U, V, P>
where
    D: DeploymentRepository,
    O: OrganisationRepository,
    W: WebhookRepository,
    A: ActionRepository,
    AU: AuditRepository,
    U: UserRepository,
    V: SecretVault,
    P: WebhookPolicy,
{
    async fn list_webhooks(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<WebhookView>, CoreError> {
        self.policy
            .can_view_webhooks(identity, organisation_id)
            .await?;
        self.deployment(organisation_id, deployment_id).await?;

        let requests = self
            .webhooks
            .recent_requests(deployment_id, REQUESTS_SHOWN_PER_WEBHOOK)
            .await?;

        Ok(self
            .webhooks
            .list(deployment_id)
            .await?
            .into_iter()
            .map(|stored| WebhookView {
                requests: requests
                    .iter()
                    .filter(|request| request.webhook_id == stored.webhook.id)
                    .cloned()
                    .collect(),
                webhook: stored.webhook,
            })
            .collect())
    }

    async fn get_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> Result<WebhookView, CoreError> {
        self.policy
            .can_view_webhooks(identity, organisation_id)
            .await?;
        self.deployment(organisation_id, deployment_id).await?;

        let stored = self.stored(deployment_id, webhook_id).await?;

        self.view(deployment_id, stored).await
    }

    async fn create_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: WebhookInput,
    ) -> Result<WebhookView, CoreError> {
        self.policy
            .can_manage_webhooks(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;

        let revision = self.webhooks.next_revision(deployment_id).await?;

        let organisation = self
            .organisations
            .find_by_id(&organisation_id)
            .await?
            .ok_or(CoreError::OrganisationNotFound {
                id: organisation_id.0,
            })?;
        let held = self.webhooks.list(deployment_id).await?.len();
        let max = organisation.plan.max_webhooks();
        if held >= max {
            return Err(CoreError::OrganisationLimitReached {
                limit_type: "webhooks".to_string(),
                max,
                current: held,
            });
        }

        self.check_realm(deployment_id, &input.realm).await?;

        let id = WebhookId(generate_uuid_v7());
        let values = Self::values_for_create(&input.headers)?;
        let sealed_headers = self.seal_headers(id, &values).await?;
        let now = Utc::now();

        let stored = StoredWebhook {
            webhook: Webhook {
                id,
                deployment_id,
                realm: input.realm,
                name: input.name,
                description: input.description,
                endpoint: input.endpoint,
                events: input.events,
                headers: input
                    .headers
                    .into_iter()
                    .map(|header| header.name)
                    .collect(),
                state: WebhookState::Pending,
                secret_version: None,
                secret_received_at: None,
                desired_revision: revision,
                created_at: now,
                updated_at: now,
            },
            sealed_headers,
        };

        self.webhooks.insert(&stored).await?;
        self.record_desired_set(&deployment, revision).await?;
        self.write_audit(
            &identity,
            organisation_id,
            WEBHOOK_CREATED_AUDIT,
            id,
            Some(AuditChange::new(Value::Null, summary(&stored.webhook))?),
        )
        .await?;

        self.view(deployment_id, stored).await
    }

    async fn update_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
        input: WebhookInput,
    ) -> Result<WebhookView, CoreError> {
        self.policy
            .can_manage_webhooks(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;

        let revision = self.webhooks.next_revision(deployment_id).await?;
        let before = self.stored(deployment_id, webhook_id).await?;
        self.check_realm(deployment_id, &input.realm).await?;

        let current = self.open_headers(&before).await?;
        let values = Self::values_for_update(&input.headers, &current)?;
        let sealed_headers = self.seal_headers(webhook_id, &values).await?;

        let stored = StoredWebhook {
            webhook: Webhook {
                realm: input.realm,
                name: input.name,
                description: input.description,
                endpoint: input.endpoint,
                events: input.events,
                headers: input
                    .headers
                    .into_iter()
                    .map(|header| header.name)
                    .collect(),
                state: WebhookState::Pending,
                desired_revision: revision,
                updated_at: Utc::now(),
                ..before.webhook.clone()
            },
            sealed_headers,
        };

        if !self.webhooks.update(&stored).await? {
            return Err(WebhookError::NotFound { id: webhook_id.0 }.into());
        }
        self.record_desired_set(&deployment, revision).await?;
        self.write_audit(
            &identity,
            organisation_id,
            WEBHOOK_UPDATED_AUDIT,
            webhook_id,
            Some(AuditChange::new(
                summary(&before.webhook),
                summary(&stored.webhook),
            )?),
        )
        .await?;

        self.view(deployment_id, stored).await
    }

    async fn delete_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> Result<(), CoreError> {
        self.policy
            .can_manage_webhooks(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;

        let revision = self.webhooks.next_revision(deployment_id).await?;
        let before = self.stored(deployment_id, webhook_id).await?;

        if !self.webhooks.delete(deployment_id, webhook_id).await? {
            return Err(WebhookError::NotFound { id: webhook_id.0 }.into());
        }
        self.record_desired_set(&deployment, revision).await?;
        self.write_audit(
            &identity,
            organisation_id,
            WEBHOOK_DELETED_AUDIT,
            webhook_id,
            Some(AuditChange::new(summary(&before.webhook), Value::Null)?),
        )
        .await
    }

    async fn rotate_webhook_secret(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> Result<WebhookRequest, CoreError> {
        self.policy
            .can_manage_webhooks(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;
        let stored = self.stored(deployment_id, webhook_id).await?;

        let now = Utc::now();
        let request = WebhookRequest {
            id: generate_uuid_v7(),
            webhook_id,
            kind: RequestKind::RotateSecret,
            delivery_id: None,
            phase: RequestPhase::Pending,
            message: None,
            created_at: now,
            updated_at: now,
        };
        self.webhooks.insert_request(&request).await?;

        self.record(
            &deployment,
            ROTATE_SECRET_ACTION_TYPE,
            json!({
                "deployment_id": deployment.id.0,
                "namespace": deployment.namespace,
                "webhook_id": webhook_id.0,
                "request_id": request.id,
            }),
        )
        .await?;

        self.write_audit(
            &identity,
            organisation_id,
            WEBHOOK_ROTATED_AUDIT,
            webhook_id,
            Some(AuditChange::new(
                json!({ "version": stored.webhook.secret_version }),
                json!({ "requested": true }),
            )?),
        )
        .await?;

        Ok(request)
    }

    async fn reveal_webhook_secret(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> Result<RevealedSecret, CoreError> {
        self.policy
            .can_manage_webhooks(identity.clone(), organisation_id)
            .await?;
        self.deployment(organisation_id, deployment_id).await?;
        self.stored(deployment_id, webhook_id).await?;

        let record = self
            .webhooks
            .secret(webhook_id)
            .await?
            .ok_or(WebhookError::SecretNotReceived)?;
        let secret = self
            .vault
            .open(&secret_context(webhook_id, record.version), &record.sealed)
            .await?;

        self.write_audit(
            &identity,
            organisation_id,
            WEBHOOK_REVEALED_AUDIT,
            webhook_id,
            Some(AuditChange::new(
                Value::Null,
                json!({ "deployment_id": deployment_id.0, "version": record.version }),
            )?),
        )
        .await?;

        Ok(RevealedSecret {
            secret,
            version: record.version,
        })
    }

    async fn retry_webhook_delivery(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
        delivery_id: String,
    ) -> Result<WebhookRequest, CoreError> {
        self.policy
            .can_manage_webhooks(identity, organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;
        self.stored(deployment_id, webhook_id).await?;
        let delivery_id = parse_delivery_id(&delivery_id)?;

        let now = Utc::now();
        let request = WebhookRequest {
            id: generate_uuid_v7(),
            webhook_id,
            kind: RequestKind::RetryDelivery,
            delivery_id: Some(delivery_id.clone()),
            phase: RequestPhase::Pending,
            message: None,
            created_at: now,
            updated_at: now,
        };
        self.webhooks.insert_request(&request).await?;

        self.record(
            &deployment,
            RETRY_DELIVERY_ACTION_TYPE,
            json!({
                "deployment_id": deployment.id.0,
                "namespace": deployment.namespace,
                "webhook_id": webhook_id.0,
                "delivery_id": delivery_id,
                "request_id": request.id,
            }),
        )
        .await?;

        Ok(request)
    }

    async fn webhook_realms(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<String>, CoreError> {
        self.policy
            .can_view_webhooks(identity, organisation_id)
            .await?;
        self.deployment(organisation_id, deployment_id).await?;

        Ok(self
            .webhooks
            .sync_state(deployment_id)
            .await?
            .realms
            .filter(|realms| !realms.is_empty())
            .unwrap_or_else(|| vec![DEFAULT_REALM.to_string()]))
    }

    async fn webhook_deliveries<I>(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        mut query: DeliveryQuery,
        index: I,
    ) -> Result<Vec<WebhookDelivery>, CoreError>
    where
        I: WebhookDeliveryIndex,
    {
        self.policy
            .can_view_webhooks(identity, organisation_id)
            .await?;
        self.deployment(organisation_id, deployment_id).await?;
        self.stored(deployment_id, query.webhook_id).await?;

        query.deployment_id = deployment_id;
        query.limit = query.limit.min(MAX_DELIVERIES);

        index.deliveries(organisation_id, query).await
    }
}

pub struct WebhookOutcomeServiceImpl<D, P, W, V> {
    deployments: D,
    dataplanes: P,
    webhooks: W,
    vault: V,
}

impl<D, P, W, V> WebhookOutcomeServiceImpl<D, P, W, V> {
    pub fn new(deployments: D, dataplanes: P, webhooks: W, vault: V) -> Self {
        Self {
            deployments,
            dataplanes,
            webhooks,
            vault,
        }
    }
}

impl<D, P, W, V> WebhookOutcomeService for WebhookOutcomeServiceImpl<D, P, W, V>
where
    D: DeploymentRepository,
    P: DataPlaneRepository,
    W: WebhookRepository,
    V: SecretVault,
{
    async fn report_webhooks_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: WebhooksOutcome,
    ) -> Result<bool, CoreError> {
        let speaking = speaking_for(&self.dataplanes, &identity).await?;

        let Some(deployment) = self.deployments.get_by_id(deployment_id).await? else {
            return Ok(false);
        };
        speaking.is(deployment.dataplane_id)?;

        let now = Utc::now();
        let mut changed = false;

        let sync = self.webhooks.sync_state(deployment_id).await?;
        if outcome.revision >= sync.applied_revision {
            for report in &outcome.webhooks {
                let Some(stored) = self.webhooks.get(deployment_id, report.id).await? else {
                    continue;
                };
                if outcome.revision < stored.webhook.desired_revision {
                    continue;
                }
                if stored.webhook.state != report.state {
                    self.webhooks
                        .set_state(report.id, &report.state, now)
                        .await?;
                    changed = true;
                }
            }

            let realms = if outcome.realms.is_empty() {
                sync.realms.clone().unwrap_or_default()
            } else {
                outcome.realms.clone()
            };

            if outcome.revision != sync.applied_revision
                || sync.realms.clone().unwrap_or_default() != realms
            {
                self.webhooks
                    .record_applied(deployment_id, outcome.revision, &realms, now)
                    .await?;
                changed = true;
            }
        }

        for report in &outcome.secrets {
            let Some(stored) = self.webhooks.get(deployment_id, report.id).await? else {
                continue;
            };
            if stored
                .webhook
                .secret_version
                .is_some_and(|version| report.version <= version)
            {
                continue;
            }

            let sealed = self
                .vault
                .seal(&secret_context(report.id, report.version), &report.secret)
                .await?;
            let stored_new = self
                .webhooks
                .store_secret(
                    report.id,
                    &SecretRecord {
                        version: report.version,
                        sealed,
                        received_at: now,
                    },
                )
                .await?;
            changed |= stored_new;
        }

        for report in &outcome.requests {
            changed |= self
                .webhooks
                .finish_request(report.id, report.phase, report.message.as_deref(), now)
                .await?;
        }

        Ok(changed)
    }
}
