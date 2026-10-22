use chrono::{DateTime, Utc};
use serde::Serialize;

/// The newest deliveries kept for the whole instance.
pub const MAX_DELIVERIES: usize = 200;

/// A FerrisKey delivery summary. FerrisKey's list leaves out the payload, and
/// this type has no place for headers or a signature either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDelivery {
    pub id: String,
    pub event: String,
    pub status: String,
    pub attempt_count: u32,
    pub last_status_code: Option<u16>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeliveryRecord {
    /// Autharie's id for the webhook, not FerrisKey's.
    pub webhook_id: String,
    pub delivery_id: String,
    pub event: String,
    pub status: String,
    pub attempt_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Newest first across webhooks, capped. Ties break on the ids so the same
/// input always renders the same bytes, which is what lets an unchanged list
/// go unwritten.
pub fn collect(per_webhook: Vec<(&str, Vec<RemoteDelivery>)>) -> Vec<DeliveryRecord> {
    let mut records: Vec<DeliveryRecord> = per_webhook
        .into_iter()
        .flat_map(|(webhook_id, deliveries)| {
            deliveries.into_iter().map(move |delivery| DeliveryRecord {
                webhook_id: webhook_id.to_string(),
                delivery_id: delivery.id,
                event: delivery.event,
                status: delivery.status,
                attempt_count: delivery.attempt_count,
                last_status_code: delivery.last_status_code,
                last_error: delivery.last_error,
                created_at: delivery.created_at,
                updated_at: delivery.updated_at,
            })
        })
        .collect();
    records.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.webhook_id.cmp(&b.webhook_id))
            .then_with(|| a.delivery_id.cmp(&b.delivery_id))
    });
    records.truncate(MAX_DELIVERIES);
    records
}

pub fn render(records: &[DeliveryRecord]) -> String {
    serde_json::to_string(records).unwrap_or_else(|_| "[]".to_string())
}

/// Whether the ConfigMap needs writing: nothing to say and nothing said yet is
/// not a change.
pub fn needs_write(current: Option<&str>, rendered: &str) -> bool {
    match current {
        Some(current) => current != rendered,
        None => rendered != "[]",
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::Value;

    use super::*;

    fn at(second: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_790_000_000 + second, 0).unwrap()
    }

    fn delivery(id: &str, created: i64) -> RemoteDelivery {
        RemoteDelivery {
            id: id.to_string(),
            event: "user.created".to_string(),
            status: "failed".to_string(),
            attempt_count: 3,
            last_status_code: Some(500),
            last_error: Some("http_500".to_string()),
            created_at: at(created),
            updated_at: at(created + 1),
        }
    }

    #[test]
    fn a_delivery_becomes_a_record_under_the_autharie_webhook_id() {
        let records = collect(vec![("autharie-id", vec![delivery("d1", 0)])]);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].webhook_id, "autharie-id");
        assert_eq!(records[0].delivery_id, "d1");
        assert_eq!(records[0].attempt_count, 3);
        assert_eq!(records[0].last_status_code, Some(500));
    }

    #[test]
    fn records_are_newest_first_across_webhooks() {
        let records = collect(vec![
            ("a", vec![delivery("a-old", 1), delivery("a-new", 30)]),
            ("b", vec![delivery("b-mid", 20)]),
        ]);

        let ids: Vec<_> = records.iter().map(|r| r.delivery_id.as_str()).collect();
        assert_eq!(ids, vec!["a-new", "b-mid", "a-old"]);
    }

    #[test]
    fn equal_times_keep_a_stable_order() {
        let one = collect(vec![
            ("b", vec![delivery("x", 5)]),
            ("a", vec![delivery("y", 5)]),
        ]);
        let two = collect(vec![
            ("a", vec![delivery("y", 5)]),
            ("b", vec![delivery("x", 5)]),
        ]);

        assert_eq!(render(&one), render(&two));
    }

    #[test]
    fn only_the_newest_two_hundred_are_kept() {
        let many: Vec<_> = (0..250).map(|n| delivery(&format!("d{n:03}"), n)).collect();

        let records = collect(vec![("a", many)]);

        assert_eq!(records.len(), MAX_DELIVERIES);
        assert_eq!(records[0].delivery_id, "d249");
        assert_eq!(records[199].delivery_id, "d050");
    }

    #[test]
    fn the_rendering_has_the_agreed_keys_and_times_and_nothing_else() {
        let mut bare = delivery("d1", 0);
        bare.last_status_code = None;
        bare.last_error = None;

        let value: Value =
            serde_json::from_str(&render(&collect(vec![("a", vec![bare])]))).unwrap();

        let mut keys: Vec<_> = value[0].as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "attempt_count",
                "created_at",
                "delivery_id",
                "event",
                "status",
                "updated_at",
                "webhook_id"
            ]
        );
        assert!(DateTime::parse_from_rfc3339(value[0]["created_at"].as_str().unwrap()).is_ok());
    }

    #[test]
    fn nothing_is_written_when_nothing_changed() {
        let rendered = render(&collect(vec![("a", vec![delivery("d1", 0)])]));

        assert!(!needs_write(Some(&rendered), &rendered));
        assert!(needs_write(Some("[]"), &rendered));
        assert!(needs_write(None, &rendered));
        assert!(!needs_write(None, "[]"));
        assert!(needs_write(Some(&rendered), "[]"));
    }
}
