use autharie_iggy::iggy::prelude::{IggyClient, IggyMessage, MessageClient, Partitioning};
use autharie_iggy::{OUTCOMES_TOPIC, PARTITION, STREAM, bounded, named};
use serde_json::to_vec;
use tracing::info;

use crate::domain::entities::agents_outcome::AgentsOutcomeReport;
use crate::domain::entities::outcome::DeploymentOutcomeReport;
use crate::domain::entities::portal_theme_outcome::PortalThemeOutcomeReport;
use crate::domain::entities::smtp_outcome::SmtpOutcomeReport;
use crate::domain::entities::webhooks_outcome::WebhooksOutcomeReport;
use crate::domain::error::GenesisError;
use crate::domain::ports::{
    AgentsOutcomePublisher, BoxFuture, OutcomePublisher, PortalThemeOutcomePublisher,
    SmtpOutcomePublisher, WebhookOutcomePublisher,
};

/// Publishes outcomes to the outcomes topic, where every Herald reads them.
///
/// A topic of its own rather than the one actions travel on: an outcome that
/// reached Genesis would fail to parse as an action and be retried for ever.
pub struct IggyOutcomePublisher {
    // One client for the life of the process: it reconnects and signs in
    // again on its own when the broker restarts or drops the connection.
    client: IggyClient,
}

impl IggyOutcomePublisher {
    pub async fn connect(url: &str) -> Result<Self, GenesisError> {
        let client = autharie_iggy::connect_with_retry(url, autharie_iggy::DEFAULT_BUDGET)
            .await
            .map_err(|e| GenesisError::MessageBus {
                message: format!("failed to reach the broker: {e}"),
            })?;

        // Declared here as well as by Herald: whichever starts first must
        // find the topic.
        autharie_iggy::ensure_topic(&client, OUTCOMES_TOPIC)
            .await
            .map_err(|e| GenesisError::MessageBus {
                message: format!("failed to declare the '{OUTCOMES_TOPIC}' topic: {e}"),
            })?;

        Ok(Self { client })
    }
}

impl IggyOutcomePublisher {
    async fn send(&self, payload: Vec<u8>) -> Result<(), GenesisError> {
        let stream = named(STREAM).map_err(|e| GenesisError::MessageBus {
            message: e.to_string(),
        })?;
        let topic = named(OUTCOMES_TOPIC).map_err(|e| GenesisError::MessageBus {
            message: e.to_string(),
        })?;

        bounded(
            "a publish",
            self.client.send_messages(
                &stream,
                &topic,
                &Partitioning::partition_id(PARTITION),
                &mut [IggyMessage::from(payload)],
            ),
        )
        .await
        .map_err(|e| GenesisError::MessageBus {
            message: format!("failed to publish outcome: {e}"),
        })?;

        Ok(())
    }
}

impl OutcomePublisher for IggyOutcomePublisher {
    fn publish<'a>(
        &'a self,
        report: DeploymentOutcomeReport,
    ) -> BoxFuture<'a, Result<(), GenesisError>> {
        Box::pin(async move {
            let payload = to_vec(&report).map_err(|e| GenesisError::MessageBus {
                message: format!("failed to serialise outcome: {e}"),
            })?;

            info!(
                deployment_id = %report.deployment_id,
                outcome = %report.outcome,
                "reporting outcome"
            );

            self.send(payload).await
        })
    }
}

impl WebhookOutcomePublisher for IggyOutcomePublisher {
    async fn publish_webhooks<'a>(
        &'a self,
        report: &'a WebhooksOutcomeReport,
    ) -> Result<(), GenesisError> {
        let payload = to_vec(report).map_err(|e| GenesisError::MessageBus {
            message: format!("failed to serialise outcome: {e}"),
        })?;

        info!(
            deployment_id = %report.deployment_id,
            revision = report.revision,
            webhooks = report.webhooks.len(),
            secrets = report.secrets.len(),
            requests = report.requests.len(),
            "reporting webhooks outcome"
        );

        self.send(payload).await
    }
}

impl AgentsOutcomePublisher for IggyOutcomePublisher {
    async fn publish_agents<'a>(
        &'a self,
        report: &'a AgentsOutcomeReport,
    ) -> Result<(), GenesisError> {
        let payload = to_vec(report).map_err(|e| GenesisError::MessageBus {
            message: format!("failed to serialise outcome: {e}"),
        })?;

        info!(
            deployment_id = %report.deployment_id,
            revision = report.revision,
            phase = %report.phase,
            policies = report.policies.len(),
            "reporting agents outcome"
        );

        self.send(payload).await
    }
}

impl SmtpOutcomePublisher for IggyOutcomePublisher {
    async fn publish_smtp<'a>(&'a self, report: &'a SmtpOutcomeReport) -> Result<(), GenesisError> {
        let payload = to_vec(report).map_err(|e| GenesisError::MessageBus {
            message: format!("failed to serialise outcome: {e}"),
        })?;

        info!(
            deployment_id = %report.deployment_id,
            revision = report.revision,
            realm = %report.realm,
            "reporting smtp outcome"
        );

        self.send(payload).await
    }
}

impl PortalThemeOutcomePublisher for IggyOutcomePublisher {
    async fn publish_portal_theme<'a>(
        &'a self,
        report: &'a PortalThemeOutcomeReport,
    ) -> Result<(), GenesisError> {
        let payload = to_vec(report).map_err(|e| GenesisError::MessageBus {
            message: format!("failed to serialise outcome: {e}"),
        })?;

        info!(
            deployment_id = %report.deployment_id,
            revision = report.revision,
            realm = %report.realm,
            "reporting portal theme outcome"
        );

        self.send(payload).await
    }
}
