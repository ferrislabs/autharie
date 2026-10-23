use std::sync::Arc;
use std::time::Duration;

use autharie_crds::common::types::{Condition, ConditionStatus, Phase};
use autharie_crds::v1alpha::identity_instance::{
    IdentityInstance, IdentityInstanceStatus, IdentityProvider,
};

use crate::domain::identity_instance::theme::{
    ThemeDecision, decide, default_theme_config, differs, marker, outcome_status, theme_config,
};
use crate::domain::ports::{
    IdentityInstanceDeployer, IdentityInstanceRepository, IdentityInstanceService,
    InstanceThemePort,
};
use crate::domain::shared_database::SharedDatabaseError;
use crate::domain::{OperatorError, ReconcileOutcome};

const DEPLOYING_REQUEUE_SECONDS: u64 = 15;
const STEADY_STATE_REQUEUE_SECONDS: u64 = 60;
const READY_CONDITION: &str = "Ready";

fn shared_database_failure_status(
    current: &IdentityInstanceStatus,
    cause: &SharedDatabaseError,
    now: String,
) -> IdentityInstanceStatus {
    let reason = Some(cause.reason().to_string());
    let message = Some(cause.to_string());

    // The same failure again keeps the time it started. A fresh timestamp on
    // every attempt makes each status write a change, each change a watch
    // event, and each event another reconcile: a hot loop against a cluster
    // that is already refusing.
    let since = current
        .conditions
        .iter()
        .find(|c| {
            c.condition_type == READY_CONDITION
                && c.status == ConditionStatus::False
                && c.reason == reason
                && c.message == message
        })
        .map(|c| c.last_transition_time.clone())
        .unwrap_or(now);

    let mut status = current.clone();
    status.ready = false;
    status.error = Some(cause.to_string());
    status
        .conditions
        .retain(|c| c.condition_type != READY_CONDITION);
    status.conditions.push(Condition {
        condition_type: READY_CONDITION.to_string(),
        status: ConditionStatus::False,
        last_transition_time: since,
        reason,
        message,
    });
    status
}

pub struct IdentityInstanceServiceImpl<R, D, T> {
    repository: Arc<R>,
    deployer: Arc<D>,
    theme: Arc<T>,
}

impl<R, D, T> IdentityInstanceServiceImpl<R, D, T> {
    pub fn new(repository: Arc<R>, deployer: Arc<D>, theme: Arc<T>) -> Self {
        Self {
            repository,
            deployer,
            theme,
        }
    }

    fn build_desired_status(
        &self,
        instance: &IdentityInstance,
        database_ready: bool,
        provider_ready: bool,
        edge_ready: bool,
        upgrade_in_progress: bool,
    ) -> IdentityInstanceStatus {
        let mut status = instance.status.clone().unwrap_or_default();

        let stopped = instance.spec.stopped;

        status.ready =
            !stopped && database_ready && provider_ready && edge_ready && !upgrade_in_progress;
        status.error = None;
        status
            .conditions
            .retain(|c| c.condition_type != READY_CONDITION);
        status.phase = Some(if stopped {
            Phase::Stopped
        } else if !database_ready {
            Phase::DatabaseProvisioning
        } else if upgrade_in_progress {
            Phase::Upgrading
        } else if provider_ready && edge_ready {
            Phase::Running
        } else {
            Phase::Deploying
        });

        if status.endpoint.is_none() {
            status.endpoint = Some(format!("https://{}", instance.spec.hostname));
        }

        if status.admin_url.is_none() {
            status.admin_url = Some(format!("https://{}/admin", instance.spec.hostname));
        }

        status
    }
}

