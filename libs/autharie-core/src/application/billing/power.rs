use autharie_domain::{
    CoreError, billing::ports::DeploymentPower, deployments::DeploymentId,
    organisation::OrganisationId,
};
use tracing::warn;

pub(crate) struct StoppingNothing;

impl DeploymentPower for StoppingNothing {
    async fn stop_billable(
        &self,
        organisation_id: &OrganisationId,
    ) -> Result<Vec<DeploymentId>, CoreError> {
        warn!(
            organisation = %organisation_id,
            "billing cannot stop deployments until #235 ships"
        );

        Ok(Vec::new())
    }

    async fn start(
        &self,
        _deployment_ids: &[DeploymentId],
    ) -> Result<Vec<DeploymentId>, CoreError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn it_stops_nothing() {
        let stopped = StoppingNothing
            .stop_billable(&OrganisationId::new())
            .await
            .expect("nothing to fail");

        assert!(stopped.is_empty());
    }

    #[tokio::test]
    async fn it_starts_nothing_even_when_asked_to_start_something() {
        let started = StoppingNothing
            .start(&[DeploymentId(Uuid::new_v4())])
            .await
            .expect("nothing to fail");

        assert!(started.is_empty());
    }
}
