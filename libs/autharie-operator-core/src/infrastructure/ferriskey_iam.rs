use std::sync::Arc;

use reqwest::Method;
use serde_json::Value;
use uuid::Uuid;

use autharie_crds::v1alpha::identity_instance::IdentityInstance;

use crate::domain::agents::AgentsError;
use crate::domain::iam::ports::FerriskeyIamPort;
use crate::domain::iam::result::{
    ApplicationDetail, ApplicationPage, ApplicationSummary, IamResult,
};
use crate::domain::iam::{IamError, IamOperation, PageNumber, PageSize, SearchTerm};

use super::ferriskey_agents::{Caller, KubeFerriskeyAgents, segment, send, text};
use super::ferriskey_clients::next_page;
use super::ferriskey_webhooks::ApiRequest;

fn into_error(error: AgentsError) -> IamError {
    match error {
        AgentsError::Retry { message } => IamError::Retry { message },
        AgentsError::Rejected { message } => IamError::Rejected { message },
        AgentsError::NotFound => IamError::rejected("the realm or the application was not found"),
    }
}

fn malformed() -> IamError {
    IamError::rejected("FerrisKey answered an application in a shape the operator does not know")
}

pub fn list_request(
    realm: &str,
    page: PageNumber,
    limit: PageSize,
    search: Option<&SearchTerm>,
) -> Result<ApiRequest, IamError> {
    let request = ApiRequest::new(
        Method::GET,
        format!("/realms/{}/clients", segment(realm).map_err(into_error)?),
    )
    .query("page", page.get())
    .query("limit", limit.get())
    .query("order_by", "created_at")
    .query("order", "desc");

    Ok(match search {
        Some(term) => request.query("search", term.as_str()),
        None => request,
    })
}

pub fn get_request(realm: &str, id: Uuid) -> Result<ApiRequest, IamError> {
    Ok(ApiRequest::new(
        Method::GET,
        format!(
            "/realms/{}/clients/{}",
            segment(realm).map_err(into_error)?,
            id.hyphenated()
        ),
    ))
}

fn flag(value: &Value, key: &str) -> bool {
    value[key].as_bool().unwrap_or(false)
}

fn owned(value: &Value, key: &str) -> String {
    text(value, key).unwrap_or_default().to_string()
}

pub fn summary_of(value: &Value) -> Option<ApplicationSummary> {
    Some(ApplicationSummary {
        id: text(value, "id")?.to_string(),
        client_id: text(value, "client_id")?.to_string(),
        name: owned(value, "name"),
        enabled: flag(value, "enabled"),
        client_type: owned(value, "client_type"),
        public_client: flag(value, "public_client"),
        service_account_enabled: flag(value, "service_account_enabled"),
        protocol: owned(value, "protocol"),
        registration_source: owned(value, "registration_source"),
        created_at: owned(value, "created_at"),
    })
}

