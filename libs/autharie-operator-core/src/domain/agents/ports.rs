use std::future::Future;

use autharie_crds::v1alpha::identity_instance::{AgentsStatus, IdentityInstance};

use super::{AgentsError, GatewayClient, ManagedPolicy, PolicyWrite, RealmSettings, RemotePolicy};
use crate::domain::OperatorError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentsOutcome {
    Settled,
    Retry,
}

pub trait IdentityInstanceAgents: Send + Sync {
    fn reconcile(&self, instance: &IdentityInstance) -> impl Future<Output = AgentsOutcome> + Send;

    fn cleanup(&self, instance: &IdentityInstance) -> impl Future<Output = ()> + Send;
}

pub trait FerriskeyAgentsPort: Send + Sync {
    fn set_authorization(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        enabled: bool,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;

    fn realm_settings(
        &self,
        instance: &IdentityInstance,
        realm: &str,
    ) -> impl Future<Output = Result<RealmSettings, AgentsError>> + Send;

    fn update_realm_settings(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        settings: &RealmSettings,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;

    fn ensure_gateway_client(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        canonical_uri: &str,
    ) -> impl Future<Output = Result<GatewayClient, AgentsError>> + Send;

    fn remove_gateway_client(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        canonical_uri: &str,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;

    fn list_managed_policies(
        &self,
        instance: &IdentityInstance,
        realm: &str,
    ) -> impl Future<Output = Result<Vec<RemotePolicy>, AgentsError>> + Send;

    fn create_policy(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        write: &PolicyWrite,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;

    fn update_policy(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        policy: &ManagedPolicy,
        write: &PolicyWrite,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;

    fn delete_policy(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        policy: &ManagedPolicy,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;
}

pub trait AgentsSecretsPort: Send + Sync {
    fn load(
        &self,
        instance: &IdentityInstance,
    ) -> impl Future<Output = Result<Option<String>, AgentsError>> + Send;

    fn store(
        &self,
        instance: &IdentityInstance,
        secret: &str,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;

    fn forget(
        &self,
        instance: &IdentityInstance,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;
}

pub trait AgentsStatusRepository: Send + Sync {
    fn patch_agents_status(
        &self,
        instance: &IdentityInstance,
        status: &AgentsStatus,
    ) -> impl Future<Output = Result<(), OperatorError>> + Send;
}

pub trait AgentInstances: Send + Sync {
    fn list(&self) -> impl Future<Output = Result<Vec<IdentityInstance>, AgentsError>> + Send;
}

pub trait GatewayConfigStore: Send + Sync {
    fn current(&self) -> impl Future<Output = Result<Option<String>, AgentsError>> + Send;

    fn publish(&self, document: &str) -> impl Future<Output = Result<(), AgentsError>> + Send;
}

pub trait AgentsEdge: Send + Sync {
    fn apply_route(
        &self,
        instance: &IdentityInstance,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;

    fn remove_route(
        &self,
        instance: &IdentityInstance,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;
}
