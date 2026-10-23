use std::collections::{BTreeMap, BTreeSet};

use autharie_crds::v1alpha::identity_instance::{
    AgentClientStatus, AgentCredentialStatus, AgentsSpec, IdentityInstance,
};

use super::AgentsError;
use super::ports::{
    AgentInstances, AgentsEdge, AgentsSecretsPort, AgentsStatusRepository, FerriskeyAgentsPort,
    FerriskeyClientsPort, GatewayConfigStore,
};
use super::registry::{
    AgentClient, AgentClientId, RequestKind, SealRequest, agents_among, plan, report,
    seal_requests, still_asked,
};
use super::sealing::SealingKey;
use super::service::{AgentsReconciler, Tally};

#[derive(Debug, Default)]
pub(super) struct RegistryReport {
    pub(super) clients: Vec<AgentClientStatus>,
    pub(super) truncated: bool,
    pub(super) credentials: Vec<AgentCredentialStatus>,
    pub(super) disabled: Vec<String>,
}

fn rejected(message: impl Into<String>) -> AgentsError {
    AgentsError::Rejected {
        message: message.into(),
    }
}

impl<F, S, R, L, G, E> AgentsReconciler<F, S, R, L, G, E>
where
    F: FerriskeyAgentsPort + FerriskeyClientsPort,
    S: AgentsSecretsPort,
    R: AgentsStatusRepository,
    L: AgentInstances,
    G: GatewayConfigStore,
    E: AgentsEdge,
{
    pub(super) async fn sync_registry(
        &self,
        instance: &IdentityInstance,
        agents: &AgentsSpec,
        tally: &mut Tally,
    ) -> RegistryReport {
        let realm = agents.realm.as_str();
        let previous = instance
            .status
            .as_ref()
            .and_then(|status| status.agents.as_ref());
        let requests = seal_requests(&agents.autonomous);
        let mut credentials = previous
            .map(|status| still_asked(&status.credentials, &requests))
            .unwrap_or_default();

        let remote = match self.ferriskey.list_clients(instance, realm).await {
            Ok(remote) => remote,
            Err(error) => {
                tally.issue(error);
                return RegistryReport {
                    clients: previous
                        .map(|status| status.clients.clone())
                        .unwrap_or_default(),
                    truncated: previous.is_some_and(|status| status.clients_truncated),
                    credentials,
                    disabled: previous
                        .map(|status| status.disabled_clients.clone())
                        .unwrap_or_default(),
                };
            }
        };

        let mut found = agents_among(remote);
        let mut disabled: BTreeSet<String> = previous
            .map(|status| {
                status
                    .disabled_clients
                    .iter()
                    .filter(|id| found.iter().any(|agent| agent.client_id() == id.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        self.apply_plan(instance, agents, &mut found, &mut disabled, tally)
            .await;

        for request in &requests {
            if credentials
                .iter()
                .any(|credential| credential.request_id == request.request_id)
            {
                continue;
            }
            match self
                .seal(instance, realm, agents, request, &mut found, &mut disabled)
                .await
            {
                Ok(credential) => credentials.push(credential),
                Err(error) => tally.issue(error),
            }
        }

        let (clients, truncated) = report(&found);
        RegistryReport {
            clients,
            truncated,
            credentials,
            disabled: disabled.into_iter().collect(),
        }
    }

    async fn apply_plan(
        &self,
        instance: &IdentityInstance,
        agents: &AgentsSpec,
        found: &mut Vec<AgentClient>,
        disabled: &mut BTreeSet<String>,
        tally: &mut Tally,
    ) {
        let realm = agents.realm.as_str();
        let steps = plan(found, &agents.access, &agents.deleted_agents, disabled);
        let mut gone: BTreeSet<String> = BTreeSet::new();
        let mut flipped: BTreeMap<String, bool> = BTreeMap::new();

        for client in &steps.delete {
            match self.ferriskey.delete_client(instance, realm, client).await {
                Ok(()) | Err(AgentsError::NotFound) => {
                    gone.insert(client.client_id().to_string());
                }
                Err(error) => tally.issue(error),
            }
        }
        for client in &steps.disable {
            match self
                .ferriskey
                .set_client_enabled(instance, realm, client, false)
                .await
            {
                Ok(()) => {
                    flipped.insert(client.client_id().to_string(), false);
                    disabled.insert(client.client_id().to_string());
                }
                Err(error) => tally.issue(error),
            }
        }
        for client in &steps.adopt {
            disabled.insert(client.client_id().to_string());
        }
        for client in &steps.enable {
            match self
                .ferriskey
                .set_client_enabled(instance, realm, client, true)
                .await
            {
                Ok(()) => {
                    flipped.insert(client.client_id().to_string(), true);
                    disabled.remove(client.client_id());
                }
                Err(error) => tally.issue(error),
            }
        }

        found.retain(|client| !gone.contains(client.client_id()));
        for client in found.iter_mut() {
            if let Some(enabled) = flipped.get(client.client_id()) {
                *client = client.clone().with_enabled(*enabled);
            }
        }
        disabled.retain(|id| !gone.contains(id));
    }

    async fn seal(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        agents: &AgentsSpec,
        request: &SealRequest<'_>,
        found: &mut Vec<AgentClient>,
        disabled: &mut BTreeSet<String>,
    ) -> Result<AgentCredentialStatus, AgentsError> {
        let key = SealingKey::try_from(request.key).map_err(|error| rejected(error.to_string()))?;
        let id = AgentClientId::autonomous(request.client_id)
            .map_err(|_| rejected("an autonomous agent needs an agent- client id"))?;
        let name = agents
            .autonomous
            .iter()
            .find(|agent| agent.client_id == request.client_id)
            .map_or(request.client_id, |agent| agent.name.as_str());

        let existing = found
            .iter()
            .position(|client| client.client_id() == id.as_str());
        if request.kind == RequestKind::Rotate
            && let Some(position) = existing
        {
            let old = found.remove(position);
            self.ferriskey.delete_client(instance, realm, &old).await?;
            disabled.remove(id.as_str());
        }

        let client = match found
            .iter()
            .find(|client| client.client_id() == id.as_str())
        {
            Some(client) => client.clone(),
            None => {
                let created = self
                    .ferriskey
                    .create_autonomous_client(instance, realm, &id, name)
                    .await?;
                found.push(created.clone());
                created
            }
        };

        let secret = self
            .ferriskey
            .client_secret(instance, realm, &client)
            .await?;
        let sealed = key
            .seal(secret.expose())
            .map_err(|error| rejected(error.to_string()))?;

        Ok(AgentCredentialStatus {
            request_id: request.request_id.to_string(),
            client_id: id.as_str().to_string(),
            sealed,
        })
    }
}
