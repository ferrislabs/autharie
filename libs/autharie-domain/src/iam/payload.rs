use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use super::operation::IamOperation;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
enum OperationPayload<'a> {
    #[serde(rename = "applications.list")]
    ApplicationsList {
        page: u32,
        limit: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        search: Option<&'a str>,
    },
    #[serde(rename = "applications.get")]
    ApplicationsGet { id: Uuid },
}

impl<'a> From<&'a IamOperation> for OperationPayload<'a> {
    fn from(operation: &'a IamOperation) -> Self {
        match operation {
            IamOperation::ApplicationsList {
                page,
                limit,
                search,
            } => Self::ApplicationsList {
                page: page.value(),
                limit: limit.value(),
                search: search.as_ref().map(|term| term.as_str()),
            },
            IamOperation::ApplicationsGet { id } => Self::ApplicationsGet { id: *id },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IamActionPayload<'a> {
    deployment_id: Uuid,
    namespace: &'a str,
    request_id: Uuid,
    realm: &'a str,
    operation: OperationPayload<'a>,
    sealing_key: Value,
    expires_at: DateTime<Utc>,
}

impl<'a> IamActionPayload<'a> {
    pub fn new(
        deployment_id: Uuid,
        namespace: &'a str,
        request_id: Uuid,
        realm: &'a str,
        operation: &'a IamOperation,
        sealing_key: Value,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            deployment_id,
            namespace,
            request_id,
            realm,
            operation: OperationPayload::from(operation),
            sealing_key,
            expires_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 24, 10, 5, 0).unwrap()
    }

    fn payload(operation: &IamOperation) -> Value {
        serde_json::to_value(IamActionPayload::new(
            Uuid::nil(),
            "production-auth",
            Uuid::from_u128(9),
            "master",
            operation,
            json!({ "kty": "RSA", "n": "AQ", "e": "AQAB", "alg": "RSA-OAEP-256" }),
            at(),
        ))
        .unwrap()
    }

    #[test]
    fn a_list_carries_every_field_of_the_contract_with_page_and_limit_always_present() {
        let operation = IamOperation::applications_list(None, None, None).unwrap();

        assert_eq!(
            payload(&operation),
            json!({
                "deployment_id": Uuid::nil(),
                "namespace": "production-auth",
                "request_id": Uuid::from_u128(9),
                "realm": "master",
                "operation": { "type": "applications.list", "page": 1, "limit": 25 },
                "sealing_key": { "kty": "RSA", "n": "AQ", "e": "AQAB", "alg": "RSA-OAEP-256" },
                "expires_at": "2026-10-24T10:05:00Z"
            })
        );
    }

    #[test]
    fn a_search_term_is_carried_only_when_there_is_one() {
        let operation = IamOperation::applications_list(Some(2), Some(10), Some("web")).unwrap();

        assert_eq!(
            payload(&operation)["operation"],
            json!({ "type": "applications.list", "page": 2, "limit": 10, "search": "web" })
        );
    }

    #[test]
    fn a_get_carries_the_id_alone() {
        let id = Uuid::from_u128(5);

        assert_eq!(
            payload(&IamOperation::ApplicationsGet { id })["operation"],
            json!({ "type": "applications.get", "id": id })
        );
    }
}
