use autharie_core::deployments::Deployment;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema, PartialEq)]
pub struct DeploymentView {
    #[serde(flatten)]
    pub deployment: Deployment,
    /// The public hostname, from the same function DNS and the reachability
    /// probe use. `null` when this installation has no DNS zone configured.
    pub hostname: Option<String>,
}

// Added here rather than on the domain entity: the hostname depends on the
// installation's zone, which the entity does not know.
impl DeploymentView {
    pub fn new(deployment: Deployment, zone: Option<&str>) -> Self {
        let hostname = zone.map(|zone| autharie_core::dns::hostname_for(&deployment.name.0, zone));

        Self {
            deployment,
            hostname,
        }
    }
}
