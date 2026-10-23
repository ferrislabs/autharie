use reqwest::Method;
use serde_json::{Value, json};

use autharie_crds::v1alpha::identity_instance::IdentityInstance;

use crate::domain::agents::AgentsError;
use crate::domain::agents::ports::{ClientSecret, FerriskeyClientsPort};
use crate::domain::agents::registry::{AgentClient, AgentClientId, RemoteClient};

use super::ferriskey_agents::{
    Caller, KubeFerriskeyAgents, client_path, id_of, malformed, segment, send, text,
};
use super::ferriskey_webhooks::{ApiRequest, PAGE_LIMIT};

const MAX_PAGES: u32 = 100;

fn node(value: &Value) -> &Value {
    if value["data"].is_object() {
        &value["data"]
    } else {
        value
    }
}

pub fn list_clients_request(realm: &str, page: u32) -> Result<ApiRequest, AgentsError> {
    Ok(
        ApiRequest::new(Method::GET, format!("/realms/{}/clients", segment(realm)?))
            .query("page", page)
            .query("limit", PAGE_LIMIT)
            .query("order_by", "created_at")
            .query("order", "desc"),
    )
}

pub fn parse_remote(value: &Value) -> Option<RemoteClient> {
    Some(RemoteClient {
        id: text(value, "id")?.to_string(),
        client_id: text(value, "client_id")?.to_string(),
        name: text(value, "name").map(ToOwned::to_owned),
        registration_source: text(value, "registration_source")
            .unwrap_or("admin")
            .to_string(),
        enabled: value["enabled"].as_bool().unwrap_or(true),
        created_at: text(value, "created_at").map(ToOwned::to_owned),
    })
}

pub fn next_page(body: &Value) -> Option<u32> {
    body["metadata"]["next_page"]
        .as_u64()
        .and_then(|page| u32::try_from(page).ok())
}

async fn list_all<C: Caller>(caller: &C, realm: &str) -> Result<Vec<RemoteClient>, AgentsError> {
    let mut clients = Vec::new();
    let mut page = 1;

    for _ in 0..MAX_PAGES {
        let body = send(caller, list_clients_request(realm, page)?).await?;
        clients.extend(
            super::ferriskey_agents::items(&body)
                .into_iter()
                .filter_map(parse_remote),
        );
        match next_page(&body) {
            Some(next) if next > page => page = next,
            _ => break,
        }
    }

    Ok(clients)
}

pub fn set_enabled_request(
    realm: &str,
    client: &AgentClient,
    enabled: bool,
) -> Result<ApiRequest, AgentsError> {
    Ok(
        ApiRequest::new(Method::PATCH, client_path(realm, client.remote_id())?)
            .body(json!({ "enabled": enabled })),
    )
}

pub fn delete_request(realm: &str, client: &AgentClient) -> Result<ApiRequest, AgentsError> {
    Ok(ApiRequest::new(
        Method::DELETE,
        client_path(realm, client.remote_id())?,
    ))
}

pub fn create_request(
    realm: &str,
    id: &AgentClientId,
    name: &str,
) -> Result<ApiRequest, AgentsError> {
    Ok(
        ApiRequest::new(Method::POST, format!("/realms/{}/clients", segment(realm)?)).body(json!({
            "name": name,
            "client_id": id.as_str(),
            "client_type": "confidential",
            "service_account_enabled": true,
            "public_client": false,
            "enabled": true,
            "direct_access_grants_enabled": false,
            "token_exchange_enabled": false,
            "require_pkce": false,
        })),
    )
}

async fn create<C: Caller>(
    caller: &C,
    realm: &str,
    id: &AgentClientId,
    name: &str,
) -> Result<AgentClient, AgentsError> {
    let created = send(caller, create_request(realm, id, name)?).await?;
    let body = node(&created);

    AgentClient::try_from(RemoteClient {
        id: id_of(body, "a client")?,
        client_id: id.as_str().to_string(),
        name: Some(name.to_string()),
        registration_source: "admin".to_string(),
        enabled: body["enabled"].as_bool().unwrap_or(true),
        created_at: text(body, "created_at").map(ToOwned::to_owned),
    })
    .map_err(|_| malformed("a client that is not an agent"))
}