fn redirect_uris(value: &Value) -> Vec<String> {
    value["redirect_uris"]
        .as_array()
        .map(|uris| {
            uris.iter()
                .filter_map(|uri| uri.as_str().or_else(|| uri["value"].as_str()))
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn detail_of(value: &Value) -> Option<ApplicationDetail> {
    Some(ApplicationDetail {
        summary: summary_of(value)?,
        direct_access_grants_enabled: flag(value, "direct_access_grants_enabled"),
        require_pkce: flag(value, "require_pkce"),
        token_exchange_enabled: flag(value, "token_exchange_enabled"),
        consent_required: flag(value, "consent_required"),
        redirect_uris: redirect_uris(value),
        access_token_lifetime: value["access_token_lifetime"].as_i64(),
        refresh_token_lifetime: value["refresh_token_lifetime"].as_i64(),
        id_token_lifetime: value["id_token_lifetime"].as_i64(),
        updated_at: owned(value, "updated_at"),
    })
}

pub fn page_of(body: &Value) -> Result<ApplicationPage, IamError> {
    let items = body["data"].as_array().ok_or_else(malformed)?;

    Ok(ApplicationPage {
        items: items
            .iter()
            .map(|item| summary_of(item).ok_or_else(malformed))
            .collect::<Result<_, _>>()?,
        next_page: next_page(body),
    })
}

async fn run<C: Caller>(
    caller: &C,
    realm: &str,
    operation: &IamOperation,
) -> Result<IamResult, IamError> {
    match operation {
        IamOperation::ApplicationsList {
            page,
            limit,
            search,
        } => {
            let request = list_request(realm, *page, *limit, search.as_ref())?;
            let body = send(caller, request).await.map_err(into_error)?;
            page_of(&body).map(IamResult::Applications)
        }
        IamOperation::ApplicationsGet { id } => {
            let body = send(caller, get_request(realm, *id)?)
                .await
                .map_err(into_error)?;
            let client = if body["data"].is_object() {
                &body["data"]
            } else {
                &body
            };
            detail_of(client)
                .map(|detail| IamResult::Application(Box::new(detail)))
                .ok_or_else(malformed)
        }
    }
}

pub struct KubeFerriskeyIam {
    agents: Arc<KubeFerriskeyAgents>,
}

impl KubeFerriskeyIam {
    pub fn new(agents: Arc<KubeFerriskeyAgents>) -> Self {
        Self { agents }
    }
}

impl FerriskeyIamPort for KubeFerriskeyIam {
    async fn execute(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        operation: &IamOperation,
    ) -> Result<IamResult, IamError> {
        run(&self.agents.bound(instance), realm, operation).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::super::ferriskey_webhooks::{ApiFailure, FerriskeyWebhookApi};
    use super::*;
    use crate::infrastructure::test_support::serve;

    const APP: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a22";
    const SECRET: &str = "s3cr3t-do-not-leak-0123456789";

    type Logged = (Method, String, Vec<(&'static str, String)>);
    type Answer = Box<dyn Fn(&ApiRequest) -> Result<Value, ApiFailure> + Send + Sync>;

    struct Recorded {
        log: Mutex<Vec<Logged>>,
        answer: Answer,
    }

    impl Recorded {
        fn answering(
            answer: impl Fn(&ApiRequest) -> Result<Value, ApiFailure> + Send + Sync + 'static,
        ) -> Self {
            Self {
                log: Mutex::new(Vec::new()),
                answer: Box::new(answer),
            }
        }

        fn calls(&self) -> Vec<Logged> {
            self.log.lock().expect("not poisoned").clone()
        }
    }

    impl Caller for Recorded {
        async fn call(&self, request: &ApiRequest) -> Result<Value, ApiFailure> {
            self.log.lock().expect("not poisoned").push((
                request.method.clone(),
                request.path.clone(),
                request.query.clone(),
            ));
            (self.answer)(request)
        }
    }

    fn client(id: &str) -> Value {
        json!({
            "id": id,
            "enabled": true,
            "client_id": format!("app-{id}"),
            "secret": SECRET,
            "realm_id": "11111111-1111-4111-8111-111111111111",
            "protocol": "openid-connect",
            "public_client": false,
            "service_account_enabled": true,
            "direct_access_grants_enabled": false,
            "oauth_device_code_grant_enabled": false,
            "require_pkce": true,
            "token_exchange_enabled": false,
            "backchannel_logout_uri": null,
            "client_type": "confidential",
            "name": "Billing",
            "redirect_uris": [
                { "id": "u1", "client_id": id, "value": "https://a.example.com/cb", "enabled": true },
                { "id": "u2", "client_id": id, "value": "https://b.example.com/cb", "enabled": false }
            ],
            "access_token_lifetime": 300,
            "refresh_token_lifetime": null,
            "id_token_lifetime": 60,
            "consent_required": false,
            "registration_source": "admin",
            "created_at": "2026-10-23T10:00:00Z",
            "updated_at": "2026-10-24T10:00:00Z"
        })
    }

    fn page_operation(page: u32, limit: u32, search: Option<&str>) -> IamOperation {
        let mut operation = json!({ "type": "applications.list", "page": page, "limit": limit });
        if let Some(term) = search {
            operation["search"] = json!(term);
        }
        IamOperation::try_from(operation.to_string().as_str()).unwrap()
    }

    fn get_operation() -> IamOperation {
        IamOperation::try_from(
            json!({ "type": "applications.get", "id": APP })
                .to_string()
                .as_str(),
        )
        .unwrap()
    }

    #[test]
    fn a_list_asks_for_the_page_newest_first_and_never_more_than_fifty() {
        let operation = page_operation(3, 5000, None);
        let IamOperation::ApplicationsList {
            page,
            limit,
            search,
        } = operation
        else {
            panic!("a list");
        };

        let request = list_request("acme", page, limit, search.as_ref()).unwrap();

        assert_eq!(request.method, Method::GET);
        assert_eq!(request.path, "/realms/acme/clients");
        assert_eq!(
            request.query,
            vec![
                ("page", "3".to_string()),
                ("limit", "50".to_string()),
                ("order_by", "created_at".to_string()),
                ("order", "desc".to_string()),
            ]
        );
        assert_eq!(request.body, None);
    }

    #[test]
    fn a_search_term_goes_out_as_the_search_parameter() {
        let IamOperation::ApplicationsList {
            page,
            limit,
            search,
        } = page_operation(1, 25, Some("billing"))
        else {
            panic!("a list");
        };

        let request = list_request("acme", page, limit, search.as_ref()).unwrap();

        assert!(request.query.contains(&("search", "billing".to_string())));
    }

    #[tokio::test]
    async fn a_search_term_is_url_encoded_on_the_wire() {
        let (url, server) = serve(vec![(200, r#"{"data":[],"metadata":{}}"#)]);
        let IamOperation::ApplicationsList {
            page,
            limit,
            search,
        } = page_operation(1, 25, Some("a&b c=d"))
        else {
            panic!("a list");
        };
        let request = list_request("acme", page, limit, search.as_ref()).unwrap();

        FerriskeyWebhookApi::new()
            .execute(&url, "t", &request)
            .await
            .unwrap();

        let seen = server.join().unwrap();
        let line = seen[0].head.lines().next().unwrap();
        assert!(line.starts_with("GET /realms/acme/clients?"), "{line}");
        assert!(
            line.contains("search=a%26b+c%3Dd") || line.contains("search=a%26b%20c%3Dd"),
            "{line}"
        );
    }

    #[test]
    fn a_get_names_the_clients_own_uuid_in_the_path() {
        let request = get_request("acme", Uuid::parse_str(APP).unwrap()).unwrap();

        assert_eq!(request.path, format!("/realms/acme/clients/{APP}"));
        assert!(request.query.is_empty());
    }

    #[test]
    fn a_realm_that_could_change_the_path_never_reaches_it() {
        for realm in ["", "..", "a/b", "a?b", "a b"] {
            assert!(matches!(
                get_request(realm, Uuid::nil()),
                Err(IamError::Rejected { .. })
            ));
            assert!(
                list_request(realm, PageNumber::clamped(1), PageSize::clamped(1), None).is_err()
            );
        }
    }

    #[test]
    fn spec_iam_3_a_listed_client_loses_its_secret() {
        let summary = summary_of(&client(APP)).expect("mapped");

        let shown = serde_json::to_string(&summary).unwrap();

        assert!(!shown.contains(SECRET));
        assert!(!shown.contains("secret"));
        assert_eq!(summary.client_id, format!("app-{APP}"));
        assert_eq!(summary.client_type, "confidential");
        assert!(summary.service_account_enabled);
    }

    #[test]
    fn spec_iam_3_a_read_client_loses_its_secret_and_flattens_its_redirect_uris() {
        let detail = detail_of(&client(APP)).expect("mapped");

        let shown = serde_json::to_string(&detail).unwrap();

        assert!(!shown.contains(SECRET));
        assert!(!shown.contains("secret"));
        assert_eq!(
            detail.redirect_uris,
            ["https://a.example.com/cb", "https://b.example.com/cb"]
        );
        assert_eq!(detail.access_token_lifetime, Some(300));
        assert_eq!(detail.refresh_token_lifetime, None);
        assert_eq!(detail.id_token_lifetime, Some(60));
        assert!(detail.require_pkce);
        assert_eq!(detail.updated_at, "2026-10-24T10:00:00Z");
    }

    #[test]
    fn a_client_without_an_id_is_not_an_application() {
        assert!(summary_of(&json!({ "client_id": "x" })).is_none());
        assert!(summary_of(&json!({ "id": "x" })).is_none());
    }

    #[tokio::test]
    async fn a_page_is_mapped_and_names_the_next_one() {
        let caller = Recorded::answering(|_| {
            Ok(json!({
                "data": [client("a"), client("b")],
                "metadata": { "page": 1, "next_page": 2 }
            }))
        });

        let result = run(&caller, "acme", &page_operation(1, 2, None))
            .await
            .expect("listed");

        let IamResult::Applications(page) = result else {
            panic!("a page");
        };
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.next_page, Some(2));
        assert_eq!(caller.calls().len(), 1);
        assert!(!serde_json::to_string(&page).unwrap().contains(SECRET));
    }

    #[tokio::test]
    async fn the_last_page_has_no_next_page() {
        let caller = Recorded::answering(|_| {
            Ok(json!({ "data": [client("a")], "metadata": { "page": 1, "next_page": null } }))
        });

        let IamResult::Applications(page) = run(&caller, "acme", &page_operation(1, 25, None))
            .await
            .unwrap()
        else {
            panic!("a page");
        };

        assert_eq!(page.next_page, None);
    }

    #[tokio::test]
    async fn one_application_is_read_from_the_data_envelope() {
        let caller = Recorded::answering(|_| Ok(json!({ "data": client(APP) })));

        let IamResult::Application(detail) = run(&caller, "acme", &get_operation()).await.unwrap()
        else {
            panic!("a detail");
        };

        assert_eq!(detail.summary.id, APP);
        assert_eq!(caller.calls()[0].1, format!("/realms/acme/clients/{APP}"));
    }

    #[tokio::test]
    async fn an_answer_in_an_unknown_shape_is_refused_not_retried() {
        let caller = Recorded::answering(|_| Ok(json!({ "unexpected": true })));

        assert!(matches!(
            run(&caller, "acme", &page_operation(1, 25, None)).await,
            Err(IamError::Rejected { .. })
        ));
        assert!(matches!(
            run(&caller, "acme", &get_operation()).await,
            Err(IamError::Rejected { .. })
        ));
    }

    #[tokio::test]
    async fn a_missing_application_is_a_refusal_and_a_down_ferriskey_a_retry() {
        let missing = Recorded::answering(|_| {
            Err(ApiFailure::Rejected {
                status: 404,
                message: "FerrisKey answered 404".to_string(),
            })
        });
        let down =
            Recorded::answering(|_| Err(ApiFailure::Retry("FerrisKey is unreachable".to_string())));

        let refused = run(&missing, "acme", &get_operation()).await.unwrap_err();
        let retried = run(&down, "acme", &get_operation()).await.unwrap_err();

        assert_eq!(
            refused,
            IamError::rejected("the realm or the application was not found")
        );
        assert!(retried.is_retry());
    }
}
