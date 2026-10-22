use std::sync::Arc;

use autharie_crds::v1alpha::identity_instance::{
    AgentPolicyPhase, AgentPolicySpec, AgentPolicyStatus, AgentsPhase, AgentsSpec, AgentsStatus,
    IdentityInstance, IdentityProvider,
};
use futures::lock::Mutex;

use super::config::{AgentsEnvironment, render};
use super::plan::{Change, plan};
use super::ports::{
    AgentInstances, AgentsEdge, AgentsOutcome, AgentsSecretsPort, AgentsStatusRepository,
    FerriskeyAgentsPort, GatewayConfigStore, IdentityInstanceAgents,
};
use super::{
    AgentsError, ManagedPolicy, PolicyWrite, agents_enabled, managed_id, managed_name,
    remote_description, settings_with_resource, settings_without_resource, settled_off,
};

pub struct AgentsReconciler<F, S, R, L, G, E> {
    ferriskey: Arc<F>,
    secrets: Arc<S>,
    status: Arc<R>,
    instances: Arc<L>,
    config: Arc<G>,
    edge: Arc<E>,
    environment: AgentsEnvironment,
    rebuild: Mutex<()>,
}

impl<F, S, R, L, G, E> AgentsReconciler<F, S, R, L, G, E> {
    pub fn new(
        ferriskey: Arc<F>,
        secrets: Arc<S>,
        status: Arc<R>,
        instances: Arc<L>,
        config: Arc<G>,
        edge: Arc<E>,
        environment: AgentsEnvironment,
    ) -> Self {
        Self {
            ferriskey,
            secrets,
            status,
            instances,
            config,
            edge,
            environment,
            rebuild: Mutex::new(()),
        }
    }
}

#[derive(Default)]
struct Tally {
    issues: Vec<AgentsError>,
    retry: bool,
}

impl Tally {
    fn issue(&mut self, error: AgentsError) {
        tracing::warn!(error = %error.message(), "agent settings could not be applied");
        self.issues.push(error);
    }

    fn phase(&self) -> AgentsPhase {
        if self.issues.iter().any(|issue| !issue.is_retry()) {
            AgentsPhase::Failed
        } else if !self.issues.is_empty() || self.retry {
            AgentsPhase::Applying
        } else {
            AgentsPhase::Ready
        }
    }

    fn message(&self) -> Option<String> {
        self.issues.first().map(|issue| issue.message().to_string())
    }
}

fn serves_agents(instance: &IdentityInstance) -> bool {
    instance.spec.provider == IdentityProvider::Ferriskey
        && instance.metadata.deletion_timestamp.is_none()
}

impl<F, S, R, L, G, E> IdentityInstanceAgents for AgentsReconciler<F, S, R, L, G, E>
where
    F: FerriskeyAgentsPort,
    S: AgentsSecretsPort,
    R: AgentsStatusRepository,
    L: AgentInstances,
    G: GatewayConfigStore,
    E: AgentsEdge,
{
    async fn reconcile(&self, instance: &IdentityInstance) -> AgentsOutcome {
        if !serves_agents(instance) || instance.spec.stopped || !instance.is_ready() {
            return AgentsOutcome::Settled;
        }
        let Some(agents) = instance.spec.agents.as_ref() else {
            return AgentsOutcome::Settled;
        };

        if agents.enabled {
            self.enable(instance, agents).await
        } else {
            self.disable(instance, agents).await
        }
    }

    async fn cleanup(&self, instance: &IdentityInstance) {
        if instance.spec.provider != IdentityProvider::Ferriskey {
            return;
        }
        if let Err(error) = self.edge.remove_route(instance).await {
            tracing::warn!(error = %error.message(), "could not remove the MCP route of a deleted instance");
        }
        if let Err(error) = self.rebuild_gateway_config().await {
            tracing::warn!(error = %error.message(), "could not rebuild the gateway configuration after a deletion");
        }
    }
}

