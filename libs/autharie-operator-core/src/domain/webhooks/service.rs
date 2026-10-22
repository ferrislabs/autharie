use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use autharie_crds::v1alpha::identity_instance::{
    IdentityInstance, IdentityInstanceStatus, IdentityProvider, WebhookPhase, WebhookRequest,
    WebhookRequestKind, WebhookRequestPhase, WebhookRequestStatus, WebhookSpec, WebhookStatus,
};

use super::deliveries::{self, RemoteDelivery};
use super::plan::{Change, DesiredWebhook, plan};
use super::ports::{
    FerriskeyWebhookPort, IdentityInstanceWebhooks, SecretsSnapshot, WebhookDeliveriesStore,
    WebhookOutcome, WebhookSecretsPort, WebhookStatusReport, WebhookStatusRepository,
};
use super::{
    ManagedRemote, WebhookError, WebhookWrite, headers_fingerprint, managed_id, managed_name,
    remote_description, resolve_headers,
};

/// Old results are kept so a request is never run twice, but not for ever.
const MAX_RECORDED_REQUESTS: usize = 100;

pub struct WebhookReconciler<R, F, K, C> {
    repository: Arc<R>,
    ferriskey: Arc<F>,
    secrets: Arc<K>,
    deliveries: Arc<C>,
}

impl<R, F, K, C> WebhookReconciler<R, F, K, C> {
    pub fn new(repository: Arc<R>, ferriskey: Arc<F>, secrets: Arc<K>, deliveries: Arc<C>) -> Self {
        Self {
            repository,
            ferriskey,
            secrets,
            deliveries,
        }
    }
}

enum RequestResult {
    Recorded(WebhookRequestStatus),
    /// Not the request's fault; try it again on the next pass.
    Defer,
}

#[derive(Default)]
struct SyncResult {
    statuses: Vec<WebhookStatus>,
    failed_deletes: BTreeSet<String>,
    retry: bool,
}

impl<R, F, K, C> IdentityInstanceWebhooks for WebhookReconciler<R, F, K, C>
where
    R: WebhookStatusRepository,
    F: FerriskeyWebhookPort,
    K: WebhookSecretsPort,
    C: WebhookDeliveriesStore,
{
    async fn reconcile(&self, instance: &IdentityInstance) -> WebhookOutcome {
        if instance.spec.provider != IdentityProvider::Ferriskey
            || instance.spec.stopped
            || !instance.is_ready()
        {
            return WebhookOutcome::Settled;
        }

        let empty = IdentityInstanceStatus::default();
        let previous = instance.status.as_ref().unwrap_or(&empty);
        let spec = &instance.spec;
        let mut retry = false;

        let snapshot = match self.secrets.load(instance).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(error = %error.message(), "could not read the webhook secrets");
                return WebhookOutcome::Retry;
            }
        };

        let realms = match self.ferriskey.list_realms(instance).await {
            Ok(mut realms) => {
                realms.sort();
                realms.dedup();
                realms
            }
            Err(error) => {
                tracing::warn!(error = %error.message(), "could not list the realms");
                retry = true;
                previous.realms.clone()
            }
        };

        let mut synced = if spec.webhooks.is_empty() && previous.webhooks.is_empty() {
            SyncResult::default()
        } else {
            self.sync(instance, previous, &snapshot, &realms).await
        };
        retry |= synced.retry;

        self.forget_removed(instance, &snapshot, &synced.failed_deletes)
            .await;

        let mut recorded = previous.webhook_requests.clone();
        for request in &spec.webhook_requests {
            if recorded.iter().any(|done| done.id == request.id) {
                continue;
            }
            match self
                .run_request(instance, request, &mut synced.statuses)
                .await
            {
                RequestResult::Recorded(status) => recorded.push(status),
                RequestResult::Defer => retry = true,
            }
        }
        trim_recorded(&mut recorded, &spec.webhook_requests);

        self.publish_deliveries(instance, &synced.statuses).await;

        let report = WebhookStatusReport {
            webhooks: synced.statuses,
            realms,
            webhook_requests: recorded,
        };
        let unchanged = report.webhooks == previous.webhooks
            && report.realms == previous.realms
            && report.webhook_requests == previous.webhook_requests;
        if !unchanged
            && let Err(error) = self
                .repository
                .patch_webhook_status(instance, &report)
                .await
        {
            tracing::warn!(error = %error, "could not report the webhook status");
            retry = true;
        }

        if retry {
            WebhookOutcome::Retry
        } else {
            WebhookOutcome::Settled
        }
    }
}

