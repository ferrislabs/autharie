use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::agents::AgentError;

pub const MAX_REGISTRY_CLIENTS: usize = 500;
pub const CREDENTIAL_TTL_MINUTES: i64 = 15;
pub const AUTONOMOUS_PREFIX: &str = "agent-";
pub const MIN_SEALING_BITS: usize = 2048;
pub const MAX_SEALING_BITS: usize = 8192;
pub const MAX_AUTONOMOUS_AGENTS: usize = 100;
pub const MAX_LISTED_CLIENTS: usize = 2000;

const MAX_SEALED_BYTES: usize = 4096;
const MAX_CLIENT_ID_CHARS: usize = 512;
const MAX_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 500;
const MAX_SLUG_CHARS: usize = 30;
const MIN_EXPONENT: u64 = 65_537;
const SEALING_ALG: &str = "RSA-OAEP-256";
const SUFFIX_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema)]
#[serde(try_from = "String", into = "String")]
#[schema(value_type = String)]
pub struct AgentClientId(String);

impl AgentClientId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_autonomous(&self) -> bool {
        self.0.starts_with(AUTONOMOUS_PREFIX)
    }

    pub fn generate(name: &str) -> Self {
        let mut slug = String::new();
        for c in name.chars().flat_map(char::to_lowercase) {
            if c.is_ascii_alphanumeric() {
                slug.push(c);
            } else if !slug.is_empty() && !slug.ends_with('-') {
                slug.push('-');
            }
            if slug.len() >= MAX_SLUG_CHARS {
                break;
            }
        }
        let slug = slug.trim_end_matches('-');
        let slug = if slug.is_empty() { "agent" } else { slug };
        let mut rng = rand::rng();
        let suffix: String = (0..6)
            .map(|_| SUFFIX_ALPHABET[rng.random_range(0..SUFFIX_ALPHABET.len())] as char)
            .collect();

        Self(format!("{AUTONOMOUS_PREFIX}{slug}-{suffix}"))
    }
}

impl TryFrom<&str> for AgentClientId {
    type Error = AgentError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let well_formed = !value.is_empty()
            && value.chars().count() <= MAX_CLIENT_ID_CHARS
            && value
                .chars()
                .all(|c| !c.is_control() && !c.is_whitespace() && c != '"' && c != '\\');
        if well_formed {
            Ok(Self(value.to_string()))
        } else {
            Err(AgentError::invalid("the client id is not valid"))
        }
    }
}

