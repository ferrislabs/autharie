use std::collections::BTreeSet;

use autharie_crds::v1alpha::identity_instance::{
    AgentAccessSpec, AgentClientStatus, AgentCredentialStatus, AutonomousAgentSpec, SealingKeySpec,
};
use chrono::{DateTime, Utc};

pub const AUTONOMOUS_PREFIX: &str = "agent-";
pub const MAX_REPORTED_CLIENTS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAnAgent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteClient {
    pub id: String,
    pub client_id: String,
    pub name: Option<String>,
    pub registration_source: String,
    pub enabled: bool,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSource {
    Dynamic,
    MetadataDocument,
    Autharie,
}

impl AgentSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dynamic => "dynamic",
            Self::MetadataDocument => "metadata_document",
            Self::Autharie => "autharie",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentClientId(String);

impl AgentClientId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn autonomous(client_id: &str) -> Result<Self, NotAnAgent> {
        let well_formed = client_id.starts_with(AUTONOMOUS_PREFIX)
            && client_id.len() > AUTONOMOUS_PREFIX.len()
            && client_id.len() <= 128
            && client_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
        if well_formed {
            Ok(Self(client_id.to_string()))
        } else {
            Err(NotAnAgent)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentClient {
    id: AgentClientId,
    remote_id: String,
    name: Option<String>,
    source: AgentSource,
    enabled: bool,
    created_at: Option<String>,
}

impl TryFrom<RemoteClient> for AgentClient {
    type Error = NotAnAgent;

    fn try_from(client: RemoteClient) -> Result<Self, Self::Error> {
        let (id, source) = match client.registration_source.as_str() {
            "dynamic" => (
                AgentClientId(client.client_id.clone()),
                AgentSource::Dynamic,
            ),
            "metadata_document" => (
                AgentClientId(client.client_id.clone()),
                AgentSource::MetadataDocument,
            ),
            "admin" => (
                AgentClientId::autonomous(&client.client_id)?,
                AgentSource::Autharie,
            ),
            _ => return Err(NotAnAgent),
        };
        if id.as_str().is_empty() || client.id.is_empty() {
            return Err(NotAnAgent);
        }

        Ok(Self {
            id,
            remote_id: client.id,
            name: client.name,
            source,
            enabled: client.enabled,
            created_at: client.created_at,
        })
    }
}

impl AgentClient {
    pub fn id(&self) -> &AgentClientId {
        &self.id
    }

    pub fn client_id(&self) -> &str {
        self.id.as_str()
    }

    pub fn remote_id(&self) -> &str {
        &self.remote_id
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn created(&self) -> Option<DateTime<Utc>> {
        self.created_at
            .as_deref()
            .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
            .map(|time| time.with_timezone(&Utc))
    }

    pub fn status(&self) -> AgentClientStatus {
        AgentClientStatus {
            client_id: self.id.0.clone(),
            name: self
                .name
                .clone()
                .filter(|name| !name.trim().is_empty() && name != &self.id.0),
            source: self.source.as_str().to_string(),
            kind: match self.source {
                AgentSource::Autharie => "autonomous",
                AgentSource::Dynamic | AgentSource::MetadataDocument => "delegated",
            }
            .to_string(),
            enabled: self.enabled,
            created_at: self.created_at.clone(),
        }
    }
}

pub fn agents_among(remote: Vec<RemoteClient>) -> Vec<AgentClient> {
    remote
        .into_iter()
        .filter_map(|client| AgentClient::try_from(client).ok())
        .collect()
}

pub fn report(found: &[AgentClient]) -> (Vec<AgentClientStatus>, bool) {
    let mut sorted: Vec<&AgentClient> = found.iter().collect();
    sorted.sort_by(|a, b| {
        b.created()
            .cmp(&a.created())
            .then_with(|| a.client_id().cmp(b.client_id()))
    });
    let truncated = sorted.len() > MAX_REPORTED_CLIENTS;
    sorted.truncate(MAX_REPORTED_CLIENTS);

    (
        sorted.into_iter().map(AgentClient::status).collect(),
        truncated,
    )
}

#[derive(Debug, Default)]
pub struct RegistryPlan<'a> {
    pub delete: Vec<&'a AgentClient>,
    pub disable: Vec<&'a AgentClient>,
    pub adopt: Vec<&'a AgentClient>,
    pub enable: Vec<&'a AgentClient>,
}

pub fn plan<'a>(
    agents: &'a [AgentClient],
    access: &AgentAccessSpec,
    deleted: &[String],
    disabled_by_us: &BTreeSet<String>,
) -> RegistryPlan<'a> {
    let mut plan = RegistryPlan::default();

    for agent in agents {
        let id = agent.client_id();
        if deleted.iter().any(|gone| gone == id) {
            plan.delete.push(agent);
            continue;
        }

        let blocked = access.blocked.iter().any(|blocked| blocked == id);
        match (blocked, agent.enabled()) {
            (true, true) => plan.disable.push(agent),
            (true, false) => plan.adopt.push(agent),
            (false, false) if disabled_by_us.contains(id) => plan.enable.push(agent),
            _ => {}
        }
    }

    plan
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Create,
    Rotate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealRequest<'a> {
    pub request_id: &'a str,
    pub kind: RequestKind,
    pub client_id: &'a str,
    pub key: &'a SealingKeySpec,
}

pub fn seal_requests(autonomous: &[AutonomousAgentSpec]) -> Vec<SealRequest<'_>> {
    autonomous
        .iter()
        .filter_map(|agent| {
            let key = agent.sealing_key.as_ref()?;
            let (request_id, kind) = match (&agent.request_id, &agent.rotate_request_id) {
                (Some(id), None) => (id.as_str(), RequestKind::Create),
                (None, Some(id)) => (id.as_str(), RequestKind::Rotate),
                _ => return None,
            };
            Some(SealRequest {
                request_id,
                kind,
                client_id: &agent.client_id,
                key,
            })
        })
        .collect()
}