impl<R, F, K, C> WebhookReconciler<R, F, K, C>
where
    F: FerriskeyWebhookPort,
    K: WebhookSecretsPort,
    C: WebhookDeliveriesStore,
{
    /// Failing to read deliveries is not a webhook failure: the document
    /// stays as it was and the next pass tries again.
    async fn publish_deliveries(&self, instance: &IdentityInstance, statuses: &[WebhookStatus]) {
        let mut gathered: Vec<(&str, Vec<RemoteDelivery>)> = Vec::new();
        for status in statuses {
            let Some(ferriskey_id) = status.ferriskey_id.as_deref() else {
                continue;
            };
            let Some(webhook) = instance
                .spec
                .webhooks
                .iter()
                .find(|webhook| webhook.id == status.id)
            else {
                continue;
            };
            match self
                .ferriskey
                .list_deliveries(instance, &webhook.realm, ferriskey_id)
                .await
            {
                Ok(found) => gathered.push((webhook.id.as_str(), found)),
                Err(error) => {
                    tracing::warn!(webhook = %webhook.id, error = %error.message(), "could not read deliveries; keeping the previous ones");
                    return;
                }
            }
        }

        let rendered = deliveries::render(&deliveries::collect(gathered));
        let current = match self.deliveries.current(instance).await {
            Ok(current) => current,
            Err(error) => {
                tracing::warn!(error = %error.message(), "could not read the published deliveries");
                return;
            }
        };
        if deliveries::needs_write(current.as_deref(), &rendered)
            && let Err(error) = self.deliveries.publish(instance, &rendered).await
        {
            tracing::warn!(error = %error.message(), "could not publish the deliveries");
        }
    }

    async fn sync(
        &self,
        instance: &IdentityInstance,
        previous: &IdentityInstanceStatus,
        snapshot: &SecretsSnapshot,
        realms: &[String],
    ) -> SyncResult {
        let spec = &instance.spec;
        let mut scan: BTreeSet<&str> = realms.iter().map(String::as_str).collect();
        scan.extend(spec.webhooks.iter().map(|webhook| webhook.realm.as_str()));

        let mut remote = Vec::new();
        let mut unlistable: BTreeMap<&str, String> = BTreeMap::new();
        for realm in scan {
            match self.ferriskey.list_managed_webhooks(instance, realm).await {
                Ok(found) => {
                    for webhook in found {
                        let Some(id) = webhook.name.as_deref().and_then(managed_id) else {
                            continue;
                        };
                        remote.push(ManagedRemote {
                            realm: realm.to_string(),
                            autharie_id: id.to_string(),
                            webhook,
                        });
                    }
                }
                Err(error) => {
                    tracing::warn!(realm, error = %error.message(), "could not list the webhooks of a realm");
                    unlistable.insert(realm, error.message().to_string());
                }
            }
        }

        let resolved: Vec<_> = spec
            .webhooks
            .iter()
            .map(|webhook| {
                let blocked_by_realm = unlistable.get(webhook.realm.as_str()).map(|reason| {
                    format!(
                        "the webhooks of realm {} could not be read: {reason}",
                        webhook.realm
                    )
                });
                match resolve_headers(webhook, &snapshot.headers) {
                    Ok(headers) => {
                        let fingerprint = headers_fingerprint(&headers);
                        let changed = snapshot.applied.get(&webhook.id).map(String::as_str)
                            != Some(fingerprint.as_str());
                        (blocked_by_realm, Some((headers, fingerprint)), changed)
                    }
                    Err(missing) => (
                        blocked_by_realm.or_else(|| {
                            Some(format!(
                                "the value of header {} is not available yet",
                                missing.join(", ")
                            ))
                        }),
                        None,
                        false,
                    ),
                }
            })
            .collect();

        let desired: Vec<_> = spec
            .webhooks
            .iter()
            .zip(&resolved)
            .map(|(webhook, (blocked, _, changed))| DesiredWebhook {
                spec: webhook,
                blocked: blocked.clone(),
                headers_changed: *changed,
            })
            .collect();
        let plan = plan(&desired, &remote);

        let mut result = SyncResult::default();
        for ((webhook, change), (_, headers, _)) in
            spec.webhooks.iter().zip(&plan.changes).zip(&resolved)
        {
            let before = previous
                .webhooks
                .iter()
                .find(|status| status.id == webhook.id);
            let status = self
                .apply(
                    instance,
                    webhook,
                    change,
                    headers.as_ref(),
                    snapshot,
                    before,
                    &mut result.retry,
                )
                .await;
            if status.phase == WebhookPhase::Failed {
                result.retry = true;
            }
            result.statuses.push(status);
        }

        for stale in &plan.deletes {
            match self
                .ferriskey
                .delete_webhook(instance, &stale.realm, &stale.webhook.ferriskey_id)
                .await
            {
                Ok(()) => {}
                Err(error) => {
                    tracing::warn!(
                        webhook = %stale.autharie_id,
                        error = %error.message(),
                        "could not delete a webhook that is no longer wanted"
                    );
                    result.failed_deletes.insert(stale.autharie_id.clone());
                    result.retry = true;
                }
            }
        }

        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply(
        &self,
        instance: &IdentityInstance,
        webhook: &WebhookSpec,
        change: &Change<'_>,
        headers: Option<&(BTreeMap<String, String>, String)>,
        snapshot: &SecretsSnapshot,
        before: Option<&WebhookStatus>,
        retry: &mut bool,
    ) -> WebhookStatus {
        let version = before.map_or(0, |status| status.secret_version);
        let failed = |ferriskey_id: Option<String>, message: String| WebhookStatus {
            id: webhook.id.clone(),
            ferriskey_id,
            phase: WebhookPhase::Failed,
            message: Some(message),
            secret_version: version,
        };

        let write = |send_headers: bool| WebhookWrite {
            name: managed_name(&webhook.id),
            description: remote_description(webhook),
            endpoint: webhook.endpoint.clone(),
            events: webhook.events.clone(),
            headers: headers
                .filter(|_| send_headers)
                .map(|(values, _)| values.clone()),
        };

        let (ferriskey_id, needs_secret) = match change {
            Change::Blocked(reason) => return failed(None, reason.clone()),
            Change::Create => {
                match self
                    .ferriskey
                    .create_webhook(instance, &webhook.realm, &write(true))
                    .await
                {
                    Ok(created) => {
                        self.remember_headers(instance, webhook, headers, retry)
                            .await;
                        (created.ferriskey_id, true)
                    }
                    Err(error) => return failed(None, error.message().to_string()),
                }
            }
            Change::Update {
                ferriskey_id,
                send_headers,
            } => {
                if let Err(error) = self
                    .ferriskey
                    .update_webhook(
                        instance,
                        &webhook.realm,
                        ferriskey_id,
                        &write(*send_headers),
                    )
                    .await
                {
                    return failed(
                        Some((*ferriskey_id).to_string()),
                        error.message().to_string(),
                    );
                }
                if *send_headers {
                    self.remember_headers(instance, webhook, headers, retry)
                        .await;
                }
                (
                    (*ferriskey_id).to_string(),
                    version == 0 || !snapshot.secret_ids.contains(&webhook.id),
                )
            }
            Change::Keep { ferriskey_id } => (
                (*ferriskey_id).to_string(),
                version == 0 || !snapshot.secret_ids.contains(&webhook.id),
            ),
        };

        let secret_version = if needs_secret {
            match self
                .obtain_secret(instance, webhook, &ferriskey_id, version)
                .await
            {
                Ok(next) => next,
                Err(error) => return failed(Some(ferriskey_id), error.message().to_string()),
            }
        } else {
            version
        };

        WebhookStatus {
            id: webhook.id.clone(),
            ferriskey_id: Some(ferriskey_id),
            phase: WebhookPhase::Active,
            message: None,
            secret_version,
        }
    }

    async fn remember_headers(
        &self,
        instance: &IdentityInstance,
        webhook: &WebhookSpec,
        headers: Option<&(BTreeMap<String, String>, String)>,
        retry: &mut bool,
    ) {
        let Some((_, fingerprint)) = headers else {
            return;
        };
        if let Err(error) = self
            .secrets
            .put_applied(instance, &webhook.id, fingerprint)
            .await
        {
            tracing::warn!(webhook = %webhook.id, error = %error.message(), "could not note which headers were sent");
            *retry = true;
        }
    }

    /// Rotates, keeps the secret where Genesis reads it, and returns the new
    /// version. FerrisKey shows a secret once, so it is stored before anything
    /// else is allowed to fail.
    async fn obtain_secret(
        &self,
        instance: &IdentityInstance,
        webhook: &WebhookSpec,
        ferriskey_id: &str,
        version: u64,
    ) -> Result<u64, WebhookError> {
        let secret = self
            .ferriskey
            .rotate_secret(instance, &webhook.realm, ferriskey_id)
            .await?;
        self.secrets
            .put_secret(instance, &webhook.id, &secret)
            .await?;
        Ok(version + 1)
    }

    async fn forget_removed(
        &self,
        instance: &IdentityInstance,
        snapshot: &SecretsSnapshot,
        failed_deletes: &BTreeSet<String>,
    ) {
        let wanted: BTreeSet<&str> = instance
            .spec
            .webhooks
            .iter()
            .map(|webhook| webhook.id.as_str())
            .collect();
        let stale: Vec<String> = snapshot
            .secret_ids
            .iter()
            .chain(snapshot.applied.keys())
            .filter(|id| !wanted.contains(id.as_str()) && !failed_deletes.contains(*id))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if stale.is_empty() {
            return;
        }
        if let Err(error) = self.secrets.forget(instance, &stale).await {
            tracing::warn!(error = %error.message(), "could not drop the secrets of removed webhooks");
        }
    }

    async fn run_request(
        &self,
        instance: &IdentityInstance,
        request: &WebhookRequest,
        statuses: &mut [WebhookStatus],
    ) -> RequestResult {
        let record = |phase: WebhookRequestPhase, message: Option<String>| {
            RequestResult::Recorded(WebhookRequestStatus {
                id: request.id.clone(),
                phase,
                message,
            })
        };
        let refuse = |message: String| record(WebhookRequestPhase::Failed, Some(message));

        let Some(webhook) = instance
            .spec
            .webhooks
            .iter()
            .find(|webhook| webhook.id == request.webhook_id)
        else {
            return refuse("the webhook is not part of this deployment".to_string());
        };
        let Some(status) = statuses.iter_mut().find(|status| status.id == webhook.id) else {
            return refuse("the webhook is not part of this deployment".to_string());
        };
        let (WebhookPhase::Active, Some(ferriskey_id)) =
            (status.phase, status.ferriskey_id.clone())
        else {
            return refuse(match status.message.as_deref() {
                Some(reason) => format!("the webhook is not active: {reason}"),
                None => "the webhook is not active".to_string(),
            });
        };

        let outcome = match request.kind {
            WebhookRequestKind::RotateSecret => self
                .obtain_secret(instance, webhook, &ferriskey_id, status.secret_version)
                .await
                .map(|next| status.secret_version = next),
            WebhookRequestKind::RetryDelivery => match request.delivery_id.as_deref() {
                Some(delivery_id) => {
                    self.ferriskey
                        .retry_delivery(instance, &webhook.realm, &ferriskey_id, delivery_id)
                        .await
                }
                None => return refuse("a retry needs the delivery to retry".to_string()),
            },
        };

        match outcome {
            Ok(()) => record(WebhookRequestPhase::Done, None),
            Err(error) if error.is_retry() => RequestResult::Defer,
            Err(error) => refuse(error.message().to_string()),
        }
    }
}

fn trim_recorded(recorded: &mut Vec<WebhookRequestStatus>, asked: &[WebhookRequest]) {
    while recorded.len() > MAX_RECORDED_REQUESTS {
        match recorded
            .iter()
            .position(|done| !asked.iter().any(|request| request.id == done.id))
        {
            Some(oldest) => {
                recorded.remove(oldest);
            }
            None => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use autharie_crds::common::types::{Phase, ResourceRequirements};
    use autharie_crds::v1alpha::identity_instance::{
        DatabaseConfig, DatabaseMode, IdentityInstanceSpec, ManagedClusterConfig,
        ManagedClusterStorage,
    };
    use kube::core::ObjectMeta;

    use super::*;
    use crate::domain::OperatorError;
    use crate::domain::webhooks::RemoteWebhook;

    const ID_A: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";
    const ID_B: &str = "7f1c2d3e-0000-4000-8000-00000000000b";

    #[derive(Default)]
    struct FerriskeyState {
        realms: Vec<String>,
        hooks: Vec<(String, RemoteWebhook)>,
        calls: Vec<String>,
        counter: u32,
        failures: BTreeMap<String, WebhookError>,
        sent_headers: Vec<Option<BTreeMap<String, String>>>,
        deliveries: BTreeMap<String, Vec<RemoteDelivery>>,
    }

    #[derive(Default)]
    struct FakeFerriskey {
        state: Mutex<FerriskeyState>,
    }

    impl FakeFerriskey {
        fn with_realms(realms: &[&str]) -> Self {
            let fake = Self::default();
            fake.state.lock().unwrap().realms = realms.iter().map(ToString::to_string).collect();
            fake
        }

        fn insert(&self, realm: &str, id: &str, name: &str) {
            self.state.lock().unwrap().hooks.push((
                realm.to_string(),
                RemoteWebhook {
                    ferriskey_id: id.to_string(),
                    name: Some(name.to_string()),
                    description: Some("hand made".to_string()),
                    endpoint: "https://theirs.example.com".to_string(),
                    events: vec!["user.created".to_string()],
                },
            ));
        }

        fn fail(&self, operation: &str, error: WebhookError) {
            self.state
                .lock()
                .unwrap()
                .failures
                .insert(operation.to_string(), error);
        }

        fn heal(&self, operation: &str) {
            self.state.lock().unwrap().failures.remove(operation);
        }

        fn calls(&self) -> Vec<String> {
            self.state.lock().unwrap().calls.clone()
        }

        fn clear_calls(&self) {
            self.state.lock().unwrap().calls.clear();
        }

        fn hooks(&self) -> Vec<(String, RemoteWebhook)> {
            self.state.lock().unwrap().hooks.clone()
        }

        fn enter(&self, call: String, operation: &str) -> Result<(), WebhookError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(call);
            match state.failures.get(operation) {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }
    }

    impl FerriskeyWebhookPort for FakeFerriskey {
        async fn list_realms(&self, _: &IdentityInstance) -> Result<Vec<String>, WebhookError> {
            self.enter("list_realms".to_string(), "list_realms")?;
            Ok(self.state.lock().unwrap().realms.clone())
        }

        async fn list_managed_webhooks(
            &self,
            _: &IdentityInstance,
            realm: &str,
        ) -> Result<Vec<RemoteWebhook>, WebhookError> {
            self.enter(format!("list {realm}"), &format!("list {realm}"))?;
            Ok(self
                .state
                .lock()
                .unwrap()
                .hooks
                .iter()
                .filter(|(found, _)| found == realm)
                .map(|(_, hook)| hook.clone())
                .collect())
        }

        async fn create_webhook(
            &self,
            _: &IdentityInstance,
            realm: &str,
            write: &WebhookWrite,
        ) -> Result<RemoteWebhook, WebhookError> {
            self.enter(format!("create {realm}"), "create")?;
            let mut state = self.state.lock().unwrap();
            state.counter += 1;
            let hook = RemoteWebhook {
                ferriskey_id: format!("fk-{}", state.counter),
                name: Some(write.name.clone()),
                description: Some(write.description.clone()),
                endpoint: write.endpoint.clone(),
                events: write.events.clone(),
            };
            state.sent_headers.push(write.headers.clone());
            state.hooks.push((realm.to_string(), hook.clone()));
            Ok(hook)
        }

        async fn update_webhook(
            &self,
            _: &IdentityInstance,
            realm: &str,
            ferriskey_id: &str,
            write: &WebhookWrite,
        ) -> Result<(), WebhookError> {
            self.enter(format!("update {realm} {ferriskey_id}"), "update")?;
            let mut state = self.state.lock().unwrap();
            state.sent_headers.push(write.headers.clone());
            let hook = state
                .hooks
                .iter_mut()
                .find(|(found, hook)| found == realm && hook.ferriskey_id == ferriskey_id)
                .map(|(_, hook)| hook)
                .expect("update of a webhook that exists");
            hook.endpoint = write.endpoint.clone();
            hook.events = write.events.clone();
            hook.description = Some(write.description.clone());
            Ok(())
        }

        async fn delete_webhook(
            &self,
            _: &IdentityInstance,
            realm: &str,
            ferriskey_id: &str,
        ) -> Result<(), WebhookError> {
            self.enter(format!("delete {realm} {ferriskey_id}"), "delete")?;
            self.state
                .lock()
                .unwrap()
                .hooks
                .retain(|(found, hook)| !(found == realm && hook.ferriskey_id == ferriskey_id));
            Ok(())
        }

        async fn rotate_secret(
            &self,
            _: &IdentityInstance,
            realm: &str,
            ferriskey_id: &str,
        ) -> Result<String, WebhookError> {
            self.enter(format!("rotate {realm} {ferriskey_id}"), "rotate")?;
            let mut state = self.state.lock().unwrap();
            state.counter += 1;
            Ok(format!("whsec-{}", state.counter))
        }

        async fn list_deliveries(
            &self,
            _: &IdentityInstance,
            _: &str,
            ferriskey_id: &str,
        ) -> Result<Vec<RemoteDelivery>, WebhookError> {
            self.enter(format!("deliveries {ferriskey_id}"), "deliveries")?;
            Ok(self
                .state
                .lock()
                .unwrap()
                .deliveries
                .get(ferriskey_id)
                .cloned()
                .unwrap_or_default())
        }

        async fn retry_delivery(
            &self,
            _: &IdentityInstance,
            realm: &str,
            ferriskey_id: &str,
            delivery_id: &str,
        ) -> Result<(), WebhookError> {
            self.enter(
                format!("retry {realm} {ferriskey_id} {delivery_id}"),
                "retry",
            )
        }
    }

    #[derive(Default)]
    struct FakeSecrets {
        state: Mutex<SecretsState>,
    }

    #[derive(Default)]
    struct SecretsState {
        headers: BTreeMap<String, String>,
        applied: BTreeMap<String, String>,
        secrets: BTreeMap<String, String>,
        fail_put_secret: bool,
    }

    impl FakeSecrets {
        fn with_headers(headers: &[(&str, &str)]) -> Self {
            let fake = Self::default();
            fake.state.lock().unwrap().headers = headers
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect();
            fake
        }

        fn set_header(&self, key: &str, value: &str) {
            self.state
                .lock()
                .unwrap()
                .headers
                .insert(key.to_string(), value.to_string());
        }

        fn secret(&self, id: &str) -> Option<String> {
            self.state.lock().unwrap().secrets.get(id).cloned()
        }

        fn drop_secret(&self, id: &str) {
            self.state.lock().unwrap().secrets.remove(id);
        }

        fn fail_put_secret(&self, fail: bool) {
            self.state.lock().unwrap().fail_put_secret = fail;
        }

        fn applied_ids(&self) -> Vec<String> {
            self.state.lock().unwrap().applied.keys().cloned().collect()
        }
    }

    impl WebhookSecretsPort for FakeSecrets {
        async fn load(&self, _: &IdentityInstance) -> Result<SecretsSnapshot, WebhookError> {
            let state = self.state.lock().unwrap();
            Ok(SecretsSnapshot {
                headers: state.headers.clone(),
                applied: state.applied.clone(),
                secret_ids: state.secrets.keys().cloned().collect(),
            })
        }

        async fn put_secret(
            &self,
            _: &IdentityInstance,
            webhook_id: &str,
            secret: &str,
        ) -> Result<(), WebhookError> {
            let mut state = self.state.lock().unwrap();
            if state.fail_put_secret {
                return Err(WebhookError::Retry {
                    message: "the secret could not be written".to_string(),
                });
            }
            state
                .secrets
                .insert(webhook_id.to_string(), secret.to_string());
            Ok(())
        }

        async fn put_applied(
            &self,
            _: &IdentityInstance,
            webhook_id: &str,
            fingerprint: &str,
        ) -> Result<(), WebhookError> {
            self.state
                .lock()
                .unwrap()
                .applied
                .insert(webhook_id.to_string(), fingerprint.to_string());
            Ok(())
        }

        async fn forget(
            &self,
            _: &IdentityInstance,
            webhook_ids: &[String],
        ) -> Result<(), WebhookError> {
            let mut state = self.state.lock().unwrap();
            for id in webhook_ids {
                state.secrets.remove(id);
                state.applied.remove(id);
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeDeliveries {
        document: Mutex<Option<String>>,
        writes: Mutex<u32>,
    }

    impl WebhookDeliveriesStore for FakeDeliveries {
        async fn current(&self, _: &IdentityInstance) -> Result<Option<String>, WebhookError> {
            Ok(self.document.lock().unwrap().clone())
        }

        async fn publish(&self, _: &IdentityInstance, document: &str) -> Result<(), WebhookError> {
            *self.document.lock().unwrap() = Some(document.to_string());
            *self.writes.lock().unwrap() += 1;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeRepository {
        reports: Mutex<Vec<WebhookStatusReport>>,
        fail: Mutex<bool>,
    }

    impl FakeRepository {
        fn reports(&self) -> Vec<WebhookStatusReport> {
            self.reports.lock().unwrap().clone()
        }

        fn last(&self) -> WebhookStatusReport {
            self.reports()
                .last()
                .cloned()
                .expect("a report was written")
        }
    }

    impl WebhookStatusRepository for FakeRepository {
        async fn patch_webhook_status(
            &self,
            _: &IdentityInstance,
            report: &WebhookStatusReport,
        ) -> Result<(), OperatorError> {
            if *self.fail.lock().unwrap() {
                return Err(OperatorError::Kube {
                    message: "conflict".to_string(),
                });
            }
            self.reports.lock().unwrap().push(report.clone());
            Ok(())
        }
    }

    struct Harness {
        repository: Arc<FakeRepository>,
        ferriskey: Arc<FakeFerriskey>,
        secrets: Arc<FakeSecrets>,
        deliveries: Arc<FakeDeliveries>,
        reconciler: WebhookReconciler<FakeRepository, FakeFerriskey, FakeSecrets, FakeDeliveries>,
    }

    fn harness(ferriskey: FakeFerriskey, secrets: FakeSecrets) -> Harness {
        let repository = Arc::new(FakeRepository::default());
        let ferriskey = Arc::new(ferriskey);
        let secrets = Arc::new(secrets);
        let deliveries = Arc::new(FakeDeliveries::default());
        Harness {
            reconciler: WebhookReconciler::new(
                repository.clone(),
                ferriskey.clone(),
                secrets.clone(),
                deliveries.clone(),
            ),
            deliveries,
            repository,
            ferriskey,
            secrets,
        }
    }

    fn webhook(id: &str, realm: &str) -> WebhookSpec {
        WebhookSpec {
            id: id.to_string(),
            realm: realm.to_string(),
            name: "Audit".to_string(),
            description: Some("to the SIEM".to_string()),
            endpoint: "https://example.com/hook".to_string(),
            events: vec!["user.created".to_string()],
            header_names: Vec::new(),
        }
    }

    fn request(id: &str, kind: WebhookRequestKind, webhook: &str) -> WebhookRequest {
        WebhookRequest {
            id: id.to_string(),
            kind,
            webhook_id: webhook.to_string(),
            delivery_id: None,
        }
    }

    fn instance(webhooks: Vec<WebhookSpec>, requests: Vec<WebhookRequest>) -> IdentityInstance {
        IdentityInstance {
            metadata: ObjectMeta {
                name: Some("acme".to_string()),
                namespace: Some("tenant-a".to_string()),
                ..Default::default()
            },
            spec: IdentityInstanceSpec {
                agents: None,
                restore: None,
                iam: None,
                organisation_id: "org".to_string(),
                provider: IdentityProvider::Ferriskey,
                version: "1.0.0".to_string(),
                hostname: "auth.acme.test".to_string(),
                replicas: 1,
                stopped: false,
                database: DatabaseConfig {
                    mode: DatabaseMode::ManagedCluster,
                    shared: None,
                    managed_cluster: ManagedClusterConfig {
                        instances: 1,
                        storage: ManagedClusterStorage {
                            size: "1Gi".to_string(),
                            storage_class: None,
                        },
                        resources: ResourceRequirements {
                            requests: None,
                            limits: None,
                        },
                    },
                },
                ferriskey: None,
                ingress: None,
                allowed_cidrs: None,
                backup: None,
                webhooks,
                webhooks_revision: 1,
                webhook_requests: requests,
            },
            status: Some(IdentityInstanceStatus {
                phase: Some(Phase::Running),
                ready: true,
                ..Default::default()
            }),
        }
    }

    fn after(mut instance: IdentityInstance, report: &WebhookStatusReport) -> IdentityInstance {
        let status = instance.status.get_or_insert_with(Default::default);
        status.webhooks = report.webhooks.clone();
        status.realms = report.realms.clone();
        status.webhook_requests = report.webhook_requests.clone();
        instance
    }

    fn active(id: &str, ferriskey_id: &str, version: u64) -> WebhookStatus {
        WebhookStatus {
            id: id.to_string(),
            ferriskey_id: Some(ferriskey_id.to_string()),
            phase: WebhookPhase::Active,
            message: None,
            secret_version: version,
        }
    }

    #[tokio::test]
    async fn a_new_webhook_is_created_then_rotated_and_its_secret_stored() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let instance = instance(vec![webhook(ID_A, "master")], vec![]);

        let outcome = h.reconciler.reconcile(&instance).await;

        assert_eq!(outcome, WebhookOutcome::Settled);
        assert_eq!(
            h.ferriskey.calls(),
            vec![
                "list_realms",
                "list master",
                "create master",
                "rotate master fk-1",
                "deliveries fk-1"
            ]
        );
        let hooks = h.ferriskey.hooks();
        assert_eq!(
            hooks[0].1.name.as_deref(),
            Some(&*format!("autharie:{ID_A}"))
        );
        assert_eq!(
            hooks[0].1.description.as_deref(),
            Some("Audit — to the SIEM")
        );
        assert_eq!(h.secrets.secret(ID_A).as_deref(), Some("whsec-2"));
        let report = h.repository.last();
        assert_eq!(report.webhooks, vec![active(ID_A, "fk-1", 1)]);
        assert_eq!(report.realms, vec!["master"]);
        assert!(!format!("{report:?}").contains("whsec-2"));
    }

    #[tokio::test]
    async fn a_settled_deployment_makes_no_change_and_no_patch() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let settled = after(first, &h.repository.last());
        h.ferriskey.clear_calls();
        let patches = h.repository.reports().len();

        let outcome = h.reconciler.reconcile(&settled).await;

        assert_eq!(outcome, WebhookOutcome::Settled);
        assert_eq!(
            h.ferriskey.calls(),
            vec!["list_realms", "list master", "deliveries fk-1"]
        );
        assert_eq!(h.repository.reports().len(), patches);
    }

    #[tokio::test]
    async fn webhooks_that_are_not_ours_are_never_touched() {
        let ferriskey = FakeFerriskey::with_realms(&["master"]);
        ferriskey.insert("master", "theirs-1", "Customer hook");
        ferriskey.insert("master", "theirs-2", "autharie:not-a-uuid");
        ferriskey.insert("master", "theirs-3", &format!("Autharie:{ID_B}"));
        let h = harness(ferriskey, FakeSecrets::default());
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let removed = after(instance(vec![], vec![]), &h.repository.last());

        h.reconciler.reconcile(&removed).await;

        let names: Vec<_> = h
            .ferriskey
            .hooks()
            .into_iter()
            .map(|(_, hook)| hook.ferriskey_id)
            .collect();
        assert_eq!(names, vec!["theirs-1", "theirs-2", "theirs-3"]);
        assert!(
            h.ferriskey
                .calls()
                .iter()
                .all(|call| !call.contains("theirs"))
        );
    }

    #[tokio::test]
    async fn a_webhook_left_out_of_the_spec_is_deleted_and_its_secret_dropped() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(
            vec![webhook(ID_A, "master"), webhook(ID_B, "master")],
            vec![],
        );
        h.reconciler.reconcile(&first).await;
        let both = after(first, &h.repository.last());

        let outcome = h
            .reconciler
            .reconcile(&{
                let mut next = both;
                next.spec.webhooks.retain(|webhook| webhook.id == ID_A);
                next
            })
            .await;

        assert_eq!(outcome, WebhookOutcome::Settled);
        assert_eq!(h.ferriskey.hooks().len(), 1);
        assert_eq!(h.secrets.secret(ID_B), None);
        assert!(h.secrets.secret(ID_A).is_some());
        assert_eq!(h.repository.last().webhooks.len(), 1);
    }

    #[tokio::test]
    async fn an_emptied_spec_clears_the_reported_webhooks() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let created = after(first, &h.repository.last());

        h.reconciler
            .reconcile(&{
                let mut next = created;
                next.spec.webhooks.clear();
                next
            })
            .await;

        assert!(h.ferriskey.hooks().is_empty());
        assert!(h.repository.last().webhooks.is_empty());
        assert!(h.secrets.applied_ids().is_empty());
    }

    #[tokio::test]
    async fn a_changed_endpoint_updates_without_resending_headers() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let created = after(first, &h.repository.last());
        h.ferriskey.clear_calls();

        let mut changed = created;
        changed.spec.webhooks[0].endpoint = "https://example.com/new".to_string();
        h.reconciler.reconcile(&changed).await;

        assert_eq!(
            h.ferriskey.calls(),
            vec![
                "list_realms",
                "list master",
                "update master fk-1",
                "deliveries fk-1"
            ]
        );
        let state = h.ferriskey.state.lock().unwrap();
        assert_eq!(state.sent_headers.last().unwrap(), &None);
        assert_eq!(state.hooks[0].1.endpoint, "https://example.com/new");
    }

    #[tokio::test]
    async fn header_values_are_sent_on_creation_and_again_only_when_they_change() {
        let secrets =
            FakeSecrets::with_headers(&[(&format!("{ID_A}.Authorization"), "Bearer one")]);
        let h = harness(FakeFerriskey::with_realms(&["master"]), secrets);
        let mut spec = webhook(ID_A, "master");
        spec.header_names = vec!["Authorization".to_string()];
        let first = instance(vec![spec], vec![]);
        h.reconciler.reconcile(&first).await;
        let created = after(first, &h.repository.last());
        assert_eq!(
            h.ferriskey.state.lock().unwrap().sent_headers[0],
            Some(BTreeMap::from([(
                "Authorization".to_string(),
                "Bearer one".to_string()
            )]))
        );

        h.ferriskey.clear_calls();
        h.reconciler.reconcile(&created).await;
        assert_eq!(
            h.ferriskey.calls(),
            vec!["list_realms", "list master", "deliveries fk-1"]
        );

        h.secrets
            .set_header(&format!("{ID_A}.Authorization"), "Bearer two");
        h.reconciler.reconcile(&created).await;
        let state = h.ferriskey.state.lock().unwrap();
        assert!(state.calls.contains(&"update master fk-1".to_string()));
        assert_eq!(
            state.sent_headers.last().unwrap().as_ref().unwrap()["Authorization"],
            "Bearer two"
        );
    }

    #[tokio::test]
    async fn a_missing_header_value_fails_the_webhook_without_calling_ferriskey() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let mut spec = webhook(ID_A, "master");
        spec.header_names = vec!["Authorization".to_string()];

        let outcome = h.reconciler.reconcile(&instance(vec![spec], vec![])).await;

        assert_eq!(outcome, WebhookOutcome::Retry);
        assert!(h.ferriskey.hooks().is_empty());
        let status = &h.repository.last().webhooks[0];
        assert_eq!(status.phase, WebhookPhase::Failed);
        assert!(status.message.as_deref().unwrap().contains("Authorization"));
        assert!(
            !h.ferriskey
                .calls()
                .iter()
                .any(|call| call.starts_with("create"))
        );
    }

    #[tokio::test]
    async fn a_moved_webhook_is_recreated_in_the_new_realm_with_a_newer_secret() {
        let h = harness(
            FakeFerriskey::with_realms(&["master", "acme"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let created = after(first, &h.repository.last());

        let mut moved = created;
        moved.spec.webhooks[0].realm = "acme".to_string();
        h.reconciler.reconcile(&moved).await;

        let hooks = h.ferriskey.hooks();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].0, "acme");
        let status = &h.repository.last().webhooks[0];
        assert_eq!(status.secret_version, 2);
        assert_eq!(
            status.ferriskey_id.as_deref(),
            Some(hooks[0].1.ferriskey_id.as_str())
        );
    }

    #[tokio::test]
    async fn a_failed_create_is_reported_and_not_repeated_in_the_status() {
        let ferriskey = FakeFerriskey::with_realms(&["master"]);
        ferriskey.fail(
            "create",
            WebhookError::Rejected {
                message: "FerrisKey answered 400: endpoint refused".to_string(),
            },
        );
        let h = harness(ferriskey, FakeSecrets::default());
        let first = instance(vec![webhook(ID_A, "master")], vec![]);

        let outcome = h.reconciler.reconcile(&first).await;

        assert_eq!(outcome, WebhookOutcome::Retry);
        let report = h.repository.last();
        assert_eq!(report.webhooks[0].phase, WebhookPhase::Failed);
        assert_eq!(
            report.webhooks[0].message.as_deref(),
            Some("FerrisKey answered 400: endpoint refused")
        );
        let failed = after(first, &report);
        let patches = h.repository.reports().len();

        let again = h.reconciler.reconcile(&failed).await;

        assert_eq!(again, WebhookOutcome::Retry);
        assert_eq!(h.repository.reports().len(), patches);
    }

    #[tokio::test]
    async fn a_create_whose_rotation_failed_rotates_later_instead_of_creating_twice() {
        let ferriskey = FakeFerriskey::with_realms(&["master"]);
        ferriskey.fail(
            "rotate",
            WebhookError::Retry {
                message: "FerrisKey is unreachable".to_string(),
            },
        );
        let h = harness(ferriskey, FakeSecrets::default());
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let report = h.repository.last();
        assert_eq!(report.webhooks[0].phase, WebhookPhase::Failed);
        assert_eq!(report.webhooks[0].ferriskey_id.as_deref(), Some("fk-1"));
        let failed = after(first, &report);
        h.ferriskey.heal("rotate");

        let outcome = h.reconciler.reconcile(&failed).await;

        assert_eq!(outcome, WebhookOutcome::Settled);
        assert_eq!(h.ferriskey.hooks().len(), 1);
        assert_eq!(h.repository.last().webhooks, vec![active(ID_A, "fk-1", 1)]);
        assert!(h.secrets.secret(ID_A).is_some());
    }

    #[tokio::test]
    async fn a_secret_that_vanished_is_replaced_with_a_newer_version() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let created = after(first, &h.repository.last());
        h.secrets.drop_secret(ID_A);

        h.reconciler.reconcile(&created).await;

        assert_eq!(h.repository.last().webhooks[0].secret_version, 2);
        assert!(h.secrets.secret(ID_A).is_some());
    }

    #[tokio::test]
    async fn a_realm_that_cannot_be_listed_blocks_its_webhooks_so_nothing_is_duplicated() {
        let ferriskey = FakeFerriskey::with_realms(&["master"]);
        ferriskey.fail(
            "list master",
            WebhookError::Retry {
                message: "FerrisKey is unreachable".to_string(),
            },
        );
        let h = harness(ferriskey, FakeSecrets::default());

        let outcome = h
            .reconciler
            .reconcile(&instance(vec![webhook(ID_A, "master")], vec![]))
            .await;

        assert_eq!(outcome, WebhookOutcome::Retry);
        assert!(
            !h.ferriskey
                .calls()
                .iter()
                .any(|call| call.starts_with("create"))
        );
        assert_eq!(h.repository.last().webhooks[0].phase, WebhookPhase::Failed);
    }

    #[tokio::test]
    async fn a_rotate_request_runs_once_bumps_the_version_and_rewrites_the_secret() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let created = after(first, &h.repository.last());
        let before = h.secrets.secret(ID_A);
        let mut asked = created;
        asked.spec.webhook_requests = vec![request("r1", WebhookRequestKind::RotateSecret, ID_A)];

        h.reconciler.reconcile(&asked).await;

        let report = h.repository.last();
        assert_eq!(report.webhooks[0].secret_version, 2);
        assert_eq!(
            report.webhook_requests,
            vec![WebhookRequestStatus {
                id: "r1".to_string(),
                phase: WebhookRequestPhase::Done,
                message: None
            }]
        );
        assert_ne!(h.secrets.secret(ID_A), before);

        let rotated = after(asked, &report);
        h.ferriskey.clear_calls();
        let patches = h.repository.reports().len();
        h.reconciler.reconcile(&rotated).await;
        assert!(
            !h.ferriskey
                .calls()
                .iter()
                .any(|call| call.starts_with("rotate"))
        );
        assert_eq!(h.repository.reports().len(), patches);
    }

    #[tokio::test]
    async fn a_retry_request_calls_ferriskey_once_with_the_delivery() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let mut asked = after(first, &h.repository.last());
        let mut retry = request("r1", WebhookRequestKind::RetryDelivery, ID_A);
        retry.delivery_id = Some("d-9".to_string());
        asked.spec.webhook_requests = vec![retry];

        h.reconciler.reconcile(&asked).await;

        assert!(
            h.ferriskey
                .calls()
                .contains(&"retry master fk-1 d-9".to_string())
        );
        assert_eq!(
            h.repository.last().webhook_requests[0].phase,
            WebhookRequestPhase::Done
        );
    }

    #[tokio::test]
    async fn requests_that_cannot_work_are_recorded_as_failed_without_a_call() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let mut asked = after(first, &h.repository.last());
        asked.spec.webhook_requests = vec![
            request("no-delivery", WebhookRequestKind::RetryDelivery, ID_A),
            request("unknown", WebhookRequestKind::RotateSecret, ID_B),
        ];
        h.ferriskey.clear_calls();

        h.reconciler.reconcile(&asked).await;

        let recorded = h.repository.last().webhook_requests;
        assert_eq!(recorded.len(), 2);
        assert!(
            recorded
                .iter()
                .all(|r| r.phase == WebhookRequestPhase::Failed)
        );
        assert!(
            !h.ferriskey
                .calls()
                .iter()
                .any(|call| call.starts_with("retry") || call.starts_with("rotate"))
        );
    }

    #[tokio::test]
    async fn a_request_refused_by_ferriskey_is_recorded_with_its_message() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let mut asked = after(first, &h.repository.last());
        let mut retry = request("r1", WebhookRequestKind::RetryDelivery, ID_A);
        retry.delivery_id = Some("d-9".to_string());
        asked.spec.webhook_requests = vec![retry];
        h.ferriskey.fail(
            "retry",
            WebhookError::Rejected {
                message: "FerrisKey answered 409: delivery is in flight".to_string(),
            },
        );

        h.reconciler.reconcile(&asked).await;

        let recorded = &h.repository.last().webhook_requests[0];
        assert_eq!(recorded.phase, WebhookRequestPhase::Failed);
        assert_eq!(
            recorded.message.as_deref(),
            Some("FerrisKey answered 409: delivery is in flight")
        );
    }

    #[tokio::test]
    async fn a_request_hit_by_a_transient_error_is_not_recorded_and_runs_again() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let mut asked = after(first, &h.repository.last());
        asked.spec.webhook_requests = vec![request("r1", WebhookRequestKind::RotateSecret, ID_A)];
        h.ferriskey.fail(
            "rotate",
            WebhookError::Retry {
                message: "FerrisKey is unreachable".to_string(),
            },
        );

        let outcome = h.reconciler.reconcile(&asked).await;

        assert_eq!(outcome, WebhookOutcome::Retry);
        assert!(h.repository.last().webhook_requests.is_empty());
        h.ferriskey.heal("rotate");
        let outcome = h.reconciler.reconcile(&asked).await;
        assert_eq!(outcome, WebhookOutcome::Settled);
        assert_eq!(h.repository.last().webhooks[0].secret_version, 2);
    }

    #[tokio::test]
    async fn a_secret_that_cannot_be_stored_does_not_count_as_issued() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        h.secrets.fail_put_secret(true);

        let outcome = h
            .reconciler
            .reconcile(&instance(vec![webhook(ID_A, "master")], vec![]))
            .await;

        assert_eq!(outcome, WebhookOutcome::Retry);
        let status = &h.repository.last().webhooks[0];
        assert_eq!(status.phase, WebhookPhase::Failed);
        assert_eq!(status.secret_version, 0);
    }

    #[tokio::test]
    async fn realms_are_reported_sorted_and_kept_when_the_listing_fails() {
        let h = harness(
            FakeFerriskey::with_realms(&["zeta", "master", "acme"]),
            FakeSecrets::default(),
        );
        let bare = instance(vec![], vec![]);
        h.reconciler.reconcile(&bare).await;
        assert_eq!(h.repository.last().realms, vec!["acme", "master", "zeta"]);
        let known = after(bare, &h.repository.last());
        h.ferriskey.fail(
            "list_realms",
            WebhookError::Retry {
                message: "FerrisKey is unreachable".to_string(),
            },
        );
        let patches = h.repository.reports().len();

        let outcome = h.reconciler.reconcile(&known).await;

        assert_eq!(outcome, WebhookOutcome::Retry);
        assert_eq!(h.repository.reports().len(), patches);
    }

    #[tokio::test]
    async fn an_instance_with_no_webhooks_only_lists_realms() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );

        h.reconciler.reconcile(&instance(vec![], vec![])).await;

        assert_eq!(h.ferriskey.calls(), vec!["list_realms"]);
    }

    #[tokio::test]
    async fn instances_that_cannot_serve_are_left_alone() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let wanted = vec![webhook(ID_A, "master")];

        let mut not_ready = instance(wanted.clone(), vec![]);
        not_ready.status.as_mut().unwrap().ready = false;
        let mut keycloak = instance(wanted.clone(), vec![]);
        keycloak.spec.provider = IdentityProvider::Keycloak;
        let mut stopped = instance(wanted, vec![]);
        stopped.spec.stopped = true;

        for instance in [not_ready, keycloak, stopped] {
            assert_eq!(
                h.reconciler.reconcile(&instance).await,
                WebhookOutcome::Settled
            );
        }
        assert!(h.ferriskey.calls().is_empty());
        assert!(h.repository.reports().is_empty());
    }

    #[tokio::test]
    async fn a_status_that_cannot_be_written_asks_for_another_pass() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        *h.repository.fail.lock().unwrap() = true;

        let outcome = h
            .reconciler
            .reconcile(&instance(vec![webhook(ID_A, "master")], vec![]))
            .await;

        assert_eq!(outcome, WebhookOutcome::Retry);
    }

    fn remote_delivery(id: &str, created: i64) -> RemoteDelivery {
        use chrono::TimeZone;
        RemoteDelivery {
            id: id.to_string(),
            event: "user.created".to_string(),
            status: "succeeded".to_string(),
            attempt_count: 1,
            last_status_code: Some(200),
            last_error: None,
            created_at: chrono::Utc
                .timestamp_opt(1_790_000_000 + created, 0)
                .unwrap(),
            updated_at: chrono::Utc
                .timestamp_opt(1_790_000_001 + created, 0)
                .unwrap(),
        }
    }

    #[tokio::test]
    async fn deliveries_are_published_under_the_autharie_id_and_only_rewritten_on_change() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let created = after(first, &h.repository.last());
        h.ferriskey.state.lock().unwrap().deliveries.insert(
            "fk-1".to_string(),
            vec![remote_delivery("d2", 10), remote_delivery("d1", 0)],
        );

        h.reconciler.reconcile(&created).await;
        h.reconciler.reconcile(&created).await;

        let document = h.deliveries.document.lock().unwrap().clone().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(parsed[0]["delivery_id"], "d2");
        assert_eq!(parsed[0]["webhook_id"], ID_A);
        assert_eq!(parsed[1]["delivery_id"], "d1");
        assert_eq!(*h.deliveries.writes.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn a_failed_delivery_read_keeps_the_previous_document_and_the_webhook_active() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let created = after(first, &h.repository.last());
        h.ferriskey
            .state
            .lock()
            .unwrap()
            .deliveries
            .insert("fk-1".to_string(), vec![remote_delivery("d1", 0)]);
        h.reconciler.reconcile(&created).await;
        let before = h.deliveries.document.lock().unwrap().clone();
        h.ferriskey.fail(
            "deliveries",
            WebhookError::Retry {
                message: "FerrisKey is unreachable".to_string(),
            },
        );
        h.ferriskey.state.lock().unwrap().deliveries.clear();

        let outcome = h.reconciler.reconcile(&created).await;

        assert_eq!(outcome, WebhookOutcome::Settled);
        assert_eq!(*h.deliveries.document.lock().unwrap(), before);
        assert_eq!(h.repository.reports().len(), 1);
    }

    #[tokio::test]
    async fn removing_every_webhook_empties_the_published_document() {
        let h = harness(
            FakeFerriskey::with_realms(&["master"]),
            FakeSecrets::default(),
        );
        let first = instance(vec![webhook(ID_A, "master")], vec![]);
        h.reconciler.reconcile(&first).await;
        let created = after(first, &h.repository.last());
        h.ferriskey
            .state
            .lock()
            .unwrap()
            .deliveries
            .insert("fk-1".to_string(), vec![remote_delivery("d1", 0)]);
        h.reconciler.reconcile(&created).await;

        let mut emptied = created;
        emptied.spec.webhooks.clear();
        h.reconciler.reconcile(&emptied).await;

        assert_eq!(h.deliveries.document.lock().unwrap().as_deref(), Some("[]"));
    }

    #[test]
    fn recorded_requests_are_capped_but_the_ones_still_asked_for_stay() {
        let mut recorded: Vec<_> = (0..MAX_RECORDED_REQUESTS + 5)
            .map(|n| WebhookRequestStatus {
                id: format!("r{n}"),
                phase: WebhookRequestPhase::Done,
                message: None,
            })
            .collect();
        let asked = vec![request("r0", WebhookRequestKind::RotateSecret, ID_A)];

        trim_recorded(&mut recorded, &asked);

        assert_eq!(recorded.len(), MAX_RECORDED_REQUESTS);
        assert_eq!(recorded[0].id, "r0");
        assert_eq!(recorded[1].id, "r6");
    }
}
