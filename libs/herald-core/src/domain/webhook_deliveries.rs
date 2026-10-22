use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::entities::deployment::DeploymentId;
use super::entities::logs::OrganisationId;
use super::log_index::{LogIndexDocument, fingerprint};

pub const WEBHOOK_DELIVERY_KIND: &str = "webhook_delivery";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WebhookDelivery {
    pub webhook_id: String,
    pub delivery_id: String,
    pub event: String,
    pub status: String,
    pub attempt_count: u64,
    #[serde(default)]
    pub last_status_code: Option<u16>,
    #[serde(default)]
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn parse_deliveries(raw: &str) -> Result<Vec<WebhookDelivery>, serde_json::Error> {
    serde_json::from_str(raw)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebhookDeliveryFields {
    pub kind: &'static str,
    pub webhook_id: String,
    pub delivery_id: String,
    pub event: String,
    pub status: String,
    pub attempt_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl LogIndexDocument {
    pub fn from_delivery(
        organisation_id: &OrganisationId,
        deployment_id: &DeploymentId,
        delivery: &WebhookDelivery,
    ) -> Self {
        let message = format!("webhook delivery of {} {}", delivery.event, delivery.status);

        Self {
            timestamp: delivery.updated_at,
            organisation_id: organisation_id.clone(),
            deployment_id: deployment_id.clone(),
            source: "webhook".to_string(),
            level: if delivery.status == "failed" {
                "warn"
            } else {
                "info"
            }
            .to_string(),
            fingerprint: fingerprint(&message),
            message,
            agent_decision: None,
            delivery: Some(WebhookDeliveryFields {
                kind: WEBHOOK_DELIVERY_KIND,
                webhook_id: delivery.webhook_id.clone(),
                delivery_id: delivery.delivery_id.clone(),
                event: delivery.event.clone(),
                status: delivery.status.clone(),
                attempt_count: delivery.attempt_count,
                last_status_code: delivery.last_status_code,
                last_error: delivery.last_error.clone(),
                created_at: delivery.created_at,
                updated_at: delivery.updated_at,
            }),
        }
    }
}

#[derive(Debug, Default)]
pub struct DeliveryLedger {
    shipped: HashMap<DeploymentId, HashMap<String, (String, DateTime<Utc>)>>,
}

impl DeliveryLedger {
    pub fn unshipped<'a>(
        &self,
        deployment_id: &DeploymentId,
        current: &'a [WebhookDelivery],
    ) -> Vec<&'a WebhookDelivery> {
        let known = self.shipped.get(deployment_id);

        current
            .iter()
            .filter(|delivery| {
                known
                    .and_then(|known| known.get(&delivery.delivery_id))
                    .is_none_or(|(status, updated_at)| {
                        *status != delivery.status || *updated_at != delivery.updated_at
                    })
            })
            .collect()
    }

    pub fn settle(&mut self, deployment_id: &DeploymentId, current: &[WebhookDelivery], ok: bool) {
        let previous = self.shipped.remove(deployment_id).unwrap_or_default();

        let next: HashMap<_, _> = current
            .iter()
            .filter_map(|delivery| {
                let entry = if ok {
                    Some((delivery.status.clone(), delivery.updated_at))
                } else {
                    previous.get(&delivery.delivery_id).cloned()
                };
                entry.map(|entry| (delivery.delivery_id.clone(), entry))
            })
            .collect();

        if !next.is_empty() {
            self.shipped.insert(deployment_id.clone(), next);
        }
    }

    pub fn retain(&mut self, served: &HashSet<DeploymentId>) {
        self.shipped.retain(|id, _| served.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(time: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(time)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn delivery(id: &str, status: &str, updated_at: &str) -> WebhookDelivery {
        WebhookDelivery {
            webhook_id: "w1".to_string(),
            delivery_id: id.to_string(),
            event: "user.created".to_string(),
            status: status.to_string(),
            attempt_count: 1,
            last_status_code: Some(500),
            last_error: Some("boom".to_string()),
            created_at: at("2026-10-22T08:00:00Z"),
            updated_at: at(updated_at),
        }
    }

    fn dep() -> DeploymentId {
        DeploymentId::new("dep-1")
    }

    #[test]
    fn a_configmap_document_is_parsed() {
        let raw = json!([
            {
                "webhook_id": "w1", "delivery_id": "d1", "event": "user.created",
                "status": "failed", "attempt_count": 3, "last_status_code": 502,
                "last_error": "bad gateway",
                "created_at": "2026-10-22T08:00:00Z", "updated_at": "2026-10-22T08:05:00Z"
            },
            {
                "webhook_id": "w1", "delivery_id": "d2", "event": "user.deleted",
                "status": "pending", "attempt_count": 0,
                "created_at": "2026-10-22T08:06:00Z", "updated_at": "2026-10-22T08:06:00Z"
            }
        ])
        .to_string();

        let parsed = parse_deliveries(&raw).expect("parses");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].last_status_code, Some(502));
        assert_eq!(parsed[1].last_status_code, None);
        assert_eq!(parsed[1].last_error, None);
    }

    #[test]
    fn an_empty_list_is_valid() {
        assert!(parse_deliveries("[]").expect("parses").is_empty());
    }

    #[test]
    fn a_malformed_document_is_an_error() {
        assert!(parse_deliveries("{not json").is_err());
        assert!(parse_deliveries(r#"{"a": 1}"#).is_err());
        assert!(parse_deliveries(r#"[{"delivery_id": "d1"}]"#).is_err());
    }

    #[test]
    fn everything_is_new_the_first_time() {
        let ledger = DeliveryLedger::default();
        let current = vec![
            delivery("d1", "pending", "2026-10-22T08:00:00Z"),
            delivery("d2", "failed", "2026-10-22T08:01:00Z"),
        ];

        assert_eq!(ledger.unshipped(&dep(), &current).len(), 2);
    }

    #[test]
    fn an_unchanged_delivery_is_not_shipped_again() {
        let mut ledger = DeliveryLedger::default();
        let current = vec![delivery("d1", "failed", "2026-10-22T08:00:00Z")];
        ledger.settle(&dep(), &current, true);

        assert!(ledger.unshipped(&dep(), &current).is_empty());
    }

    #[test]
    fn a_changed_status_or_updated_at_is_shipped_again() {
        let mut ledger = DeliveryLedger::default();
        ledger.settle(
            &dep(),
            &[
                delivery("d1", "delivering", "2026-10-22T08:00:00Z"),
                delivery("d2", "failed", "2026-10-22T08:00:00Z"),
                delivery("d3", "succeeded", "2026-10-22T08:00:00Z"),
            ],
            true,
        );

        let current = vec![
            delivery("d1", "succeeded", "2026-10-22T08:00:00Z"),
            delivery("d2", "failed", "2026-10-22T08:09:00Z"),
            delivery("d3", "succeeded", "2026-10-22T08:00:00Z"),
            delivery("d4", "pending", "2026-10-22T08:10:00Z"),
        ];

        let ids: Vec<_> = ledger
            .unshipped(&dep(), &current)
            .iter()
            .map(|d| d.delivery_id.as_str())
            .collect();
        assert_eq!(ids, ["d1", "d2", "d4"]);
    }

    #[test]
    fn a_failed_shipment_is_tried_again() {
        let mut ledger = DeliveryLedger::default();
        let current = vec![delivery("d1", "failed", "2026-10-22T08:00:00Z")];
        ledger.settle(&dep(), &current, false);

        assert_eq!(ledger.unshipped(&dep(), &current).len(), 1);
    }

    #[test]
    fn a_failed_shipment_keeps_what_was_shipped_before() {
        let mut ledger = DeliveryLedger::default();
        let first = vec![delivery("d1", "delivering", "2026-10-22T08:00:00Z")];
        ledger.settle(&dep(), &first, true);

        let second = vec![delivery("d1", "failed", "2026-10-22T08:05:00Z")];
        ledger.settle(&dep(), &second, false);

        assert_eq!(ledger.unshipped(&dep(), &second).len(), 1);
        assert!(ledger.unshipped(&dep(), &first).is_empty());
    }

    #[test]
    fn a_restart_ships_everything_again() {
        let mut ledger = DeliveryLedger::default();
        let current = vec![delivery("d1", "failed", "2026-10-22T08:00:00Z")];
        ledger.settle(&dep(), &current, true);

        let restarted = DeliveryLedger::default();

        assert_eq!(restarted.unshipped(&dep(), &current).len(), 1);
    }

    #[test]
    fn deployments_are_tracked_apart() {
        let mut ledger = DeliveryLedger::default();
        let current = vec![delivery("d1", "failed", "2026-10-22T08:00:00Z")];
        ledger.settle(&dep(), &current, true);

        assert_eq!(
            ledger
                .unshipped(&DeploymentId::new("dep-2"), &current)
                .len(),
            1
        );
    }

    #[test]
    fn a_deployment_no_longer_served_is_forgotten() {
        let mut ledger = DeliveryLedger::default();
        let current = vec![delivery("d1", "failed", "2026-10-22T08:00:00Z")];
        ledger.settle(&dep(), &current, true);

        ledger.retain(&HashSet::new());

        assert_eq!(ledger.unshipped(&dep(), &current).len(), 1);
    }

    #[test]
    fn a_delivery_that_left_the_list_is_forgotten() {
        let mut ledger = DeliveryLedger::default();
        ledger.settle(
            &dep(),
            &[delivery("d1", "failed", "2026-10-22T08:00:00Z")],
            true,
        );
        ledger.settle(&dep(), &[], true);

        assert_eq!(
            ledger
                .unshipped(&dep(), &[delivery("d1", "failed", "2026-10-22T08:00:00Z")])
                .len(),
            1
        );
    }

    #[test]
    fn a_delivery_ships_as_a_flat_document() {
        let document = LogIndexDocument::from_delivery(
            &OrganisationId::new("org-1"),
            &dep(),
            &delivery("d1", "failed", "2026-10-22T08:05:00Z"),
        );

        assert_eq!(
            serde_json::to_value(&document).unwrap(),
            json!({
                "timestamp": "2026-10-22T08:05:00Z",
                "organisation_id": "org-1",
                "deployment_id": "dep-1",
                "source": "webhook",
                "level": "warn",
                "message": "webhook delivery of user.created failed",
                "fingerprint": document.fingerprint,
                "kind": "webhook_delivery",
                "webhook_id": "w1",
                "delivery_id": "d1",
                "event": "user.created",
                "status": "failed",
                "attempt_count": 1,
                "last_status_code": 500,
                "last_error": "boom",
                "created_at": "2026-10-22T08:00:00Z",
                "updated_at": "2026-10-22T08:05:00Z"
            })
        );
    }

    #[test]
    fn a_log_line_document_has_no_delivery_fields() {
        let mut document = LogIndexDocument::from_delivery(
            &OrganisationId::new("org-1"),
            &dep(),
            &delivery("d1", "succeeded", "2026-10-22T08:05:00Z"),
        );
        document.delivery = None;

        let value = serde_json::to_value(&document).unwrap();
        assert!(value.get("kind").is_none());
        assert!(value.get("delivery_id").is_none());
    }
}