async fn secret_of<C: Caller>(
    caller: &C,
    realm: &str,
    client: &AgentClient,
) -> Result<ClientSecret, AgentsError> {
    let body = send(
        caller,
        ApiRequest::new(
            Method::GET,
            format!("{}/client-secret", client_path(realm, client.remote_id())?),
        ),
    )
    .await?;

    text(&body, "client_secret")
        .or_else(|| text(&body["data"], "client_secret"))
        .filter(|secret| !secret.is_empty())
        .map(ClientSecret::new)
        .ok_or_else(|| malformed("a client without a secret"))
}

impl FerriskeyClientsPort for KubeFerriskeyAgents {
    async fn list_clients(
        &self,
        instance: &IdentityInstance,
        realm: &str,
    ) -> Result<Vec<RemoteClient>, AgentsError> {
        list_all(&self.bound(instance), realm).await
    }

    async fn set_client_enabled(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        client: &AgentClient,
        enabled: bool,
    ) -> Result<(), AgentsError> {
        send(
            &self.bound(instance),
            set_enabled_request(realm, client, enabled)?,
        )
        .await
        .map(|_| ())
    }

    async fn delete_client(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        client: &AgentClient,
    ) -> Result<(), AgentsError> {
        send(&self.bound(instance), delete_request(realm, client)?)
            .await
            .map(|_| ())
    }

    async fn create_autonomous_client(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        id: &AgentClientId,
        name: &str,
    ) -> Result<AgentClient, AgentsError> {
        create(&self.bound(instance), realm, id, name).await
    }

