use std::future::Future;

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    CoreError, deployments::DeploymentId, organisation::OrganisationId,
    webhooks::value_objects::WebhookId,
};

pub const MAX_DELIVERIES: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryStatus {
    Pending,
    Delivering,
    Succeeded,
    Failed,
}

impl DeliveryStatus {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "delivering" => Some(Self::Delivering),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivering => "delivering",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct WebhookDelivery {
    pub delivery_id: String,
    pub webhook_id: WebhookId,
    pub event: String,
    pub status: DeliveryStatus,
    pub attempt_count: u32,
    pub last_status_code: Option<u16>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryQuery {
    pub deployment_id: DeploymentId,
    pub webhook_id: WebhookId,
    pub status: Option<DeliveryStatus>,
    pub from: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: usize,
}

pub trait WebhookDeliveryIndex: Send + Sync {
    fn deliveries(
        &self,
        organisation_id: OrganisationId,
        query: DeliveryQuery,
    ) -> impl Future<Output = Result<Vec<WebhookDelivery>, CoreError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_round_trips_through_its_name() {
        for status in [
            DeliveryStatus::Pending,
            DeliveryStatus::Delivering,
            DeliveryStatus::Succeeded,
            DeliveryStatus::Failed,
        ] {
            assert_eq!(DeliveryStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(DeliveryStatus::parse("nope"), None);
    }
}