impl<F, S, R, L, G, E> AgentsReconciler<F, S, R, L, G, E>
where
    F: FerriskeyAgentsPort,
    S: AgentsSecretsPort,
    R: AgentsStatusRepository,
    L: AgentInstances,
    G: GatewayConfigStore,
    E: AgentsEdge,
{
    async fn enable(&self, instance: &IdentityInstance, agents: &AgentsSpec) -> AgentsOutcome {
        let canonical = self.environment.canonical_uri(instance);
        let mut tally = Tally::default();

        let client_ready = self
            .prepare(instance, &agents.realm, &canonical, &mut tally)
            .await;
        let policies = self.sync_policies(instance, agents, &mut tally).await;
        if client_ready {
            if let Err(error) = self.rebuild_gateway_config().await {
                tally.issue(error);
            }
            if let Err(error) = self.edge.apply_route(instance).await {
                tally.issue(error);
            }
        }

        let status = AgentsStatus {
            phase: tally.phase(),
            gateway_client_ready: client_ready,
            observed_revision: agents.policies_revision,
            mcp_url: Some(canonical),
            message: tally.message(),
            policies,
        };
        self.record(instance, status, tally).await
    }

    async fn prepare(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        canonical: &str,
        tally: &mut Tally,
    ) -> bool {
        if let Err(error) = self
            .ferriskey
            .set_authorization(instance, realm, true)
            .await
        {
            tally.issue(error);
            return false;
        }

        match self.ferriskey.realm_settings(instance, realm).await {
            Ok(current) => {
                if let Some(wanted) = settings_with_resource(&current, canonical)
                    && let Err(error) = self
                        .ferriskey
                        .update_realm_settings(instance, realm, &wanted)
                        .await
                {
                    tally.issue(error);
                    return false;
                }
            }
            Err(error) => {
                tally.issue(error);
                return false;
            }
        }

        let client = match self
            .ferriskey
            .ensure_gateway_client(instance, realm, canonical)
            .await
        {
            Ok(client) => client,
            Err(error) => {
                tally.issue(error);
                return false;
            }
        };

        match self.secrets.load(instance).await {
            Ok(Some(stored)) if stored == client.secret => true,
            Ok(_) => match self.secrets.store(instance, &client.secret).await {
                Ok(()) => true,
                Err(error) => {
                    tally.issue(error);
                    false
                }
            },
            Err(error) => {
                tally.issue(error);
                false
            }
        }
    }

    async fn sync_policies(
        &self,
        instance: &IdentityInstance,
        agents: &AgentsSpec,
        tally: &mut Tally,
    ) -> Vec<AgentPolicyStatus> {
        let realm = agents.realm.as_str();

        let remote: Vec<ManagedPolicy> =
            match self.ferriskey.list_managed_policies(instance, realm).await {
                Ok(found) => found
                    .into_iter()
                    .filter_map(|policy| {
                        let id = managed_id(&policy.name)?.to_string();
                        Some(ManagedPolicy {
                            autharie_id: id,
                            policy,
                        })
                    })
                    .collect(),
                Err(error) => {
                    let message = match &error {
                        AgentsError::NotFound => {
                            "FerrisKey's authorization API is not available yet".to_string()
                        }
                        other => other.message().to_string(),
                    };
                    tally.retry = true;
                    return agents
                        .policies
                        .iter()
                        .map(|spec| AgentPolicyStatus {
                            id: spec.id.clone(),
                            phase: AgentPolicyPhase::Failed,
                            message: Some(message.clone()),
                        })
                        .collect();
                }
            };

        let plan = plan(&agents.policies, &remote);
        let mut statuses = Vec::with_capacity(agents.policies.len());
        for (spec, change) in agents.policies.iter().zip(&plan.changes) {
            let result = self
                .write_policy(instance, realm, spec, change, &remote)
                .await;
            statuses.push(match result {
                Ok(()) => AgentPolicyStatus {
                    id: spec.id.clone(),
                    phase: AgentPolicyPhase::Applied,
                    message: None,
                },
                Err(error) => {
                    let phase = if error.is_retry() {
                        tally.retry = true;
                        AgentPolicyPhase::Failed
                    } else {
                        AgentPolicyPhase::Invalid
                    };
                    AgentPolicyStatus {
                        id: spec.id.clone(),
                        phase,
                        message: Some(error.message().to_string()),
                    }
                }
            });
        }

        for stale in &plan.deletes {
            if let Err(error) = self.ferriskey.delete_policy(instance, realm, stale).await
                && error != AgentsError::NotFound
            {
                tracing::warn!(policy = %stale.autharie_id, error = %error.message(), "could not delete a policy that is no longer wanted");
                tally.retry = true;
            }
        }

        statuses
    }

    async fn write_policy(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        spec: &AgentPolicySpec,
        change: &Change<'_>,
        remote: &[ManagedPolicy],
    ) -> Result<(), AgentsError> {
        let write = PolicyWrite {
            name: managed_name(&spec.id),
            description: remote_description(spec),
            source: spec.cedar.clone(),
            enabled: true,
        };

        match change {
            Change::Create => self.ferriskey.create_policy(instance, realm, &write).await,
            Change::Update { ferriskey_id } => {
                let Some(current) = remote
                    .iter()
                    .find(|found| found.policy.ferriskey_id == *ferriskey_id)
                else {
                    return Err(AgentsError::NotFound);
                };
                self.ferriskey
                    .update_policy(instance, realm, current, &write)
                    .await
            }
            Change::Keep { .. } => Ok(()),
            Change::Blocked(message) => Err(AgentsError::Rejected {
                message: message.clone(),
            }),
        }
    }

    async fn disable(&self, instance: &IdentityInstance, agents: &AgentsSpec) -> AgentsOutcome {
        let wanted = settled_off(agents.policies_revision);
        let previous = instance
            .status
            .as_ref()
            .and_then(|status| status.agents.as_ref());
        if previous == Some(&wanted) {
            return AgentsOutcome::Settled;
        }

        let realm = agents.realm.as_str();
        let canonical = self.environment.canonical_uri(instance);
        let mut tally = Tally::default();

        if let Err(error) = self.edge.remove_route(instance).await {
            tally.issue(error);
        }
        if let Err(error) = self.secrets.forget(instance).await {
            tally.issue(error);
        }
        if let Err(error) = self.rebuild_gateway_config().await {
            tally.issue(error);
        }

        match self.ferriskey.list_managed_policies(instance, realm).await {
            Ok(found) => {
                for policy in found {
                    let Some(id) = managed_id(&policy.name).map(ToOwned::to_owned) else {
                        continue;
                    };
                    let managed = ManagedPolicy {
                        autharie_id: id,
                        policy,
                    };
                    match self
                        .ferriskey
                        .delete_policy(instance, realm, &managed)
                        .await
                    {
                        Ok(()) | Err(AgentsError::NotFound) => {}
                        Err(error) => tally.issue(error),
                    }
                }
            }
            Err(AgentsError::NotFound) => {}
            Err(error) => tally.issue(error),
        }

        match self
            .ferriskey
            .remove_gateway_client(instance, realm, &canonical)
            .await
        {
            Ok(()) | Err(AgentsError::NotFound) => {}
            Err(error) => tally.issue(error),
        }

        match self.ferriskey.realm_settings(instance, realm).await {
            Ok(current) => {
                if let Some(wanted) = settings_without_resource(&current, &canonical)
                    && let Err(error) = self
                        .ferriskey
                        .update_realm_settings(instance, realm, &wanted)
                        .await
                {
                    tally.issue(error);
                }
            }
            Err(AgentsError::NotFound) => {}
            Err(error) => tally.issue(error),
        }

        let status = if tally.issues.is_empty() {
            wanted
        } else {
            AgentsStatus {
                phase: tally.phase(),
                message: tally.message(),
                ..settled_off(agents.policies_revision)
            }
        };
        self.record(instance, status, tally).await
    }

    async fn record(
        &self,
        instance: &IdentityInstance,
        status: AgentsStatus,
        mut tally: Tally,
    ) -> AgentsOutcome {
        let previous = instance
            .status
            .as_ref()
            .and_then(|status| status.agents.as_ref());

        if previous != Some(&status)
            && let Err(error) = self.status.patch_agents_status(instance, &status).await
        {
            tracing::warn!(error = %error, "could not report the agents status");
            tally.retry = true;
        }

        if status.phase == AgentsPhase::Ready && !tally.retry {
            AgentsOutcome::Settled
        } else {
            AgentsOutcome::Retry
        }
    }

    async fn rebuild_gateway_config(&self) -> Result<(), AgentsError> {
        let _guard = self.rebuild.lock().await;

        let instances = self.instances.list().await?;
        let mut upstreams = Vec::new();
        for instance in instances
            .iter()
            .filter(|instance| serves_agents(instance) && agents_enabled(instance))
        {
            let Some(secret) = self.secrets.load(instance).await? else {
                continue;
            };
            if let Some(upstream) = self.environment.upstream(instance, &secret) {
                upstreams.push(upstream);
            }
        }

        let document = render(&upstreams);
        if self.config.current().await?.as_deref() != Some(document.as_str()) {
            self.config.publish(&document).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Mutex as StdMutex;

    use autharie_crds::v1alpha::identity_instance::{IdentityInstanceSpec, IdentityInstanceStatus};

    use super::*;
    use crate::domain::OperatorError;
    use crate::domain::agents::test_spec;
    use crate::domain::agents::{GatewayClient, RealmSettings, RemotePolicy};

    const POLICY_A: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";
    const POLICY_B: &str = "7f1c2d3e-0000-4000-8000-00000000000b";
    const PERMIT: &str = "permit (principal, action, resource);";
    const CANONICAL: &str = "https://auth.acme.test/mcp";

    #[derive(Default)]
    struct FerriskeyState {
        calls: Vec<String>,
        authz: bool,
        settings: RealmSettings,
        client: Option<String>,
        policies: Vec<RemotePolicy>,
        next_id: u32,
        fail: HashMap<&'static str, AgentsError>,
        reject_source: Option<(String, AgentsError)>,
    }

    #[derive(Default)]
    struct FakeFerriskey {
        state: StdMutex<FerriskeyState>,
    }

    impl FakeFerriskey {
        fn with(&self, change: impl FnOnce(&mut FerriskeyState)) {
            change(&mut self.state.lock().expect("not poisoned"));
        }

        fn calls(&self) -> Vec<String> {
            self.state.lock().expect("not poisoned").calls.clone()
        }

        fn writes(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter(|call| {
                    ["create", "update", "delete", "set_", "ensure", "remove"]
                        .iter()
                        .any(|prefix| call.starts_with(prefix))
                })
                .collect()
        }

        fn enter(&self, call: &'static str) -> Result<(), AgentsError> {
            let mut state = self.state.lock().expect("not poisoned");
            state.calls.push(call.to_string());
            match state.fail.get(call) {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }
    }

    impl FerriskeyAgentsPort for FakeFerriskey {
        async fn set_authorization(
            &self,
            _instance: &IdentityInstance,
            _realm: &str,
            enabled: bool,
        ) -> Result<(), AgentsError> {
            self.enter("set_authorization")?;
            self.with(|state| state.authz = enabled);
            Ok(())
        }

        async fn realm_settings(
            &self,
            _instance: &IdentityInstance,
            _realm: &str,
        ) -> Result<RealmSettings, AgentsError> {
            self.enter("realm_settings")?;
            Ok(self.state.lock().expect("not poisoned").settings.clone())
        }

        async fn update_realm_settings(
            &self,
            _instance: &IdentityInstance,
            _realm: &str,
            settings: &RealmSettings,
        ) -> Result<(), AgentsError> {
            self.enter("update_realm_settings")?;
            self.with(|state| state.settings = settings.clone());
            Ok(())
        }

        async fn ensure_gateway_client(
            &self,
            _instance: &IdentityInstance,
            _realm: &str,
            _canonical_uri: &str,
        ) -> Result<GatewayClient, AgentsError> {
            self.enter("ensure_gateway_client")?;
            let mut state = self.state.lock().expect("not poisoned");
            let secret = state
                .client
                .get_or_insert_with(|| "client-secret".to_string())
                .clone();
            Ok(GatewayClient { secret })
        }

        async fn remove_gateway_client(
            &self,
            _instance: &IdentityInstance,
            _realm: &str,
            _canonical_uri: &str,
        ) -> Result<(), AgentsError> {
            self.enter("remove_gateway_client")?;
            self.with(|state| state.client = None);
            Ok(())
        }

        async fn list_managed_policies(
            &self,
            _instance: &IdentityInstance,
            _realm: &str,
        ) -> Result<Vec<RemotePolicy>, AgentsError> {
            self.enter("list_managed_policies")?;
            Ok(self.state.lock().expect("not poisoned").policies.clone())
        }

        async fn create_policy(
            &self,
            _instance: &IdentityInstance,
            _realm: &str,
            write: &PolicyWrite,
        ) -> Result<(), AgentsError> {
            self.enter("create_policy")?;
            let mut state = self.state.lock().expect("not poisoned");
            if let Some((needle, error)) = &state.reject_source
                && write.source.contains(needle.as_str())
            {
                return Err(error.clone());
            }
            state.next_id += 1;
            let ferriskey_id = format!("f{}", state.next_id);
            state.policies.push(RemotePolicy {
                ferriskey_id,
                name: write.name.clone(),
                description: Some(write.description.clone()),
                source: write.source.clone(),
                enabled: write.enabled,
            });
            Ok(())
        }

        async fn update_policy(
            &self,
            _instance: &IdentityInstance,
            _realm: &str,
            policy: &ManagedPolicy,
            write: &PolicyWrite,
        ) -> Result<(), AgentsError> {
            self.enter("update_policy")?;
            let mut state = self.state.lock().expect("not poisoned");
            if let Some(found) = state
                .policies
                .iter_mut()
                .find(|found| found.ferriskey_id == policy.policy.ferriskey_id)
            {
                found.source = write.source.clone();
                found.description = Some(write.description.clone());
                found.enabled = write.enabled;
            }
            Ok(())
        }

        async fn delete_policy(
            &self,
            _instance: &IdentityInstance,
            _realm: &str,
            policy: &ManagedPolicy,
        ) -> Result<(), AgentsError> {
            self.enter("delete_policy")?;
            self.with(|state| {
                state
                    .policies
                    .retain(|found| found.ferriskey_id != policy.policy.ferriskey_id)
            });
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeSecrets {
        stored: StdMutex<BTreeMap<String, String>>,
        stores: StdMutex<usize>,
    }

    impl AgentsSecretsPort for FakeSecrets {
        async fn load(&self, instance: &IdentityInstance) -> Result<Option<String>, AgentsError> {
            let name = instance.metadata.name.clone().unwrap_or_default();
            Ok(self
                .stored
                .lock()
                .expect("not poisoned")
                .get(&name)
                .cloned())
        }

        async fn store(
            &self,
            instance: &IdentityInstance,
            secret: &str,
        ) -> Result<(), AgentsError> {
            let name = instance.metadata.name.clone().unwrap_or_default();
            *self.stores.lock().expect("not poisoned") += 1;
            self.stored
                .lock()
                .expect("not poisoned")
                .insert(name, secret.to_string());
            Ok(())
        }

        async fn forget(&self, instance: &IdentityInstance) -> Result<(), AgentsError> {
            let name = instance.metadata.name.clone().unwrap_or_default();
            self.stored.lock().expect("not poisoned").remove(&name);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeStatus {
        patches: StdMutex<Vec<AgentsStatus>>,
        fail: StdMutex<bool>,
    }

    impl AgentsStatusRepository for FakeStatus {
        async fn patch_agents_status(
            &self,
            _instance: &IdentityInstance,
            status: &AgentsStatus,
        ) -> Result<(), OperatorError> {
            if *self.fail.lock().expect("not poisoned") {
                return Err(OperatorError::Kube {
                    message: "down".to_string(),
                });
            }
            self.patches
                .lock()
                .expect("not poisoned")
                .push(status.clone());
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeInstances {
        all: StdMutex<Vec<IdentityInstance>>,
    }

    impl AgentInstances for FakeInstances {
        async fn list(&self) -> Result<Vec<IdentityInstance>, AgentsError> {
            Ok(self.all.lock().expect("not poisoned").clone())
        }
    }

    #[derive(Default)]
    struct FakeConfig {
        document: StdMutex<Option<String>>,
        publishes: StdMutex<usize>,
    }

    impl GatewayConfigStore for FakeConfig {
        async fn current(&self) -> Result<Option<String>, AgentsError> {
            Ok(self.document.lock().expect("not poisoned").clone())
        }

        async fn publish(&self, document: &str) -> Result<(), AgentsError> {
            *self.publishes.lock().expect("not poisoned") += 1;
            *self.document.lock().expect("not poisoned") = Some(document.to_string());
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeEdge {
        calls: StdMutex<Vec<String>>,
        fail: StdMutex<Option<AgentsError>>,
    }

    impl AgentsEdge for FakeEdge {
        async fn apply_route(&self, instance: &IdentityInstance) -> Result<(), AgentsError> {
            self.calls.lock().expect("not poisoned").push(format!(
                "apply:{}",
                instance.metadata.name.clone().unwrap_or_default()
            ));
            match self.fail.lock().expect("not poisoned").clone() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        async fn remove_route(&self, instance: &IdentityInstance) -> Result<(), AgentsError> {
            self.calls.lock().expect("not poisoned").push(format!(
                "remove:{}",
                instance.metadata.name.clone().unwrap_or_default()
            ));
            Ok(())
        }
    }

    type Reconciler = AgentsReconciler<
        FakeFerriskey,
        FakeSecrets,
        FakeStatus,
        FakeInstances,
        FakeConfig,
        FakeEdge,
    >;

    struct Harness {
        ferriskey: Arc<FakeFerriskey>,
        secrets: Arc<FakeSecrets>,
        status: Arc<FakeStatus>,
        instances: Arc<FakeInstances>,
        config: Arc<FakeConfig>,
        edge: Arc<FakeEdge>,
        reconciler: Reconciler,
    }

    fn harness() -> Harness {
        let ferriskey = Arc::new(FakeFerriskey::default());
        let secrets = Arc::new(FakeSecrets::default());
        let status = Arc::new(FakeStatus::default());
        let instances = Arc::new(FakeInstances::default());
        let config = Arc::new(FakeConfig::default());
        let edge = Arc::new(FakeEdge::default());
        let reconciler = AgentsReconciler::new(
            ferriskey.clone(),
            secrets.clone(),
            status.clone(),
            instances.clone(),
            config.clone(),
            edge.clone(),
            AgentsEnvironment::new("autharie-system", None),
        );
        Harness {
            ferriskey,
            secrets,
            status,
            instances,
            config,
            edge,
            reconciler,
        }
    }

    fn policy(id: &str, cedar: &str) -> AgentPolicySpec {
        AgentPolicySpec {
            id: id.to_string(),
            name: format!("policy {id}"),
            cedar: cedar.to_string(),
        }
    }

    fn agents(enabled: bool, policies: Vec<AgentPolicySpec>) -> AgentsSpec {
        AgentsSpec {
            enabled,
            realm: "acme".to_string(),
            policies,
            policies_revision: 3,
        }
    }

    fn instance(name: &str, host: &str, spec: Option<AgentsSpec>) -> IdentityInstance {
        let base: IdentityInstanceSpec = test_spec(host);
        let mut instance = IdentityInstance::new(name, base);
        instance.metadata.namespace = Some("tenant-a".to_string());
        instance.spec.agents = spec;
        instance.status = Some(IdentityInstanceStatus {
            ready: true,
            ..Default::default()
        });
        instance
    }

    fn acme(spec: Option<AgentsSpec>) -> IdentityInstance {
        instance("deployment-1", "auth.acme.test", spec)
    }

    fn with_status(mut instance: IdentityInstance, status: AgentsStatus) -> IdentityInstance {
        instance.status.get_or_insert_with(Default::default).agents = Some(status);
        instance
    }

    fn last_status(harness: &Harness) -> AgentsStatus {
        harness
            .status
            .patches
            .lock()
            .expect("not poisoned")
            .last()
            .cloned()
            .expect("a status was written")
    }

    fn written_config(harness: &Harness) -> String {
        harness
            .config
            .document
            .lock()
            .expect("not poisoned")
            .clone()
            .expect("a config was written")
    }

    #[tokio::test]
    async fn enabling_prepares_the_realm_the_client_the_policies_the_config_and_the_route() {
        let h = harness();
        let instance = acme(Some(agents(true, vec![policy(POLICY_A, PERMIT)])));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(instance.clone());

        let outcome = h.reconciler.reconcile(&instance).await;

        assert_eq!(outcome, AgentsOutcome::Settled);
        let state = h.ferriskey.state.lock().expect("not poisoned");
        assert!(state.authz);
        assert_eq!(
            state.settings.allowed_resources,
            vec![CANONICAL.to_string()]
        );
        assert!(state.settings.cimd_enabled);
        assert!(!state.settings.dcr_enabled);
        assert_eq!(state.policies.len(), 1);
        assert_eq!(state.policies[0].name, format!("autharie:{POLICY_A}"));
        assert_eq!(state.policies[0].source, PERMIT);
        assert_eq!(
            state.policies[0].description.as_deref(),
            Some(format!("policy {POLICY_A}").as_str())
        );
        drop(state);
        assert_eq!(
            h.secrets
                .stored
                .lock()
                .expect("not poisoned")
                .get("deployment-1"),
            Some(&"client-secret".to_string())
        );
        assert!(written_config(&h).contains("canonical_uri = \"https://auth.acme.test/mcp\""));
        assert!(written_config(&h).contains("client_secret = \"client-secret\""));
        assert_eq!(
            *h.edge.calls.lock().expect("not poisoned"),
            vec!["apply:deployment-1".to_string()]
        );
    }

    #[tokio::test]
    async fn enabling_reports_a_ready_status_with_each_policy() {
        let h = harness();
        let instance = acme(Some(agents(
            true,
            vec![policy(POLICY_A, PERMIT), policy(POLICY_B, PERMIT)],
        )));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(instance.clone());

        h.reconciler.reconcile(&instance).await;

        let status = last_status(&h);
        assert_eq!(status.phase, AgentsPhase::Ready);
        assert!(status.gateway_client_ready);
        assert_eq!(status.observed_revision, 3);
        assert_eq!(status.mcp_url.as_deref(), Some(CANONICAL));
        assert_eq!(status.message, None);
        assert_eq!(status.policies.len(), 2);
        assert!(
            status
                .policies
                .iter()
                .all(|policy| policy.phase == AgentPolicyPhase::Applied)
        );
    }

    #[tokio::test]
    async fn a_second_pass_over_an_applied_state_writes_nothing() {
        let h = harness();
        let first = acme(Some(agents(true, vec![policy(POLICY_A, PERMIT)])));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(first.clone());
        h.reconciler.reconcile(&first).await;
        let applied = last_status(&h);
        let patches = h.status.patches.lock().expect("not poisoned").len();
        let publishes = *h.config.publishes.lock().expect("not poisoned");
        let stores = *h.secrets.stores.lock().expect("not poisoned");
        h.ferriskey.with(|state| state.calls.clear());

        let second = with_status(first, applied);
        let outcome = h.reconciler.reconcile(&second).await;

        assert_eq!(outcome, AgentsOutcome::Settled);
        assert_eq!(
            h.status.patches.lock().expect("not poisoned").len(),
            patches
        );
        assert_eq!(*h.config.publishes.lock().expect("not poisoned"), publishes);
        assert_eq!(*h.secrets.stores.lock().expect("not poisoned"), stores);
        assert!(
            !h.ferriskey
                .writes()
                .iter()
                .any(|call| call.contains("policy")),
            "{:?}",
            h.ferriskey.writes()
        );
    }

    #[tokio::test]
    async fn a_changed_policy_is_updated_and_a_removed_one_deleted() {
        let h = harness();
        let first = acme(Some(agents(
            true,
            vec![policy(POLICY_A, PERMIT), policy(POLICY_B, PERMIT)],
        )));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(first.clone());
        h.reconciler.reconcile(&first).await;
        h.ferriskey.with(|state| state.calls.clear());

        let forbid = "forbid (principal, action, resource);";
        let second = acme(Some(agents(true, vec![policy(POLICY_A, forbid)])));
        h.reconciler.reconcile(&second).await;

        let state = h.ferriskey.state.lock().expect("not poisoned");
        assert_eq!(state.policies.len(), 1);
        assert_eq!(state.policies[0].source, forbid);
        assert!(state.calls.contains(&"update_policy".to_string()));
        assert!(state.calls.contains(&"delete_policy".to_string()));
    }

    #[tokio::test]
    async fn policies_nobody_at_autharie_wrote_are_left_alone() {
        let h = harness();
        h.ferriskey.with(|state| {
            for (index, name) in [
                "by-hand",
                "autharie:not-a-uuid",
                "x-autharie:0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
            ]
            .iter()
            .enumerate()
            {
                state.policies.push(RemotePolicy {
                    ferriskey_id: format!("hand{index}"),
                    name: name.to_string(),
                    description: None,
                    source: PERMIT.to_string(),
                    enabled: true,
                });
            }
        });
        let instance = acme(Some(agents(true, Vec::new())));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(instance.clone());

        h.reconciler.reconcile(&instance).await;

        let state = h.ferriskey.state.lock().expect("not poisoned");
        assert_eq!(state.policies.len(), 3);
        assert!(!state.calls.contains(&"delete_policy".to_string()));
    }

    #[tokio::test]
    async fn a_policy_ferriskey_refuses_is_invalid_with_its_message_and_the_rest_apply() {
        let h = harness();
        h.ferriskey.with(|state| {
            state.reject_source = Some((
                "BAD".to_string(),
                AgentsError::Rejected {
                    message: "FerrisKey answered 422: unexpected token at line 1".to_string(),
                },
            ));
        });
        let instance = acme(Some(agents(
            true,
            vec![policy(POLICY_A, "BAD"), policy(POLICY_B, PERMIT)],
        )));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(instance.clone());

        let outcome = h.reconciler.reconcile(&instance).await;

        let status = last_status(&h);
        assert_eq!(status.policies[0].phase, AgentPolicyPhase::Invalid);
        assert_eq!(
            status.policies[0].message.as_deref(),
            Some("FerrisKey answered 422: unexpected token at line 1")
        );
        assert_eq!(status.policies[1].phase, AgentPolicyPhase::Applied);
        assert_eq!(status.phase, AgentsPhase::Ready);
        assert_eq!(outcome, AgentsOutcome::Settled);
    }

    #[tokio::test]
    async fn a_policy_that_could_not_be_written_for_now_fails_and_is_tried_again() {
        let h = harness();
        h.ferriskey.with(|state| {
            state.reject_source = Some((
                "SLOW".to_string(),
                AgentsError::Retry {
                    message: "FerrisKey is unreachable".to_string(),
                },
            ));
        });
        let instance = acme(Some(agents(true, vec![policy(POLICY_A, "SLOW")])));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(instance.clone());

        let outcome = h.reconciler.reconcile(&instance).await;

        let status = last_status(&h);
        assert_eq!(status.policies[0].phase, AgentPolicyPhase::Failed);
        assert_eq!(status.phase, AgentsPhase::Applying);
        assert_eq!(outcome, AgentsOutcome::Retry);
    }

    #[tokio::test]
    async fn a_policy_with_an_id_that_is_not_a_uuid_is_invalid_and_never_written() {
        let h = harness();
        let instance = acme(Some(agents(true, vec![policy("not-a-uuid", PERMIT)])));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(instance.clone());

        h.reconciler.reconcile(&instance).await;

        assert_eq!(last_status(&h).policies[0].phase, AgentPolicyPhase::Invalid);
        assert!(
            h.ferriskey
                .state
                .lock()
                .expect("not poisoned")
                .policies
                .is_empty()
        );
    }

    #[tokio::test]
    async fn authorization_not_yet_available_fails_the_policies_without_failing_the_client() {
        let h = harness();
        h.ferriskey.with(|state| {
            state
                .fail
                .insert("list_managed_policies", AgentsError::NotFound);
        });
        let instance = acme(Some(agents(true, vec![policy(POLICY_A, PERMIT)])));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(instance.clone());

        let outcome = h.reconciler.reconcile(&instance).await;

        let status = last_status(&h);
        assert!(status.gateway_client_ready);
        assert_eq!(status.policies[0].phase, AgentPolicyPhase::Failed);
        assert_eq!(status.phase, AgentsPhase::Applying);
        assert_eq!(outcome, AgentsOutcome::Retry);
    }

    #[tokio::test]
    async fn an_unreachable_ferriskey_leaves_the_client_unready_and_the_edge_untouched() {
        let h = harness();
        h.ferriskey.with(|state| {
            state.fail.insert(
                "ensure_gateway_client",
                AgentsError::Retry {
                    message: "FerrisKey is unreachable".to_string(),
                },
            );
        });
        let instance = acme(Some(agents(true, Vec::new())));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(instance.clone());

        let outcome = h.reconciler.reconcile(&instance).await;

        let status = last_status(&h);
        assert!(!status.gateway_client_ready);
        assert_eq!(status.phase, AgentsPhase::Applying);
        assert_eq!(status.message.as_deref(), Some("FerrisKey is unreachable"));
        assert_eq!(outcome, AgentsOutcome::Retry);
        assert!(h.edge.calls.lock().expect("not poisoned").is_empty());
        assert!(h.config.document.lock().expect("not poisoned").is_none());
    }

    #[tokio::test]
    async fn a_refusal_that_will_come_again_fails_the_whole_thing() {
        let h = harness();
        h.ferriskey.with(|state| {
            state.fail.insert(
                "realm_settings",
                AgentsError::Rejected {
                    message: "FerrisKey answered 403".to_string(),
                },
            );
        });
        let instance = acme(Some(agents(true, Vec::new())));

        let outcome = h.reconciler.reconcile(&instance).await;

        let status = last_status(&h);
        assert_eq!(status.phase, AgentsPhase::Failed);
        assert!(!status.gateway_client_ready);
        assert_eq!(outcome, AgentsOutcome::Retry);
        assert!(
            !h.ferriskey
                .calls()
                .contains(&"ensure_gateway_client".to_string())
        );
    }

    #[tokio::test]
    async fn a_route_that_cannot_be_applied_is_reported_and_retried() {
        let h = harness();
        *h.edge.fail.lock().expect("not poisoned") = Some(AgentsError::Retry {
            message: "the cluster said no".to_string(),
        });
        let instance = acme(Some(agents(true, Vec::new())));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(instance.clone());

        let outcome = h.reconciler.reconcile(&instance).await;

        let status = last_status(&h);
        assert!(status.gateway_client_ready);
        assert_eq!(status.phase, AgentsPhase::Applying);
        assert_eq!(status.message.as_deref(), Some("the cluster said no"));
        assert_eq!(outcome, AgentsOutcome::Retry);
    }

    #[tokio::test]
    async fn a_status_that_cannot_be_written_is_retried() {
        let h = harness();
        *h.status.fail.lock().expect("not poisoned") = true;
        let instance = acme(Some(agents(true, Vec::new())));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(instance.clone());

        assert_eq!(
            h.reconciler.reconcile(&instance).await,
            AgentsOutcome::Retry
        );
    }

    #[tokio::test]
    async fn the_gateway_config_holds_every_enabled_instance_and_no_other() {
        let h = harness();
        let first = acme(Some(agents(true, Vec::new())));
        let second = instance(
            "deployment-2",
            "auth.other.test",
            Some(agents(true, Vec::new())),
        );
        let off = instance(
            "deployment-3",
            "auth.off.test",
            Some(agents(false, Vec::new())),
        );
        let plain = instance("deployment-4", "auth.plain.test", None);
        let no_secret = instance(
            "deployment-5",
            "auth.fresh.test",
            Some(agents(true, Vec::new())),
        );
        let mut deleting = instance(
            "deployment-6",
            "auth.gone.test",
            Some(agents(true, Vec::new())),
        );
        deleting.metadata.deletion_timestamp = Some(
            k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(chrono::Utc::now()),
        );
        {
            let mut stored = h.secrets.stored.lock().expect("not poisoned");
            for name in [
                "deployment-1",
                "deployment-2",
                "deployment-3",
                "deployment-4",
                "deployment-6",
            ] {
                stored.insert(name.to_string(), format!("secret-of-{name}"));
            }
        }
        *h.instances.all.lock().expect("not poisoned") =
            vec![first.clone(), second, off, plain, no_secret, deleting];

        h.reconciler.reconcile(&first).await;

        let config = written_config(&h);
        assert_eq!(config.matches("[[upstreams]]").count(), 2);
        assert!(config.contains("auth.acme.test/mcp"));
        assert!(config.contains("auth.other.test/mcp"));
        assert!(!config.contains("auth.off.test"));
        assert!(!config.contains("auth.plain.test"));
        assert!(!config.contains("auth.fresh.test"));
        assert!(!config.contains("auth.gone.test"));
    }

    #[tokio::test]
    async fn disabling_removes_the_route_the_client_the_policies_and_the_config_entry() {
        let h = harness();
        let on = acme(Some(agents(true, vec![policy(POLICY_A, PERMIT)])));
        h.instances
            .all
            .lock()
            .expect("not poisoned")
            .push(on.clone());
        h.reconciler.reconcile(&on).await;
        let applied = last_status(&h);
        h.ferriskey.with(|state| {
            state.policies.push(RemotePolicy {
                ferriskey_id: "hand".to_string(),
                name: "by-hand".to_string(),
                description: None,
                source: PERMIT.to_string(),
                enabled: true,
            });
        });

        let off = with_status(acme(Some(agents(false, Vec::new()))), applied);
        *h.instances.all.lock().expect("not poisoned") = vec![off.clone()];
        let outcome = h.reconciler.reconcile(&off).await;

        assert_eq!(outcome, AgentsOutcome::Settled);
        let state = h.ferriskey.state.lock().expect("not poisoned");
        assert_eq!(state.policies.len(), 1);
        assert_eq!(state.policies[0].name, "by-hand");
        assert_eq!(state.client, None);
        assert!(state.settings.allowed_resources.is_empty());
        drop(state);
        assert!(h.secrets.stored.lock().expect("not poisoned").is_empty());
        assert!(
            h.edge
                .calls
                .lock()
                .expect("not poisoned")
                .contains(&"remove:deployment-1".to_string())
        );
        assert!(!written_config(&h).contains("[[upstreams]]"));
        assert_eq!(last_status(&h), settled_off(3));
    }

    #[tokio::test]
    async fn a_settled_off_instance_costs_nothing_to_look_at_again() {
        let h = harness();
        let off = with_status(acme(Some(agents(false, Vec::new()))), settled_off(3));

        let outcome = h.reconciler.reconcile(&off).await;

        assert_eq!(outcome, AgentsOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
        assert!(h.edge.calls.lock().expect("not poisoned").is_empty());
        assert!(h.status.patches.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn turning_off_what_was_never_on_settles_even_without_the_authorization_api() {
        let h = harness();
        h.ferriskey.with(|state| {
            state
                .fail
                .insert("list_managed_policies", AgentsError::NotFound);
        });
        let off = acme(Some(agents(false, Vec::new())));

        let outcome = h.reconciler.reconcile(&off).await;

        assert_eq!(outcome, AgentsOutcome::Settled);
        assert_eq!(last_status(&h), settled_off(3));
    }

    #[tokio::test]
    async fn a_cleanup_that_fails_is_not_settled_and_is_tried_again() {
        let h = harness();
        h.ferriskey.with(|state| {
            state.fail.insert(
                "remove_gateway_client",
                AgentsError::Retry {
                    message: "FerrisKey is unreachable".to_string(),
                },
            );
        });
        let off = acme(Some(agents(false, Vec::new())));

        let outcome = h.reconciler.reconcile(&off).await;

        let status = last_status(&h);
        assert_eq!(status.phase, AgentsPhase::Applying);
        assert_ne!(status, settled_off(3));
        assert_eq!(outcome, AgentsOutcome::Retry);
    }

    #[tokio::test]
    async fn nothing_happens_for_an_instance_without_agents() {
        let h = harness();

        let outcome = h.reconciler.reconcile(&acme(None)).await;

        assert_eq!(outcome, AgentsOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
        assert!(h.status.patches.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn nothing_happens_while_the_instance_is_not_ready_or_stopped_or_not_ferriskey() {
        let h = harness();
        let enabled = Some(agents(true, Vec::new()));
        let mut not_ready = acme(enabled.clone());
        not_ready.status = None;
        let mut stopped = acme(enabled.clone());
        stopped.spec.stopped = true;
        let mut keycloak = acme(enabled);
        keycloak.spec.provider = IdentityProvider::Keycloak;

        for instance in [not_ready, stopped, keycloak] {
            assert_eq!(
                h.reconciler.reconcile(&instance).await,
                AgentsOutcome::Settled
            );
        }

        assert!(h.ferriskey.calls().is_empty());
        assert!(h.edge.calls.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn a_deleted_instance_loses_its_route_and_its_config_entry() {
        let h = harness();
        let mut gone = acme(Some(agents(true, Vec::new())));
        gone.metadata.deletion_timestamp = Some(
            k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(chrono::Utc::now()),
        );
        h.secrets
            .stored
            .lock()
            .expect("not poisoned")
            .insert("deployment-1".to_string(), "s".to_string());
        *h.instances.all.lock().expect("not poisoned") = vec![gone.clone()];

        h.reconciler.cleanup(&gone).await;

        assert_eq!(
            *h.edge.calls.lock().expect("not poisoned"),
            vec!["remove:deployment-1".to_string()]
        );
        assert!(!written_config(&h).contains("[[upstreams]]"));
        assert!(h.ferriskey.calls().is_empty());
    }
}