pub fn still_asked(
    credentials: &[AgentCredentialStatus],
    requests: &[SealRequest<'_>],
) -> Vec<AgentCredentialStatus> {
    credentials
        .iter()
        .filter(|credential| {
            requests
                .iter()
                .any(|request| request.request_id == credential.request_id)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(client_id: &str, source: &str, enabled: bool, created: Option<&str>) -> RemoteClient {
        RemoteClient {
            id: format!("uuid-{client_id}"),
            client_id: client_id.to_string(),
            name: Some(client_id.to_string()),
            registration_source: source.to_string(),
            enabled,
            created_at: created.map(str::to_string),
        }
    }

    fn agent(client_id: &str, source: &str, enabled: bool) -> AgentClient {
        AgentClient::try_from(remote(client_id, source, enabled, None)).expect("an agent")
    }

    #[test]
    fn spec_ar_1_dcr_and_cimd_clients_are_agents_and_admin_made_clients_are_not() {
        let listed = vec![
            remote("dcr-client", "dynamic", true, None),
            remote(
                "https://app.example/client.json",
                "metadata_document",
                true,
                None,
            ),
            remote("mcp-gateway", "admin", true, None),
            remote("console", "admin", true, None),
            remote("master-realm", "admin", true, None),
        ];

        let found: Vec<_> = agents_among(listed)
            .iter()
            .map(|a| a.client_id().to_string())
            .collect();

        assert_eq!(found, vec!["dcr-client", "https://app.example/client.json"]);
    }

    #[test]
    fn a_client_autharie_created_is_an_admin_client_with_the_agent_prefix() {
        let made = AgentClient::try_from(remote("agent-n8n-abc123", "admin", true, None))
            .expect("an agent");

        assert_eq!(made.status().source, "autharie");
        assert_eq!(made.status().kind, "autonomous");
    }

    #[test]
    fn an_unknown_registration_source_is_never_an_agent_even_with_the_prefix() {
        assert!(AgentClient::try_from(remote("agent-x", "federation", true, None)).is_err());
    }

    #[test]
    fn the_prefix_alone_does_not_make_a_dynamic_look_alike_autonomous() {
        let dcr = AgentClient::try_from(remote("agent-lookalike", "dynamic", true, None)).unwrap();

        assert_eq!(dcr.status().source, "dynamic");
        assert_eq!(dcr.status().kind, "delegated");
    }

    #[test]
    fn an_autonomous_id_needs_the_prefix_a_suffix_and_a_plain_charset() {
        assert!(AgentClientId::autonomous("agent-n8n-abc123").is_ok());
        for bad in [
            "",
            "agent-",
            "mcp-gateway",
            "agent-a b",
            "agent-a/b",
            "Agent-x",
        ] {
            assert!(AgentClientId::autonomous(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn spec_ar_8_a_forged_desired_state_names_nothing_outside_the_agent_set() {
        let found = agents_among(vec![
            remote("dcr-client", "dynamic", true, None),
            remote("mcp-gateway", "admin", true, None),
            remote("console", "admin", true, None),
        ]);
        let access = AgentAccessSpec {
            approval_required: true,
            approved: vec![],
            blocked: vec!["mcp-gateway".to_string(), "console".to_string()],
        };
        let deleted = vec!["mcp-gateway".to_string(), "console".to_string()];

        let plan = plan(&found, &access, &deleted, &BTreeSet::new());

        assert!(plan.delete.is_empty());
        assert!(plan.disable.is_empty());
        assert!(plan.adopt.is_empty());
        assert!(plan.enable.is_empty());
    }

    #[test]
    fn a_blocked_enabled_agent_is_disabled_and_a_blocked_disabled_one_is_adopted() {
        let found = vec![agent("a", "dynamic", true), agent("b", "dynamic", false)];
        let access = AgentAccessSpec {
            blocked: vec!["a".to_string(), "b".to_string()],
            ..Default::default()
        };

        let plan = plan(&found, &access, &[], &BTreeSet::new());

        assert_eq!(plan.disable.len(), 1);
        assert_eq!(plan.disable[0].client_id(), "a");
        assert_eq!(plan.adopt[0].client_id(), "b");
    }

    #[test]
    fn unblocking_enables_only_what_we_disabled() {
        let found = vec![
            agent("ours", "dynamic", false),
            agent("theirs", "dynamic", false),
        ];
        let ours = BTreeSet::from(["ours".to_string()]);

        let plan = plan(&found, &AgentAccessSpec::default(), &[], &ours);

        assert_eq!(plan.enable.len(), 1);
        assert_eq!(plan.enable[0].client_id(), "ours");
    }

    #[test]
    fn a_deleted_agent_is_deleted_not_also_blocked_or_enabled() {
        let found = vec![agent("a", "dynamic", false)];
        let access = AgentAccessSpec {
            blocked: vec!["a".to_string()],
            ..Default::default()
        };
        let ours = BTreeSet::from(["a".to_string()]);

        let plan = plan(&found, &access, &["a".to_string()], &ours);

        assert_eq!(plan.delete.len(), 1);
        assert!(plan.disable.is_empty() && plan.adopt.is_empty() && plan.enable.is_empty());
    }

    #[test]
    fn spec_ar_1_the_report_is_bounded_to_500_newest_first_with_a_truncated_flag() {
        let many: Vec<AgentClient> = (0..520)
            .map(|n| {
                AgentClient::try_from(remote(
                    &format!("client-{n:03}"),
                    "dynamic",
                    true,
                    Some(&format!("2026-10-23T10:{:02}:{:02}Z", n / 60, n % 60)),
                ))
                .unwrap()
            })
            .collect();

        let (clients, truncated) = report(&many);

        assert_eq!(clients.len(), 500);
        assert!(truncated);
        assert_eq!(clients[0].client_id, "client-519");
        assert_eq!(clients[499].client_id, "client-020");
    }

    #[test]
    fn a_small_registry_is_not_truncated_and_undated_clients_come_last() {
        let found = vec![
            agent("undated", "dynamic", true),
            AgentClient::try_from(remote(
                "dated",
                "dynamic",
                true,
                Some("2026-10-23T10:00:00Z"),
            ))
            .unwrap(),
        ];

        let (clients, truncated) = report(&found);

        assert!(!truncated);
        assert_eq!(clients[0].client_id, "dated");
        assert_eq!(clients[1].client_id, "undated");
    }

    #[test]
    fn a_name_equal_to_the_client_id_is_not_repeated() {
        let found = agent("same", "dynamic", true);

        assert_eq!(found.status().name, None);
    }

    fn spec(
        client_id: &str,
        request: Option<&str>,
        rotate: Option<&str>,
        key: bool,
    ) -> AutonomousAgentSpec {
        AutonomousAgentSpec {
            client_id: client_id.to_string(),
            name: "n".to_string(),
            request_id: request.map(str::to_string),
            rotate_request_id: rotate.map(str::to_string),
            sealing_key: key.then(|| SealingKeySpec {
                kty: "RSA".to_string(),
                n: "AA".to_string(),
                e: "AQAB".to_string(),
                alg: "RSA-OAEP-256".to_string(),
            }),
        }
    }

    #[test]
    fn only_entries_with_one_request_and_a_key_are_seal_requests() {
        let entries = vec![
            spec("agent-a-1", Some("r1"), None, true),
            spec("agent-b-1", None, Some("r2"), true),
            spec("agent-c-1", None, None, false),
            spec("agent-d-1", Some("r3"), None, false),
            spec("agent-e-1", Some("r4"), Some("r5"), true),
        ];

        let requests = seal_requests(&entries);

        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].kind, RequestKind::Create);
        assert_eq!(requests[1].kind, RequestKind::Rotate);
        assert_eq!(requests[1].request_id, "r2");
    }

    #[test]
    fn a_sealed_credential_is_kept_only_while_its_request_is_asked() {
        let entries = vec![spec("agent-a-1", Some("r1"), None, true)];
        let credentials = vec![
            AgentCredentialStatus {
                request_id: "r1".to_string(),
                client_id: "agent-a-1".to_string(),
                sealed: "x".to_string(),
            },
            AgentCredentialStatus {
                request_id: "old".to_string(),
                client_id: "agent-a-1".to_string(),
                sealed: "y".to_string(),
            },
        ];

        let kept = still_asked(&credentials, &seal_requests(&entries));

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].request_id, "r1");
    }
}