impl TryFrom<String> for AgentClientId {
    type Error = AgentError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl From<AgentClientId> for String {
    fn from(value: AgentClientId) -> Self {
        value.0
    }
}

impl fmt::Display for AgentClientId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientSource {
    Dynamic,
    MetadataDocument,
    Autharie,
    Seen,
}

impl ClientSource {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dynamic" => Some(Self::Dynamic),
            "metadata_document" => Some(Self::MetadataDocument),
            "autharie" => Some(Self::Autharie),
            "seen" => Some(Self::Seen),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dynamic => "dynamic",
            Self::MetadataDocument => "metadata_document",
            Self::Autharie => "autharie",
            Self::Seen => "seen",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Delegated,
    Autonomous,
}

impl ClientKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "delegated" => Some(Self::Delegated),
            "autonomous" => Some(Self::Autonomous),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Delegated => "delegated",
            Self::Autonomous => "autonomous",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessState {
    Approved,
    Pending,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedClient {
    pub client_id: AgentClientId,
    pub name: Option<String>,
    pub source: ClientSource,
    pub kind: ClientKind,
    pub enabled: bool,
    pub created_at: Option<DateTime<Utc>>,
}

impl ReportedClient {
    pub fn parse(
        client_id: &str,
        name: Option<&str>,
        source: &str,
        kind: &str,
        enabled: bool,
        created_at: Option<DateTime<Utc>>,
    ) -> Result<Self, AgentError> {
        let source = ClientSource::parse(source)
            .filter(|source| *source != ClientSource::Seen)
            .ok_or_else(|| AgentError::invalid(format!("'{source}' is not a client source")))?;
        let kind = ClientKind::parse(kind)
            .ok_or_else(|| AgentError::invalid(format!("'{kind}' is not a client kind")))?;
        let name = name
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| text.chars().take(MAX_NAME_CHARS).collect());

        Ok(Self {
            client_id: AgentClientId::try_from(client_id)?,
            name,
            source,
            kind,
            enabled,
            created_at,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportedRegistry {
    pub clients: Vec<ReportedClient>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessLists {
    pub approval_required: bool,
    pub approved: BTreeSet<String>,
    pub blocked: BTreeSet<String>,
    pub deleted: BTreeSet<String>,
}

impl AccessLists {
    pub fn state_of(&self, client_id: &str) -> AccessState {
        if self.blocked.contains(client_id) {
            AccessState::Blocked
        } else if !self.approval_required || self.approved.contains(client_id) {
            AccessState::Approved
        } else {
            AccessState::Pending
        }
    }

    pub fn approve(&mut self, client_id: &AgentClientId) {
        self.blocked.remove(client_id.as_str());
        self.approved.insert(client_id.to_string());
        self.deleted.remove(client_id.as_str());
    }

    pub fn block(&mut self, client_id: &AgentClientId) {
        self.blocked.insert(client_id.to_string());
        self.deleted.remove(client_id.as_str());
    }

    pub fn unblock(&mut self, client_id: &AgentClientId) {
        self.blocked.remove(client_id.as_str());
    }

    pub fn forget(&mut self, client_id: &AgentClientId) {
        self.approved.remove(client_id.as_str());
        self.blocked.remove(client_id.as_str());
        self.deleted.insert(client_id.to_string());
    }

    pub fn payload(&self) -> Value {
        json!({
            "approval_required": self.approval_required,
            "approved": self.approved,
            "blocked": self.blocked,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AgentClient {
    pub client_id: AgentClientId,
    pub name: Option<String>,
    pub source: ClientSource,
    pub kind: ClientKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    pub access: AccessState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct RegistryView {
    pub approval_required: bool,
    pub data: Vec<AgentClient>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutonomousAgent {
    pub client_id: AgentClientId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub fn assemble(
    reported: &ReportedRegistry,
    autonomous: &[AutonomousAgent],
    access: &AccessLists,
    activity: &BTreeMap<String, DateTime<Utc>>,
) -> RegistryView {
    let mut known: BTreeSet<&str> = BTreeSet::new();
    let mut data: Vec<AgentClient> = Vec::new();

    for client in &reported.clients {
        if access.deleted.contains(client.client_id.as_str()) {
            continue;
        }
        known.insert(client.client_id.as_str());
        data.push(AgentClient {
            client_id: client.client_id.clone(),
            name: client.name.clone(),
            source: client.source,
            kind: client.kind,
            enabled: Some(client.enabled),
            access: access.state_of(client.client_id.as_str()),
            created_at: client.created_at,
            last_seen_at: activity.get(client.client_id.as_str()).copied(),
        });
    }

    for agent in autonomous {
        if !known.insert(agent.client_id.as_str()) {
            continue;
        }
        data.push(AgentClient {
            client_id: agent.client_id.clone(),
            name: Some(agent.name.clone()),
            source: ClientSource::Autharie,
            kind: ClientKind::Autonomous,
            enabled: None,
            access: access.state_of(agent.client_id.as_str()),
            created_at: Some(agent.created_at),
            last_seen_at: activity.get(agent.client_id.as_str()).copied(),
        });
    }

    for (seen, at) in activity {
        if known.contains(seen.as_str()) || access.deleted.contains(seen) {
            continue;
        }
        let Ok(client_id) = AgentClientId::try_from(seen.as_str()) else {
            continue;
        };
        data.push(AgentClient {
            kind: if client_id.is_autonomous() {
                ClientKind::Autonomous
            } else {
                ClientKind::Delegated
            },
            access: access.state_of(seen),
            client_id,
            name: None,
            source: ClientSource::Seen,
            enabled: None,
            created_at: None,
            last_seen_at: Some(*at),
        });
    }

    data.sort_by(|a, b| {
        b.last_seen_at
            .cmp(&a.last_seen_at)
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.client_id.cmp(&b.client_id))
    });
    data.truncate(MAX_LISTED_CLIENTS);

    RegistryView {
        approval_required: access.approval_required,
        data,
        truncated: reported.truncated,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SealingJwk {
    pub kty: String,
    pub n: String,
    pub e: String,
    #[serde(default)]
    pub alg: Option<String>,
    #[serde(default)]
    pub d: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealingKey {
    kty: String,
    n: String,
    e: String,
    alg: String,
}

fn bits(bytes: &[u8]) -> usize {
    match bytes.iter().position(|byte| *byte != 0) {
        None => 0,
        Some(first) => (bytes.len() - first) * 8 - bytes[first].leading_zeros() as usize,
    }
}

fn refuse(reason: &str) -> AgentError {
    AgentError::InvalidSealingKey {
        reason: reason.to_string(),
    }
}

impl SealingKey {
    pub fn modulus(&self) -> &str {
        &self.n
    }

    pub fn exponent(&self) -> &str {
        &self.e
    }

    pub fn payload(&self) -> Value {
        json!({ "kty": self.kty, "n": self.n, "e": self.e, "alg": self.alg })
    }
}

impl TryFrom<SealingJwk> for SealingKey {
    type Error = AgentError;

    fn try_from(jwk: SealingJwk) -> Result<Self, Self::Error> {
        if jwk.d.is_some() {
            return Err(refuse("the key must be a public key"));
        }
        if jwk.kty != "RSA" {
            return Err(refuse("the key must be an RSA key"));
        }
        if jwk.alg.as_deref() != Some(SEALING_ALG) {
            return Err(refuse("the key must be meant for RSA-OAEP-256"));
        }

        let modulus = URL_SAFE_NO_PAD
            .decode(&jwk.n)
            .map_err(|_| refuse("the modulus is not base64url"))?;
        let size = bits(&modulus);
        if size < MIN_SEALING_BITS {
            return Err(refuse("the key must be at least 2048 bits"));
        }
        if size > MAX_SEALING_BITS {
            return Err(refuse("the key must be at most 8192 bits"));
        }
        if modulus.last().is_some_and(|byte| byte % 2 == 0) {
            return Err(refuse("the modulus is not valid"));
        }

        let exponent = URL_SAFE_NO_PAD
            .decode(&jwk.e)
            .map_err(|_| refuse("the exponent is not base64url"))?;
        if exponent.len() > 8 {
            return Err(refuse("the exponent is too large"));
        }
        let exponent = exponent
            .iter()
            .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
        if exponent < MIN_EXPONENT || exponent % 2 == 0 {
            return Err(refuse("the exponent is not valid"));
        }

        Ok(Self {
            kty: jwk.kty,
            n: jwk.n,
            e: jwk.e,
            alg: SEALING_ALG.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutonomousInput {
    pub name: String,
    pub description: Option<String>,
    pub sealing_key: SealingKey,
}

impl AutonomousInput {
    pub fn parse(
        name: &str,
        description: Option<&str>,
        sealing_key: SealingJwk,
    ) -> Result<Self, AgentError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(AgentError::invalid(format!(
                "the name must be 1 to {MAX_NAME_CHARS} characters"
            )));
        }

        let description = description.map(str::trim).filter(|text| !text.is_empty());
        if description.is_some_and(|text| text.chars().count() > MAX_DESCRIPTION_CHARS) {
            return Err(AgentError::invalid(format!(
                "the description is at most {MAX_DESCRIPTION_CHARS} characters"
            )));
        }

        Ok(Self {
            name: name.to_string(),
            description: description.map(str::to_string),
            sealing_key: SealingKey::try_from(sealing_key)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Create,
    Rotate,
}

impl RequestKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "create" => Some(Self::Create),
            "rotate" => Some(Self::Rotate),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Rotate => "rotate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRequest {
    pub request_id: Uuid,
    pub client_id: AgentClientId,
    pub kind: RequestKind,
    pub sealing_key: SealingKey,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SealedCredential {
    pub client_id: AgentClientId,
    pub sealed: String,
}

impl fmt::Debug for SealedCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealedCredential")
            .field("client_id", &self.client_id)
            .field("sealed", &"<sealed>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialState {
    Missing,
    Delivered,
    Pending,
    Ready,
}

#[derive(Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct DeliveredCredential {
    pub client_id: AgentClientId,
    pub sealed: String,
    pub token_url: String,
    pub audience: String,
}

impl fmt::Debug for DeliveredCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeliveredCredential")
            .field("client_id", &self.client_id)
            .field("sealed", &"<sealed>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialFetch {
    Pending,
    Delivered(DeliveredCredential),
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReportedCredential {
    pub request_id: Uuid,
    pub client_id: AgentClientId,
    pub sealed: String,
}

impl fmt::Debug for ReportedCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReportedCredential")
            .field("request_id", &self.request_id)
            .field("client_id", &self.client_id)
            .field("sealed", &"<sealed>")
            .finish()
    }
}

impl ReportedCredential {
    pub fn parse(request_id: Uuid, client_id: &str, sealed: &str) -> Result<Self, AgentError> {
        let well_formed = !sealed.is_empty()
            && sealed.len() <= MAX_SEALED_BYTES
            && sealed
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'='));
        if !well_formed {
            return Err(AgentError::invalid("a sealed credential is not valid"));
        }

        Ok(Self {
            request_id,
            client_id: AgentClientId::try_from(client_id)?,
            sealed: sealed.to_string(),
        })
    }
}

pub fn token_url(mcp_url: &str, realm: &str) -> Option<String> {
    let origin = mcp_url.strip_suffix("/mcp")?;
    Some(format!(
        "{origin}/api/realms/{realm}/protocol/openid-connect/token"
    ))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn modulus(bytes: usize) -> String {
        let mut raw = vec![0xffu8; bytes];
        let last = raw.len() - 1;
        raw[last] = 0xfd;
        URL_SAFE_NO_PAD.encode(raw)
    }

    fn jwk(bytes: usize) -> SealingJwk {
        SealingJwk {
            kty: "RSA".to_string(),
            n: modulus(bytes),
            e: "AQAB".to_string(),
            alg: Some("RSA-OAEP-256".to_string()),
            d: None,
        }
    }

    fn id(value: &str) -> AgentClientId {
        AgentClientId::try_from(value).unwrap()
    }

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    #[test]
    fn a_2048_bit_rsa_key_for_oaep_256_is_a_sealing_key() {
        let key = SealingKey::try_from(jwk(256)).expect("accepted");

        assert_eq!(key.exponent(), "AQAB");
        assert_eq!(
            key.payload(),
            json!({ "kty": "RSA", "n": modulus(256), "e": "AQAB", "alg": "RSA-OAEP-256" })
        );
    }

    #[test]
    fn a_key_under_2048_bits_is_refused() {
        assert!(matches!(
            SealingKey::try_from(jwk(255)),
            Err(AgentError::InvalidSealingKey { .. })
        ));
    }

    #[test]
    fn leading_zero_bytes_do_not_count_toward_the_size() {
        let mut padded = URL_SAFE_NO_PAD.decode(modulus(255)).unwrap();
        padded.insert(0, 0);
        let mut key = jwk(255);
        key.n = URL_SAFE_NO_PAD.encode(padded);

        assert!(SealingKey::try_from(key).is_err());
    }

    #[test]
    fn a_key_over_8192_bits_is_refused() {
        assert!(SealingKey::try_from(jwk(1025)).is_err());
    }

    #[test]
    fn a_key_that_is_not_rsa_or_not_for_oaep_256_is_refused() {
        let mut ec = jwk(256);
        ec.kty = "EC".to_string();
        let mut other = jwk(256);
        other.alg = Some("RSA-OAEP".to_string());
        let mut unspecified = jwk(256);
        unspecified.alg = None;

        assert!(SealingKey::try_from(ec).is_err());
        assert!(SealingKey::try_from(other).is_err());
        assert!(SealingKey::try_from(unspecified).is_err());
    }

    #[test]
    fn a_private_key_is_refused_so_it_never_travels() {
        let mut private = jwk(256);
        private.d = Some("AAAA".to_string());

        assert!(SealingKey::try_from(private).is_err());
    }

    #[test]
    fn a_malformed_modulus_or_a_weak_exponent_is_refused() {
        let mut garbled = jwk(256);
        garbled.n = "***".to_string();
        let mut weak = jwk(256);
        weak.e = "Aw".to_string();
        let mut even = jwk(256);
        even.e = "AQAC".to_string();

        assert!(SealingKey::try_from(garbled).is_err());
        assert!(SealingKey::try_from(weak).is_err());
        assert!(SealingKey::try_from(even).is_err());
    }

    #[test]
    fn an_autonomous_name_is_trimmed_and_bounded() {
        let input = AutonomousInput::parse(" n8n ", Some(" "), jwk(256)).expect("accepted");

        assert_eq!(input.name, "n8n");
        assert_eq!(input.description, None);
        assert!(AutonomousInput::parse(" ", None, jwk(256)).is_err());
        assert!(AutonomousInput::parse(&"a".repeat(101), None, jwk(256)).is_err());
    }

    #[test]
    fn a_generated_client_id_carries_the_autonomous_prefix_a_slug_and_a_suffix() {
        let generated = AgentClientId::generate("My n8n Workflow!");

        assert!(generated.is_autonomous());
        let text = generated.as_str();
        assert!(text.starts_with("agent-my-n8n-workflow-"), "{text}");
        assert_eq!(text.len(), "agent-my-n8n-workflow-".len() + 6);
        assert_ne!(generated, AgentClientId::generate("My n8n Workflow!"));
        assert!(
            AgentClientId::generate("!!!")
                .as_str()
                .starts_with("agent-agent-")
        );
    }

    #[test]
    fn a_client_id_with_whitespace_quotes_or_nothing_is_refused() {
        for bad in ["", "a b", "a\"b", "a\nb"] {
            assert!(AgentClientId::try_from(bad).is_err(), "{bad:?}");
        }
        assert!(AgentClientId::try_from("https://app.example/client.json").is_ok());
    }

    #[test]
    fn access_is_blocked_then_approved_then_pending() {
        let mut lists = AccessLists {
            approval_required: true,
            ..AccessLists::default()
        };
        lists.approved.insert("a".to_string());
        lists.blocked.insert("b".to_string());
        lists.approved.insert("b".to_string());

        assert_eq!(lists.state_of("a"), AccessState::Approved);
        assert_eq!(lists.state_of("b"), AccessState::Blocked);
        assert_eq!(lists.state_of("c"), AccessState::Pending);

        lists.approval_required = false;
        assert_eq!(lists.state_of("c"), AccessState::Approved);
        assert_eq!(lists.state_of("b"), AccessState::Blocked);
    }

    #[test]
    fn approving_unblocks_and_blocking_keeps_the_approval_for_later() {
        let mut lists = AccessLists::default();
        lists.approve(&id("a"));
        lists.block(&id("a"));

        assert!(lists.approved.contains("a"));
        assert_eq!(lists.state_of("a"), AccessState::Blocked);

        lists.unblock(&id("a"));
        lists.approval_required = true;
        assert_eq!(lists.state_of("a"), AccessState::Approved);

        lists.block(&id("a"));
        lists.approve(&id("a"));
        assert!(!lists.blocked.contains("a"));
    }

    #[test]
    fn forgetting_an_agent_drops_it_from_both_lists_and_remembers_the_deletion() {
        let mut lists = AccessLists::default();
        lists.approve(&id("a"));
        lists.block(&id("a"));
        lists.forget(&id("a"));

        assert!(lists.approved.is_empty() && lists.blocked.is_empty());
        assert!(lists.deleted.contains("a"));
        lists.approve(&id("a"));
        assert!(lists.deleted.is_empty());
    }

    fn reported(client: &str, source: ClientSource) -> ReportedClient {
        ReportedClient {
            client_id: id(client),
            name: Some(client.to_string()),
            source,
            kind: ClientKind::Delegated,
            enabled: true,
            created_at: Some(at(10)),
        }
    }

    #[test]
    fn the_registry_merges_reported_clients_access_and_activity() {
        let registry = ReportedRegistry {
            clients: vec![
                reported("dcr", ClientSource::Dynamic),
                reported("cimd", ClientSource::MetadataDocument),
            ],
            truncated: false,
        };
        let mut lists = AccessLists {
            approval_required: true,
            ..AccessLists::default()
        };
        lists.approved.insert("dcr".to_string());
        let activity = BTreeMap::from([("dcr".to_string(), at(500))]);

        let view = assemble(&registry, &[], &lists, &activity);

        assert_eq!(view.data.len(), 2);
        assert_eq!(view.data[0].client_id.as_str(), "dcr");
        assert_eq!(view.data[0].access, AccessState::Approved);
        assert_eq!(view.data[0].last_seen_at, Some(at(500)));
        assert_eq!(view.data[1].client_id.as_str(), "cimd");
        assert_eq!(view.data[1].access, AccessState::Pending);
        assert_eq!(view.data[1].last_seen_at, None);
    }

    #[test]
    fn a_client_only_seen_in_decisions_is_listed_as_seen() {
        let activity = BTreeMap::from([
            ("ghost".to_string(), at(5)),
            ("agent-ci-abc123".to_string(), at(6)),
        ]);

        let view = assemble(
            &ReportedRegistry::default(),
            &[],
            &AccessLists::default(),
            &activity,
        );

        assert!(view.data.iter().all(|c| c.source == ClientSource::Seen));
        let ghost = view
            .data
            .iter()
            .find(|c| c.client_id.as_str() == "ghost")
            .unwrap();
        assert_eq!(ghost.kind, ClientKind::Delegated);
        assert_eq!(ghost.enabled, None);
        let auto = view
            .data
            .iter()
            .find(|c| c.client_id.as_str() == "agent-ci-abc123")
            .unwrap();
        assert_eq!(auto.kind, ClientKind::Autonomous);
    }

    #[test]
    fn a_deleted_agent_is_not_listed_even_if_it_is_still_reported_or_seen() {
        let registry = ReportedRegistry {
            clients: vec![reported("gone", ClientSource::Dynamic)],
            truncated: false,
        };
        let mut lists = AccessLists::default();
        lists.forget(&id("gone"));
        lists.forget(&id("also"));
        let activity = BTreeMap::from([("also".to_string(), at(5))]);

        assert!(assemble(&registry, &[], &lists, &activity).data.is_empty());
    }

    #[test]
    fn an_autonomous_agent_not_yet_reported_is_listed_from_what_the_control_plane_knows() {
        let created = AutonomousAgent {
            client_id: id("agent-n8n-abc123"),
            name: "n8n".to_string(),
            description: None,
            created_at: at(7),
        };

        let view = assemble(
            &ReportedRegistry::default(),
            &[created],
            &AccessLists::default(),
            &BTreeMap::new(),
        );

        assert_eq!(view.data[0].source, ClientSource::Autharie);
        assert_eq!(view.data[0].kind, ClientKind::Autonomous);
        assert_eq!(view.data[0].name.as_deref(), Some("n8n"));
    }

    #[test]
    fn the_approval_mode_is_part_of_the_view() {
        let lists = AccessLists {
            approval_required: true,
            ..AccessLists::default()
        };

        assert!(
            assemble(&ReportedRegistry::default(), &[], &lists, &BTreeMap::new()).approval_required
        );
    }

    #[test]
    fn truncation_is_passed_on() {
        let registry = ReportedRegistry {
            clients: vec![],
            truncated: true,
        };

        assert!(assemble(&registry, &[], &AccessLists::default(), &BTreeMap::new()).truncated);
    }

    #[test]
    fn a_sealed_credential_never_prints() {
        let credential = ReportedCredential {
            request_id: Uuid::nil(),
            client_id: id("agent-a-b"),
            sealed: "TOP-SECRET".to_string(),
        };
        let delivered = DeliveredCredential {
            client_id: id("agent-a-b"),
            sealed: "TOP-SECRET".to_string(),
            token_url: "t".to_string(),
            audience: "a".to_string(),
        };

        assert!(!format!("{credential:?}").contains("TOP-SECRET"));
        assert!(!format!("{delivered:?}").contains("TOP-SECRET"));
    }

    #[test]
    fn the_token_url_follows_the_public_mcp_url() {
        assert_eq!(
            token_url("https://auth.acme.test:8443/mcp", "acme").as_deref(),
            Some("https://auth.acme.test:8443/api/realms/acme/protocol/openid-connect/token")
        );
        assert_eq!(token_url("https://auth.acme.test/other", "acme"), None);
    }
}
