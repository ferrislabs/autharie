use std::collections::HashMap;

use autharie_core::{
    CoreError,
    organisation::OrganisationId,
    webhooks::{
        DeliveryStatus,
        delivery::{DeliveryQuery, WebhookDelivery, WebhookDeliveryIndex},
        value_objects::WebhookId,
    },
};
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use super::logs::QuickwitLogSearchIndex;

const OVERFETCH: usize = 3;

#[derive(Debug, Deserialize)]
struct DeliveryHit {
    delivery_id: String,
    webhook_id: Uuid,
    event: String,
    status: String,
    #[serde(default)]
    attempt_count: u32,
    #[serde(default)]
    last_status_code: Option<u16>,
    #[serde(default)]
    last_error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct DeliveryResponse {
    #[serde(default)]
    hits: Vec<serde_json::Value>,
}

fn as_phrase(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "'"))
}

fn build_query(query: &DeliveryQuery) -> String {
    let clauses = [
        "kind:webhook_delivery".to_string(),
        format!(
            "deployment_id:{}",
            as_phrase(&query.deployment_id.0.to_string())
        ),
        format!("webhook_id:{}", as_phrase(&query.webhook_id.0.to_string())),
    ];

    clauses.join(" AND ")
}

fn newest_per_delivery(
    hits: Vec<DeliveryHit>,
    status: Option<DeliveryStatus>,
    limit: usize,
) -> Vec<WebhookDelivery> {
    let mut latest: HashMap<String, DeliveryHit> = HashMap::new();

    for hit in hits {
        match latest.get(&hit.delivery_id) {
            Some(seen) if seen.updated_at >= hit.updated_at => {}
            _ => {
                latest.insert(hit.delivery_id.clone(), hit);
            }
        }
    }

    let mut deliveries: Vec<WebhookDelivery> = latest
        .into_values()
        .filter(|hit| {
            status.is_none_or(|wanted| DeliveryStatus::parse(&hit.status) == Some(wanted))
        })
        .filter_map(|hit| {
            Some(WebhookDelivery {
                status: DeliveryStatus::parse(&hit.status)?,
                delivery_id: hit.delivery_id,
                webhook_id: WebhookId(hit.webhook_id),
                event: hit.event,
                attempt_count: hit.attempt_count,
                last_status_code: hit.last_status_code,
                last_error: hit.last_error,
                created_at: hit.created_at,
                updated_at: hit.updated_at,
            })
        })
        .collect();

    deliveries.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.delivery_id.cmp(&b.delivery_id))
    });
    deliveries.truncate(limit);
    deliveries
}

impl WebhookDeliveryIndex for QuickwitLogSearchIndex {
    async fn deliveries(
        &self,
        organisation_id: OrganisationId,
        query: DeliveryQuery,
    ) -> Result<Vec<WebhookDelivery>, CoreError> {
        let mut body = serde_json::json!({
            "query": build_query(&query),
            "max_hits": query.limit * OVERFETCH,
            "sort_by_field": "-timestamp",
        });
        if let Some(from) = query.from {
            body["start_timestamp"] = from.timestamp().into();
        }
        if let Some(until) = query.until {
            body["end_timestamp"] = until.timestamp().into();
        }

        let response = self
            .client
            .post(self.search_url(organisation_id))
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                CoreError::InternalError(format!("failed to search the delivery index: {err}"))
            })?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(CoreError::InternalError(format!(
                "quickwit search failed with status {status}: {body}"
            )));
        }

        let parsed: DeliveryResponse = response.json().await.map_err(|err| {
            CoreError::InternalError(format!("failed to parse quickwit's response: {err}"))
        })?;

        let hits = parsed
            .hits
            .into_iter()
            .filter_map(|hit| serde_json::from_value::<DeliveryHit>(hit).ok())
            .collect();

        Ok(newest_per_delivery(hits, query.status, query.limit))
    }
}

impl WebhookDeliveryIndex for &QuickwitLogSearchIndex {
    async fn deliveries(
        &self,
        organisation_id: OrganisationId,
        query: DeliveryQuery,
    ) -> Result<Vec<WebhookDelivery>, CoreError> {
        (*self).deliveries(organisation_id, query).await
    }
}

#[cfg(test)]
mod tests {
    use autharie_core::deployments::DeploymentId;
    use chrono::Duration;

    use super::*;

    fn hit(id: &str, status: &str, created: i64, updated: i64) -> DeliveryHit {
        let base = DateTime::<Utc>::UNIX_EPOCH;
        DeliveryHit {
            delivery_id: id.to_string(),
            webhook_id: Uuid::nil(),
            event: "user.created".to_string(),
            status: status.to_string(),
            attempt_count: 1,
            last_status_code: None,
            last_error: None,
            created_at: base + Duration::seconds(created),
            updated_at: base + Duration::seconds(updated),
        }
    }

    #[test]
    fn the_query_is_scoped_to_one_webhook_of_one_deployment() {
        let query = build_query(&DeliveryQuery {
            deployment_id: DeploymentId(Uuid::from_u128(1)),
            webhook_id: WebhookId(Uuid::from_u128(2)),
            status: Some(DeliveryStatus::Failed),
            from: None,
            until: None,
            limit: 100,
        });

        assert!(query.starts_with("kind:webhook_delivery AND "));
        assert!(query.contains(&format!("deployment_id:\"{}\"", Uuid::from_u128(1))));
        assert!(query.contains(&format!("webhook_id:\"{}\"", Uuid::from_u128(2))));
        assert!(!query.contains("status:"));
    }

    #[test]
    fn a_delivery_indexed_twice_shows_its_latest_state_once() {
        let deliveries = newest_per_delivery(
            vec![
                hit("a", "failed", 10, 11),
                hit("a", "succeeded", 10, 20),
                hit("b", "pending", 30, 30),
            ],
            None,
            100,
        );

        assert_eq!(deliveries.len(), 2);
        assert_eq!(deliveries[0].delivery_id, "b");
        assert_eq!(deliveries[1].delivery_id, "a");
        assert_eq!(deliveries[1].status, DeliveryStatus::Succeeded);
    }

    #[test]
    fn the_status_filter_applies_to_the_latest_state_of_each_delivery() {
        let hits = || {
            vec![
                hit("a", "failed", 10, 11),
                hit("a", "succeeded", 10, 20),
                hit("b", "failed", 30, 30),
            ]
        };

        let failed = newest_per_delivery(hits(), Some(DeliveryStatus::Failed), 100);

        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].delivery_id, "b");
    }

    #[test]
    fn a_hit_with_an_unknown_status_is_left_out_and_the_limit_holds() {
        let deliveries = newest_per_delivery(
            vec![
                hit("a", "exploded", 1, 1),
                hit("b", "failed", 2, 2),
                hit("c", "failed", 3, 3),
            ],
            None,
            1,
        );

        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].delivery_id, "c");
    }
}