impl<R, D, T> IdentityInstanceService for IdentityInstanceServiceImpl<R, D, T>
where
    R: IdentityInstanceRepository,
    D: IdentityInstanceDeployer,
    T: InstanceThemePort,
{
    async fn reconcile(
        &self,
        instance: IdentityInstance,
    ) -> Result<ReconcileOutcome, OperatorError> {
        if let Err(error) = self.ensure_instance(&instance).await {
            if let OperatorError::SharedDatabase(cause) = &error {
                let status = shared_database_failure_status(
                    &instance.status.clone().unwrap_or_default(),
                    cause,
                    chrono::Utc::now().to_rfc3339(),
                );
                if instance.status.as_ref() != Some(&status)
                    && let Err(patch_error) = self.repository.patch_status(&instance, status).await
                {
                    tracing::warn!(error = %patch_error, "could not report the shared database failure");
                }
            }
            return Err(error);
        }
        let database_ready = self.deployer.database_ready(&instance).await?;
        let provider_ready = self.deployer.provider_ready(&instance).await?;
        let edge_ready = self.deployer.edge_ready(&instance).await?;
        let upgrade_in_progress = self.deployer.upgrade_in_progress(&instance).await?;
        let current_status = instance.status.clone().unwrap_or_default();
        let desired_status = self.build_desired_status(
            &instance,
            database_ready,
            provider_ready,
            edge_ready,
            upgrade_in_progress,
        );

        if desired_status != current_status {
            self.repository
                .patch_status(&instance, desired_status)
                .await?;
            return Ok(ReconcileOutcome::requeue_after(Duration::from_secs(
                DEPLOYING_REQUEUE_SECONDS,
            )));
        }

        if instance.spec.stopped {
            let seconds = if provider_ready {
                STEADY_STATE_REQUEUE_SECONDS
            } else {
                DEPLOYING_REQUEUE_SECONDS
            };
            return Ok(ReconcileOutcome::requeue_after(Duration::from_secs(
                seconds,
            )));
        }

        if !database_ready || !provider_ready || !edge_ready || upgrade_in_progress {
            return Ok(ReconcileOutcome::requeue_after(Duration::from_secs(
                DEPLOYING_REQUEUE_SECONDS,
            )));
        }

        if self.reconcile_branding(&instance).await? == BrandingOutcome::Retry {
            return Ok(ReconcileOutcome::requeue_after(Duration::from_secs(
                DEPLOYING_REQUEUE_SECONDS,
            )));
        }

        Ok(ReconcileOutcome::requeue_after(Duration::from_secs(
            STEADY_STATE_REQUEUE_SECONDS,
        )))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BrandingOutcome {
    Settled,
    Retry,
}

impl<R, D, T> IdentityInstanceServiceImpl<R, D, T>
where
    R: IdentityInstanceRepository,
    T: InstanceThemePort,
{
    async fn reconcile_branding(
        &self,
        instance: &IdentityInstance,
    ) -> Result<BrandingOutcome, OperatorError> {
        if instance.spec.provider != IdentityProvider::Ferriskey {
            return Ok(BrandingOutcome::Settled);
        }

        let previous = instance
            .status
            .as_ref()
            .and_then(|status| status.iam.as_ref());
        let wanted = instance
            .spec
            .iam
            .as_ref()
            .and_then(|iam| iam.branding.as_ref());

        let (config, target) = match decide(wanted, previous.and_then(|iam| iam.applied.as_deref()))
        {
            ThemeDecision::Nothing => return Ok(BrandingOutcome::Settled),
            ThemeDecision::Apply(branding) => (theme_config(branding), Some(marker(branding))),
            ThemeDecision::RestoreDefault => (default_theme_config(), None),
        };

        let outcome = self.theme.put_theme(instance, &config).await;
        let retry = matches!(
            outcome,
            Err(crate::domain::identity_instance::theme::ThemeError::Retry { .. })
        );
        let next = outcome_status(previous, target, outcome, chrono::Utc::now().to_rfc3339());

        if differs(previous, &next) {
            let mut status = instance.status.clone().unwrap_or_default();
            status.iam = Some(next);
            self.repository.patch_status(instance, status).await?;
        }

        Ok(if retry {
            BrandingOutcome::Retry
        } else {
            BrandingOutcome::Settled
        })
    }
}

impl<R, D, T> IdentityInstanceServiceImpl<R, D, T>
where
    D: IdentityInstanceDeployer,
{
    async fn ensure_instance(&self, instance: &IdentityInstance) -> Result<(), OperatorError> {
        self.deployer.ensure_provider_resources(instance).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::identity_instance::theme::ThemeError;
    use crate::domain::ports::{
        MockIdentityInstanceDeployer, MockIdentityInstanceRepository, MockInstanceThemePort,
    };
    use autharie_crds::common::types::Phase;
    use autharie_crds::common::types::ResourceRequirements;
    use autharie_crds::v1alpha::identity_instance::{
        Branding, DatabaseConfig, DatabaseMode, IamConfig, IamPhase, IamStatus, IdentityInstance,
        IdentityInstanceSpec, IdentityInstanceStatus, IdentityProvider, ManagedClusterConfig,
        ManagedClusterStorage,
    };
    use kube::core::ObjectMeta;
    use std::sync::Arc;

    fn instance_with_status(status: Option<IdentityInstanceStatus>) -> IdentityInstance {
        IdentityInstance {
            metadata: ObjectMeta {
                name: Some("instance-1".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: IdentityInstanceSpec {
                webhooks: Vec::new(),
                webhooks_revision: 0,
                webhook_requests: Vec::new(),
                agents: None,
                smtp: Vec::new(),
                restore: None,
                organisation_id: "org-1".to_string(),
                provider: IdentityProvider::Keycloak,
                version: "25.0.0".to_string(),
                hostname: "auth.acme.test".to_string(),
                replicas: 1,
                stopped: false,
                database: DatabaseConfig {
                    mode: DatabaseMode::ManagedCluster,
                    shared: None,
                    managed_cluster: ManagedClusterConfig {
                        instances: 1,
                        storage: ManagedClusterStorage {
                            size: "10Gi".to_string(),
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
                iam: None,
                backup: None,
            },
            status,
        }
    }

    #[tokio::test]
    async fn reconcile_updates_status_and_requeues() {
        let instance = instance_with_status(None);
        let mut repository = MockIdentityInstanceRepository::new();
        let mut deployer = MockIdentityInstanceDeployer::new();

        deployer
            .expect_ensure_provider_resources()
            .times(1)
            .returning(|_| Box::pin(async { Ok(()) }));
        deployer
            .expect_database_ready()
            .times(1)
            .returning(|_| Box::pin(async { Ok(false) }));
        deployer
            .expect_provider_ready()
            .times(1)
            .returning(|_| Box::pin(async { Ok(false) }));
        deployer
            .expect_edge_ready()
            .times(1)
            .returning(|_| Box::pin(async { Ok(false) }));
        deployer
            .expect_upgrade_in_progress()
            .times(1)
            .returning(|_| Box::pin(async { Ok(false) }));

        repository
            .expect_patch_status()
            .times(1)
            .withf(|instance, status| {
                instance.metadata.name.as_deref() == Some("instance-1")
                    && status.phase == Some(Phase::DatabaseProvisioning)
                    && !status.ready
                    && status.endpoint.as_deref() == Some("https://auth.acme.test")
                    && status.admin_url.as_deref() == Some("https://auth.acme.test/admin")
            })
            .returning(|instance, _status| {
                let instance = instance.clone();
                Box::pin(async move { Ok(instance) })
            });

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(deployer),
            Arc::new(MockInstanceThemePort::new()),
        );
        let outcome = service.reconcile(instance).await.unwrap();

        assert_eq!(outcome.requeue_after, Some(Duration::from_secs(15)));
    }

    #[tokio::test]
    async fn reconcile_no_status_change_does_not_patch() {
        let status = IdentityInstanceStatus {
            phase: Some(Phase::Running),
            ready: true,
            endpoint: Some("https://auth.acme.test".to_string()),
            admin_url: Some("https://auth.acme.test/admin".to_string()),
            ..Default::default()
        };
        let instance = instance_with_status(Some(status));
        let mut repository = MockIdentityInstanceRepository::new();
        let mut deployer = MockIdentityInstanceDeployer::new();

        deployer
            .expect_ensure_provider_resources()
            .times(1)
            .returning(|_| Box::pin(async { Ok(()) }));
        deployer
            .expect_database_ready()
            .times(1)
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_provider_ready()
            .times(1)
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_edge_ready()
            .times(1)
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_upgrade_in_progress()
            .times(1)
            .returning(|_| Box::pin(async { Ok(false) }));
        repository.expect_patch_status().times(0);

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(deployer),
            Arc::new(MockInstanceThemePort::new()),
        );
        let outcome = service.reconcile(instance).await.unwrap();

        assert_eq!(
            outcome.requeue_after,
            Some(Duration::from_secs(STEADY_STATE_REQUEUE_SECONDS))
        );
    }

    #[tokio::test]
    async fn reconcile_requeues_while_provider_not_ready_even_without_status_change() {
        let status = IdentityInstanceStatus {
            phase: Some(Phase::Deploying),
            ready: false,
            endpoint: Some("https://auth.acme.test".to_string()),
            admin_url: Some("https://auth.acme.test/admin".to_string()),
            ..Default::default()
        };
        let instance = instance_with_status(Some(status));
        let mut repository = MockIdentityInstanceRepository::new();
        let mut deployer = MockIdentityInstanceDeployer::new();

        deployer
            .expect_ensure_provider_resources()
            .times(1)
            .returning(|_| Box::pin(async { Ok(()) }));
        deployer
            .expect_database_ready()
            .times(1)
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_provider_ready()
            .times(1)
            .returning(|_| Box::pin(async { Ok(false) }));
        deployer
            .expect_edge_ready()
            .times(1)
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_upgrade_in_progress()
            .times(1)
            .returning(|_| Box::pin(async { Ok(false) }));
        repository.expect_patch_status().times(0);

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(deployer),
            Arc::new(MockInstanceThemePort::new()),
        );
        let outcome = service.reconcile(instance).await.unwrap();

        assert_eq!(
            outcome.requeue_after,
            Some(Duration::from_secs(DEPLOYING_REQUEUE_SECONDS))
        );
    }

    #[test]
    fn build_desired_status_keeps_existing_fields() {
        let status = IdentityInstanceStatus {
            phase: Some(Phase::Running),
            ready: true,
            endpoint: Some("https://already.example".to_string()),
            admin_url: Some("https://already.example/admin".to_string()),
            ..Default::default()
        };
        let instance = instance_with_status(Some(status.clone()));
        let service = IdentityInstanceServiceImpl::new(
            Arc::new(MockIdentityInstanceRepository::new()),
            Arc::new(MockIdentityInstanceDeployer::new()),
            Arc::new(MockInstanceThemePort::new()),
        );

        let desired = service.build_desired_status(&instance, true, true, true, false);
        assert_eq!(desired.phase, status.phase);
        assert_eq!(desired.ready, status.ready);
        assert_eq!(desired.endpoint, status.endpoint);
        assert_eq!(desired.admin_url, status.admin_url);
    }

    fn stopped_status() -> IdentityInstanceStatus {
        IdentityInstanceStatus {
            phase: Some(Phase::Stopped),
            ready: false,
            endpoint: Some("https://auth.acme.test".to_string()),
            admin_url: Some("https://auth.acme.test/admin".to_string()),
            ..Default::default()
        }
    }

    fn service_without_collaborators() -> IdentityInstanceServiceImpl<
        MockIdentityInstanceRepository,
        MockIdentityInstanceDeployer,
        MockInstanceThemePort,
    > {
        IdentityInstanceServiceImpl::new(
            Arc::new(MockIdentityInstanceRepository::new()),
            Arc::new(MockIdentityInstanceDeployer::new()),
            Arc::new(MockInstanceThemePort::new()),
        )
    }

    #[test]
    fn a_stopped_instance_is_stopped_and_not_ready_whatever_is_running() {
        let mut instance = instance_with_status(Some(running_status(None)));
        instance.spec.stopped = true;
        let service = service_without_collaborators();

        for upgrade_in_progress in [false, true] {
            let desired =
                service.build_desired_status(&instance, true, true, true, upgrade_in_progress);

            assert_eq!(desired.phase, Some(Phase::Stopped));
            assert!(!desired.ready);
        }
        assert_eq!(
            service
                .build_desired_status(&instance, false, false, false, false)
                .phase,
            Some(Phase::Stopped)
        );
    }

    #[test]
    fn a_started_instance_leaves_stopped_for_the_usual_phases() {
        let instance = instance_with_status(Some(stopped_status()));
        let service = service_without_collaborators();

        let desired = service.build_desired_status(&instance, true, true, true, false);
        assert_eq!(desired.phase, Some(Phase::Running));
        assert!(desired.ready);

        let desired = service.build_desired_status(&instance, false, false, false, false);
        assert_eq!(desired.phase, Some(Phase::DatabaseProvisioning));
        assert!(!desired.ready);
    }

    #[tokio::test]
    async fn stopping_an_instance_reports_it_stopped() {
        let mut instance = instance_with_status(Some(running_status(None)));
        instance.spec.stopped = true;
        let mut repository = MockIdentityInstanceRepository::new();
        repository
            .expect_patch_status()
            .times(1)
            .withf(|_, status| status.phase == Some(Phase::Stopped) && !status.ready)
            .returning(|instance, _| {
                let instance = instance.clone();
                Box::pin(async move { Ok(instance) })
            });

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(ready_deployer()),
            Arc::new(MockInstanceThemePort::new()),
        );
        let outcome = service.reconcile(instance).await.unwrap();

        assert_eq!(
            outcome.requeue_after,
            Some(Duration::from_secs(DEPLOYING_REQUEUE_SECONDS))
        );
    }

    #[tokio::test]
    async fn a_stopped_instance_is_not_branded() {
        let mut instance = ferriskey_with(Some(radius(6)), None);
        instance.spec.stopped = true;
        instance.status = Some(stopped_status());
        let mut theme = MockInstanceThemePort::new();
        theme.expect_put_theme().times(0);
        let mut repository = MockIdentityInstanceRepository::new();
        repository.expect_patch_status().times(0);

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(ready_deployer()),
            Arc::new(theme),
        );
        let outcome = service.reconcile(instance).await.unwrap();

        assert_eq!(
            outcome.requeue_after,
            Some(Duration::from_secs(STEADY_STATE_REQUEUE_SECONDS))
        );
    }

    #[tokio::test]
    async fn a_hibernated_database_does_not_keep_a_stopped_instance_on_the_fast_requeue() {
        let mut instance = instance_with_status(Some(stopped_status()));
        instance.spec.stopped = true;
        let mut deployer = MockIdentityInstanceDeployer::new();
        deployer
            .expect_ensure_provider_resources()
            .times(1)
            .returning(|_| Box::pin(async { Ok(()) }));
        deployer
            .expect_database_ready()
            .returning(|_| Box::pin(async { Ok(false) }));
        deployer
            .expect_provider_ready()
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_edge_ready()
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_upgrade_in_progress()
            .returning(|_| Box::pin(async { Ok(false) }));
        let mut repository = MockIdentityInstanceRepository::new();
        repository.expect_patch_status().times(0);

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(deployer),
            Arc::new(MockInstanceThemePort::new()),
        );
        let outcome = service.reconcile(instance).await.unwrap();

        assert_eq!(
            outcome.requeue_after,
            Some(Duration::from_secs(STEADY_STATE_REQUEUE_SECONDS))
        );
    }

    #[tokio::test]
    async fn a_stopped_instance_still_going_down_is_looked_at_again_soon() {
        let mut instance = ferriskey_with(Some(radius(6)), None);
        instance.spec.stopped = true;
        instance.status = Some(stopped_status());
        let mut deployer = MockIdentityInstanceDeployer::new();
        deployer
            .expect_ensure_provider_resources()
            .times(1)
            .returning(|_| Box::pin(async { Ok(()) }));
        deployer
            .expect_database_ready()
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_provider_ready()
            .returning(|_| Box::pin(async { Ok(false) }));
        deployer
            .expect_edge_ready()
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_upgrade_in_progress()
            .returning(|_| Box::pin(async { Ok(false) }));
        let mut theme = MockInstanceThemePort::new();
        theme.expect_put_theme().times(0);
        let mut repository = MockIdentityInstanceRepository::new();
        repository.expect_patch_status().times(0);

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(deployer),
            Arc::new(theme),
        );
        let outcome = service.reconcile(instance).await.unwrap();

        assert_eq!(
            outcome.requeue_after,
            Some(Duration::from_secs(DEPLOYING_REQUEUE_SECONDS))
        );
    }

    fn ready_deployer() -> MockIdentityInstanceDeployer {
        let mut deployer = MockIdentityInstanceDeployer::new();
        deployer
            .expect_ensure_provider_resources()
            .returning(|_| Box::pin(async { Ok(()) }));
        deployer
            .expect_database_ready()
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_provider_ready()
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_edge_ready()
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_upgrade_in_progress()
            .returning(|_| Box::pin(async { Ok(false) }));
        deployer
    }

    fn running_status(iam: Option<IamStatus>) -> IdentityInstanceStatus {
        IdentityInstanceStatus {
            phase: Some(Phase::Running),
            ready: true,
            endpoint: Some("https://auth.acme.test".to_string()),
            admin_url: Some("https://auth.acme.test/admin".to_string()),
            iam,
            ..Default::default()
        }
    }

    fn ferriskey_with(branding: Option<Branding>, iam: Option<IamStatus>) -> IdentityInstance {
        let mut instance = instance_with_status(Some(running_status(iam)));
        instance.spec.provider = IdentityProvider::Ferriskey;
        instance.spec.iam = Some(IamConfig { branding });
        instance
    }

    fn radius(radius: u8) -> Branding {
        Branding {
            colors: None,
            radius: Some(radius),
        }
    }

    fn iam_status(phase: IamPhase, applied: Option<String>) -> IamStatus {
        IamStatus {
            phase,
            message: None,
            applied,
            observed_at: Some("t".to_string()),
        }
    }

    #[tokio::test]
    async fn branding_not_yet_applied_is_applied_and_recorded() {
        let branding = radius(6);
        let instance = ferriskey_with(Some(branding.clone()), None);
        let mut theme = MockInstanceThemePort::new();
        theme
            .expect_put_theme()
            .times(1)
            .withf(|_, config| config["borders"]["widgetRadius"] == 6)
            .returning(|_, _| Box::pin(async { Ok(()) }));
        let mut repository = MockIdentityInstanceRepository::new();
        let expected = marker(&branding);
        repository
            .expect_patch_status()
            .times(1)
            .withf(move |_, status| {
                status.iam.as_ref().is_some_and(|iam| {
                    iam.phase == IamPhase::Applied
                        && iam.applied.as_deref() == Some(expected.as_str())
                        && iam.message.is_none()
                })
            })
            .returning(|instance, _| {
                let instance = instance.clone();
                Box::pin(async move { Ok(instance) })
            });

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(ready_deployer()),
            Arc::new(theme),
        );
        let outcome = service.reconcile(instance).await.unwrap();

        assert_eq!(
            outcome.requeue_after,
            Some(Duration::from_secs(STEADY_STATE_REQUEUE_SECONDS))
        );
    }

    #[tokio::test]
    async fn unchanged_branding_is_not_applied_again() {
        let branding = radius(6);
        let applied = iam_status(IamPhase::Applied, Some(marker(&branding)));
        let instance = ferriskey_with(Some(branding), Some(applied));
        let mut repository = MockIdentityInstanceRepository::new();
        repository.expect_patch_status().times(0);

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(ready_deployer()),
            Arc::new(MockInstanceThemePort::new()),
        );
        service.reconcile(instance).await.unwrap();
    }

    #[tokio::test]
    async fn no_branding_and_no_marker_makes_no_call() {
        let instance = ferriskey_with(None, None);
        let mut repository = MockIdentityInstanceRepository::new();
        repository.expect_patch_status().times(0);

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(ready_deployer()),
            Arc::new(MockInstanceThemePort::new()),
        );
        service.reconcile(instance).await.unwrap();
    }

    #[tokio::test]
    async fn cleared_branding_restores_the_default_once() {
        let applied = iam_status(IamPhase::Applied, Some(marker(&radius(6))));
        let instance = ferriskey_with(None, Some(applied));
        let mut theme = MockInstanceThemePort::new();
        theme
            .expect_put_theme()
            .times(1)
            .withf(|_, config| *config == serde_json::json!({}))
            .returning(|_, _| Box::pin(async { Ok(()) }));
        let mut repository = MockIdentityInstanceRepository::new();
        repository
            .expect_patch_status()
            .times(1)
            .withf(|_, status| {
                status
                    .iam
                    .as_ref()
                    .is_some_and(|iam| iam.phase == IamPhase::Applied && iam.applied.is_none())
            })
            .returning(|instance, _| {
                let instance = instance.clone();
                Box::pin(async move { Ok(instance) })
            });

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(ready_deployer()),
            Arc::new(theme),
        );
        service.reconcile(instance).await.unwrap();
    }

    #[tokio::test]
    async fn unreachable_provider_is_pending_and_requeued_soon() {
        let instance = ferriskey_with(Some(radius(6)), None);
        let mut theme = MockInstanceThemePort::new();
        theme.expect_put_theme().times(1).returning(|_, _| {
            Box::pin(async {
                Err(ThemeError::Retry {
                    message: "FerrisKey is not answering".to_string(),
                })
            })
        });
        let mut repository = MockIdentityInstanceRepository::new();
        repository
            .expect_patch_status()
            .times(1)
            .withf(|_, status| {
                status.iam.as_ref().is_some_and(|iam| {
                    iam.phase == IamPhase::Pending
                        && iam.message.as_deref() == Some("FerrisKey is not answering")
                        && iam.applied.is_none()
                })
            })
            .returning(|instance, _| {
                let instance = instance.clone();
                Box::pin(async move { Ok(instance) })
            });

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(ready_deployer()),
            Arc::new(theme),
        );
        let outcome = service.reconcile(instance).await.unwrap();

        assert_eq!(
            outcome.requeue_after,
            Some(Duration::from_secs(DEPLOYING_REQUEUE_SECONDS))
        );
    }

    #[tokio::test]
    async fn repeated_pending_with_the_same_reason_does_not_patch_again() {
        let mut pending = iam_status(IamPhase::Pending, None);
        pending.message = Some("FerrisKey is not answering".to_string());
        let instance = ferriskey_with(Some(radius(6)), Some(pending));
        let mut theme = MockInstanceThemePort::new();
        theme.expect_put_theme().times(1).returning(|_, _| {
            Box::pin(async {
                Err(ThemeError::Retry {
                    message: "FerrisKey is not answering".to_string(),
                })
            })
        });
        let mut repository = MockIdentityInstanceRepository::new();
        repository.expect_patch_status().times(0);

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(ready_deployer()),
            Arc::new(theme),
        );
        service.reconcile(instance).await.unwrap();
    }

    #[tokio::test]
    async fn rejected_config_is_failed_with_the_message() {
        let instance = ferriskey_with(Some(radius(6)), None);
        let mut theme = MockInstanceThemePort::new();
        theme.expect_put_theme().times(1).returning(|_, _| {
            Box::pin(async {
                Err(ThemeError::Rejected {
                    message: "invalid color".to_string(),
                })
            })
        });
        let mut repository = MockIdentityInstanceRepository::new();
        repository
            .expect_patch_status()
            .times(1)
            .withf(|_, status| {
                status.iam.as_ref().is_some_and(|iam| {
                    iam.phase == IamPhase::Failed && iam.message.as_deref() == Some("invalid color")
                })
            })
            .returning(|instance, _| {
                let instance = instance.clone();
                Box::pin(async move { Ok(instance) })
            });

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(ready_deployer()),
            Arc::new(theme),
        );
        let outcome = service.reconcile(instance).await.unwrap();

        assert_eq!(
            outcome.requeue_after,
            Some(Duration::from_secs(STEADY_STATE_REQUEUE_SECONDS))
        );
    }

    #[tokio::test]
    async fn an_instance_that_is_not_ready_gets_no_theme() {
        let mut instance = ferriskey_with(Some(radius(6)), None);
        instance.status = None;
        let mut deployer = MockIdentityInstanceDeployer::new();
        deployer
            .expect_ensure_provider_resources()
            .returning(|_| Box::pin(async { Ok(()) }));
        deployer
            .expect_database_ready()
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_provider_ready()
            .returning(|_| Box::pin(async { Ok(false) }));
        deployer
            .expect_edge_ready()
            .returning(|_| Box::pin(async { Ok(true) }));
        deployer
            .expect_upgrade_in_progress()
            .returning(|_| Box::pin(async { Ok(false) }));
        let mut repository = MockIdentityInstanceRepository::new();
        repository.expect_patch_status().returning(|instance, _| {
            let instance = instance.clone();
            Box::pin(async move { Ok(instance) })
        });

        let service = IdentityInstanceServiceImpl::new(
            Arc::new(repository),
            Arc::new(deployer),
            Arc::new(MockInstanceThemePort::new()),
        );
        service.reconcile(instance).await.unwrap();
    }

    #[test]
    fn a_refused_shared_database_reads_as_ready_false_with_its_reason() {
        let status = shared_database_failure_status(
            &IdentityInstanceStatus {
                ready: true,
                ..Default::default()
            },
            &SharedDatabaseError::Refused("no".to_string()),
            "now".to_string(),
        );

        assert!(!status.ready);
        assert_eq!(status.conditions.len(), 1);
        assert_eq!(status.conditions[0].condition_type, "Ready");
        assert_eq!(status.conditions[0].status, ConditionStatus::False);
        assert_eq!(
            status.conditions[0].reason.as_deref(),
            Some("SharedDatabaseRefused")
        );
        assert!(status.error.is_some());
    }

    /// The loop this guards against: a new timestamp on every attempt made
    /// each write a change, and each change another reconcile.
    #[test]
    fn the_same_failure_again_writes_an_identical_status() {
        let cause = SharedDatabaseError::Refused("no".to_string());
        let first =
            shared_database_failure_status(&IdentityInstanceStatus::default(), &cause, "t1".into());
        let again = shared_database_failure_status(&first, &cause, "t2".into());

        assert_eq!(again, first);
        assert_eq!(again.conditions[0].last_transition_time, "t1");
    }

    #[test]
    fn a_different_failure_starts_a_new_transition() {
        let first = shared_database_failure_status(
            &IdentityInstanceStatus::default(),
            &SharedDatabaseError::Refused("no".to_string()),
            "t1".into(),
        );
        let next =
            shared_database_failure_status(&first, &SharedDatabaseError::Unreachable, "t2".into());

        assert_eq!(next.conditions[0].last_transition_time, "t2");
    }

    #[test]
    fn an_unreachable_shared_database_has_its_own_reason() {
        let status = shared_database_failure_status(
            &IdentityInstanceStatus::default(),
            &SharedDatabaseError::Unreachable,
            "now".to_string(),
        );

        assert_eq!(
            status.conditions[0].reason.as_deref(),
            Some("SharedDatabaseUnreachable")
        );
    }
}