    async fn client_secret(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        client: &AgentClient,
    ) -> Result<ClientSecret, AgentsError> {
        secret_of(&self.bound(instance), realm, client).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::super::ferriskey_webhooks::ApiFailure;
    use super::*;

    struct Recorded {
        log: Mutex<Vec<(Method, String, Vec<(&'static str, String)>, Option<Value>)>>,
        answer: Box<dyn Fn(&ApiRequest) -> Result<Value, ApiFailure> + Send + Sync>,
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

        fn calls(&self) -> Vec<(Method, String, Vec<(&'static str, String)>, Option<Value>)> {
            self.log.lock().expect("not poisoned").clone()
        }
    }

    impl Caller for Recorded {
        async fn call(&self, request: &ApiRequest) -> Result<Value, ApiFailure> {
            self.log.lock().expect("not poisoned").push((
                request.method.clone(),
                request.path.clone(),
                request.query.clone(),
                request.body.clone(),
            ));
            (self.answer)(request)
        }
    }

    fn agent(client_id: &str) -> AgentClient {
        AgentClient::try_from(RemoteClient {
            id: "u1".to_string(),
            client_id: client_id.to_string(),
            name: None,
            registration_source: "dynamic".to_string(),
            enabled: true,
            created_at: None,
        })
        .unwrap()
    }

    fn client_json(id: &str, client_id: &str, source: &str) -> Value {
        json!({ "id": id, "client_id": client_id, "name": client_id,
                "registration_source": source, "enabled": true,
                "created_at": "2026-10-23T10:00:00Z", "secret": "must-not-be-read" })
    }

    #[tokio::test]
    async fn every_page_is_read_newest_first_until_the_last() {
        let caller = Recorded::answering(|request| {
            let page: u32 = request
                .query
                .iter()
                .find(|(key, _)| *key == "page")
                .and_then(|(_, value)| value.parse().ok())
                .unwrap();
            Ok(match page {
                1 => json!({ "data": [client_json("u1", "a", "dynamic")],
                             "metadata": { "page": 1, "next_page": 2 } }),
                2 => json!({ "data": [client_json("u2", "b", "metadata_document")],
                             "metadata": { "page": 2, "next_page": null } }),
                _ => unreachable!(),
            })
        });

        let listed = list_all(&caller, "acme").await.expect("listed");

        assert_eq!(
            listed
                .iter()
                .map(|c| c.client_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        let calls = caller.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, "/realms/acme/clients");
        assert!(calls[0].2.contains(&("limit", "100".to_string())));
        assert!(calls[0].2.contains(&("order_by", "created_at".to_string())));
        assert!(calls[0].2.contains(&("order", "desc".to_string())));
    }

    #[tokio::test]
    async fn a_pager_that_never_ends_is_cut_off() {
        let caller = Recorded::answering(|request| {
            let page: u32 = request
                .query
                .iter()
                .find(|(key, _)| *key == "page")
                .and_then(|(_, value)| value.parse().ok())
                .unwrap();
            Ok(json!({ "data": [], "metadata": { "next_page": page + 1 } }))
        });

        list_all(&caller, "acme").await.expect("listed");

        assert_eq!(caller.calls().len() as u32, MAX_PAGES);
    }

    #[test]
    fn a_client_without_a_registration_source_reads_as_admin_made() {
        let parsed = parse_remote(&json!({ "id": "u", "client_id": "c" })).expect("parsed");

        assert_eq!(parsed.registration_source, "admin");
        assert!(parsed.enabled);
    }

    #[test]
    fn the_secret_listed_with_a_client_is_never_read() {
        let parsed = parse_remote(&client_json("u1", "c", "dynamic")).expect("parsed");

        assert!(!format!("{parsed:?}").contains("must-not-be-read"));
    }

    #[test]
    fn disabling_and_deleting_address_the_clients_own_uuid() {
        let client = agent("dcr");

        let patch = set_enabled_request("acme", &client, false).unwrap();
        let delete = delete_request("acme", &client).unwrap();

        assert_eq!(patch.method, Method::PATCH);
        assert_eq!(patch.path, "/realms/acme/clients/u1");
        assert_eq!(patch.body, Some(json!({ "enabled": false })));
        assert_eq!(delete.method, Method::DELETE);
        assert_eq!(delete.path, "/realms/acme/clients/u1");
    }

    #[test]
    fn an_autonomous_client_is_confidential_machine_to_machine_and_has_no_redirect() {
        let id = AgentClientId::autonomous("agent-n8n-abc123").unwrap();

        let request = create_request("acme", &id, "n8n").unwrap();
        let body = request.body.expect("a body");

        assert_eq!(body["client_id"], "agent-n8n-abc123");
        assert_eq!(body["client_type"], "confidential");
        assert_eq!(body["service_account_enabled"], true);
        assert_eq!(body["public_client"], false);
        assert_eq!(body["direct_access_grants_enabled"], false);
        assert_eq!(body["token_exchange_enabled"], false);
        assert!(body.get("redirect_uris").is_none());
    }

    #[tokio::test]
    async fn creating_a_client_returns_it_as_an_agent_made_by_autharie() {
        let caller =
            Recorded::answering(|_| Ok(json!({ "id": "u9", "client_id": "agent-n8n-abc123" })));
        let id = AgentClientId::autonomous("agent-n8n-abc123").unwrap();

        let created = create(&caller, "acme", &id, "n8n").await.expect("created");

        assert_eq!(created.remote_id(), "u9");
        assert_eq!(created.status().source, "autharie");
    }

    #[tokio::test]
    async fn the_secret_is_read_from_the_client_secret_route_and_never_printed() {
        let caller = Recorded::answering(|_| Ok(json!({ "client_secret": "s3cret" })));

        let secret = secret_of(&caller, "acme", &agent("dcr"))
            .await
            .expect("secret");

        assert_eq!(secret.expose(), "s3cret");
        assert_eq!(caller.calls()[0].1, "/realms/acme/clients/u1/client-secret");
        assert!(!format!("{secret:?}").contains("s3cret"));
    }

    #[tokio::test]
    async fn a_client_without_a_secret_is_malformed_not_empty() {
        let caller = Recorded::answering(|_| Ok(json!({ "client_secret": null })));

        assert!(secret_of(&caller, "acme", &agent("dcr")).await.is_err());
    }

    #[test]
    fn a_realm_that_is_not_a_slug_never_reaches_a_path() {
        assert!(list_clients_request("a/b", 1).is_err());
    }
}
