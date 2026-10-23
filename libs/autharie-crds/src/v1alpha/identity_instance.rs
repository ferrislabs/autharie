use std::fmt::Display;

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::types::{Condition, Phase, ResourceRequirements};

#[derive(CustomResource, Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "autharie.fr",
    version = "v1alpha",
    kind = "IdentityInstance",
    plural = "identityinstances",
    shortname = "ii",
    namespaced,
    status = "IdentityInstanceStatus",
    printcolumn = r#"{"name":"Provider", "type":"string", "jsonPath":".spec.provider"}"#,
    printcolumn = r#"{"name":"Version", "type":"string", "jsonPath":".spec.version"}"#,
    printcolumn = r#"{"name":"Phase", "type":"string", "jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Ready", "type":"boolean", "jsonPath":".status.ready"}"#,
    printcolumn = r#"{"name":"Age", "type":"date", "jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct IdentityInstanceSpec {
    pub organisation_id: String,

    pub provider: IdentityProvider,

    pub version: String,

    pub hostname: String,

    /// Replicas of a FerrisKey instance (API and webapp), 1 to 5. Keycloak
    /// always runs one.
    #[serde(default = "default_replicas")]
    #[schemars(range(min = 1, max = 5))]
    pub replicas: i32,

    /// Scaled to nothing, with the database hibernated and its volumes kept.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped: bool,

    pub database: DatabaseConfig,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ferriskey: Option<FerriskeyConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress: Option<IngressConfig>,

    /// Source ranges allowed to reach this instance, in CIDR notation.
    ///
    /// Absent is open, and so is an empty list: there is no way to spell
    /// "reachable by nobody" here, the same way there is none in the control
    /// plane's own type. An instance whose allow list is cleared goes back to
    /// being reachable rather than disappearing from the network.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_cidrs: Option<Vec<String>>,

    /// Where this instance's database is archived, and with what.
    ///
    /// Absent means nothing is archived. That is a decision an installation can
    /// make, and it is a different one from a store being unreachable, so it is
    /// expressed as absence rather than as an empty destination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<BackupConfig>,

    /// Where this instance's database comes from, when it is a recovery.
    ///
    /// Absent is the ordinary case: a new instance starts empty. Present, the
    /// database is bootstrapped from somebody else's archive instead -- which
    /// is read-only from this instance's side, so the deployment it restores
    /// is never touched.
    ///
    /// Set once, when the instance is created, and meaningless afterwards: a
    /// cluster already running is not re-bootstrapped by changing this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore: Option<RestoreConfig>,

    /// How the identity provider behaves and looks for this instance.
    ///
    /// Absent means the provider's own defaults. The control plane writes the
    /// whole value each time, so what is not named here is not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iam: Option<IamConfig>,

    /// The webhooks this deployment should have, whole. FerrisKey only.
    ///
    /// The operator makes FerrisKey match this list for the webhooks it
    /// manages and leaves every other webhook alone. Header values are not
    /// here: they live in the `<instance>-webhook-headers` Secret.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webhooks: Vec<WebhookSpec>,

    /// Which version of the desired set this is, so a reader can tell an old
    /// list from a new one. The operator does not act on it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub webhooks_revision: u64,

    /// One-off asks (rotate a secret, retry a delivery). Each id is executed
    /// once and its result recorded in `status.webhookRequests`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webhook_requests: Vec<WebhookRequest>,

    /// The agent settings this deployment should have, whole. FerrisKey only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<AgentsSpec>,

    /// The outgoing mail settings of each realm that has some. FerrisKey only.
    ///
    /// One entry per realm, each with its own revision. A removal stays as an
    /// entry with `remove: true`, so an older save delivered late cannot bring
    /// the provider back. The password is not here: it is in the
    /// `<instance>-smtp` Secret, under the realm's name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub smtp: Vec<SmtpSpec>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub portal_themes: Vec<PortalThemeSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalThemeSpec {
    pub realm: String,

    pub revision: u64,

    #[schemars(schema_with = "free_form_object")]
    pub config: serde_json::Value,
}

fn free_form_object(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "object",
        "x-kubernetes-preserve-unknown-fields": true
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PortalThemePhase {
    Pending,
    Applied,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalThemeStatus {
    pub realm: String,

    pub revision: u64,

    pub phase: PortalThemePhase,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SmtpSpec {
    pub realm: String,

    /// Grows with every save of this realm's settings; the operator acts on an
    /// entry whose revision it has not reported yet.
    pub revision: u64,

    /// The realm has no mail provider. The other fields are then not read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remove: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 65535))]
    pub port: Option<u16>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<SmtpEncryption>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_email: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SmtpEncryption {
    Tls,
    Starttls,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SmtpPhase {
    Pending,
    Applied,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SmtpStatus {
    pub realm: String,

    /// The revision of `spec.smtp` this entry reports on.
    pub revision: u64,

    pub phase: SmtpPhase,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentsSpec {
    pub enabled: bool,

    pub realm: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policies: Vec<AgentPolicySpec>,

    #[serde(default, skip_serializing_if = "is_zero")]
    pub policies_revision: u64,

    /// The customer MCP servers agents may reach through the gateway.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<AgentServerSpec>,
    #[serde(default, skip_serializing_if = "AgentAccessSpec::is_open")]
    pub access: AgentAccessSpec,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub autonomous: Vec<AutonomousAgentSpec>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_agents: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentAccessSpec {
    #[serde(default)]
    pub approval_required: bool,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approved: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked: Vec<String>,
}

impl AgentAccessSpec {
    fn is_open(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutonomousAgentSpec {
    pub client_id: String,

    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate_request_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealing_key: Option<SealingKeySpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SealingKeySpec {
    pub kty: String,
    pub n: String,
    pub e: String,
    pub alg: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentServerSpec {
    /// Autharie's id for the server (a UUID).
    pub id: String,

    /// The server's short name: it prefixes its tools and its gateway path.
    pub slug: String,

    /// Where the data plane reaches the server: a public `https://` URL.
    pub url: String,

    pub enabled: bool,

    /// When someone asked for the tools to be listed again (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_requested_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicySpec {
    pub id: String,

    pub name: String,

    pub cedar: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookSpec {
    /// Autharie's id for the webhook (a UUID). FerrisKey has its own.
    pub id: String,

    pub realm: String,

    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    pub endpoint: String,

    /// FerrisKey's dotted event names.
    #[serde(default)]
    pub events: Vec<String>,

    /// Names of the custom headers; the values are in the headers Secret under
    /// `<webhook id>.<header name>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header_names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum WebhookRequestKind {
    RotateSecret,
    RetryDelivery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookRequest {
    pub id: String,

    pub kind: WebhookRequestKind,

    pub webhook_id: String,

    /// Only for `retryDelivery`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IamConfig {
    /// The look of the login pages. Absent leaves the provider's theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branding: Option<Branding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Branding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<BrandingColors>,

    /// Corner radius in pixels, 0 to 24.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 24))]
    pub radius: Option<u8>,
}

/// Every colour is `#rrggbb`. Each one is optional and falls back to the theme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrandingColors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub primary: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub primary_text: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub links: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub page_background: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub widget_background: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub text: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub error: Option<String>,
}

/// What a recovery reads to come up.
///
/// Both fields are the *source's*, not this instance's. Barman files an
/// archive under the cluster that wrote it, inside the prefix that deployment
/// archives to -- so a recovery has to name somebody else's prefix and
/// somebody else's server, and the control plane computes both because it owns
/// the layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RestoreConfig {
    /// `s3://bucket/organisation/deployment`, the source's prefix.
    pub destination_path: String,

    /// The name barman filed the archive under, which is the source's own
    /// database cluster.
    ///
    /// Without it a recovery finds an empty prefix. CloudNativePG does not
    /// treat that as an error -- it bootstraps an empty cluster -- so the
    /// operator refuses a recovery whose source it cannot see rather than
    /// letting one come up looking restored.
    pub server_name: String,

    /// Which archive this came from, carried for the operator's logs and for
    /// anybody reading the cluster afterwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_id: Option<String>,
}

/// Where archives go.
///
/// The destination is computed by the control plane, which owns the layout,
/// and carried here rather than rebuilt. An operator deriving the path itself
/// would be a second implementation of the prefix rule, and the two would
/// disagree the first time either changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackupConfig {
    /// `s3://bucket/organisation/deployment`, with no trailing slash.
    ///
    /// Barman writes the data and the WALs into their own folders underneath.
    pub destination_path: String,

    /// Where the store answers. Absent means AWS, which is the only store that
    /// needs no endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,

    /// The secret holding the credentials the cluster writes with, in this
    /// instance's own namespace.
    ///
    /// A name rather than the credentials themselves. A CRD is readable by
    /// anything that can list the namespace, and a secret is at least the
    /// object Kubernetes knows to treat carefully.
    pub credentials_secret: String,

    /// What the store is asked to do with the archive once it has it.
    ///
    /// `AES256` or `aws:kms`; absent leaves it to the bucket's own policy. The
    /// store decrypts on read either way, which is the part worth knowing and
    /// the part this field cannot change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<String>,
}

/// Status of the IdentityInstance
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct IdentityInstanceStatus {
    /// Current phase of the instance
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,

    /// Is the instance ready to serve traffic
    #[serde(default)]
    pub ready: bool,

    /// Public endpoint URL (e.g., https://auth.acme.com)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Admin console URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_url: Option<String>,

    /// Conditions represent the latest available observations
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,

    /// Last time the status was updated
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,

    /// Error message if the instance is in Failed phase
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// What became of `spec.iam`. Absent until the operator has had something to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iam: Option<IamStatus>,

    /// What became of each of `spec.webhooks`, in the same order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webhooks: Vec<WebhookStatus>,

    /// The realms FerrisKey lists, so a console can offer them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub realms: Vec<String>,

    /// The requests already executed, so a restart does not run them twice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webhook_requests: Vec<WebhookRequestStatus>,

    /// What became of `spec.agents`. Absent until the operator has had something to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<AgentsStatus>,

    /// What became of each of `spec.smtp`, in the same order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub smtp: Vec<SmtpStatus>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub portal_themes: Vec<PortalThemeStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AgentsPhase {
    Applying,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AgentPolicyPhase {
    Applied,
    Invalid,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentsStatus {
    pub phase: AgentsPhase,

    #[serde(default)]
    pub gateway_client_ready: bool,

    #[serde(default, skip_serializing_if = "is_zero")]
    pub observed_revision: u64,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_url: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policies: Vec<AgentPolicyStatus>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clients: Vec<AgentClientStatus>,

    #[serde(default, skip_serializing_if = "is_false")]
    pub clients_truncated: bool,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials: Vec<AgentCredentialStatus>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_clients: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<AgentServerStatus>,
}

impl Default for AgentsStatus {
    fn default() -> Self {
        Self {
            phase: AgentsPhase::Applying,
            gateway_client_ready: false,
            observed_revision: 0,
            mcp_url: None,
            message: None,
            policies: Vec::new(),
            clients: Vec::new(),
            clients_truncated: false,
            credentials: Vec::new(),
            disabled_clients: Vec::new(),
            servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AgentServerPhase {
    Ready,
    Unreachable,
    Refused,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentClientStatus {
    pub client_id: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    pub source: String,

    pub kind: String,

    pub enabled: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentCredentialStatus {
    pub request_id: String,

    pub client_id: String,

    pub sealed: String,
}

impl std::fmt::Debug for AgentCredentialStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentCredentialStatus")
            .field("request_id", &self.request_id)
            .field("client_id", &self.client_id)
            .field("sealed", &"<sealed>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentServerTool {
    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default)]
    pub read_only: bool,

    #[serde(default)]
    pub destructive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentServerStatus {
    pub id: String,

    /// The slug the server had when it was last reconciled, so its audience
    /// client can still be found once the server is gone from the spec.
    pub slug: String,

    pub phase: AgentServerPhase,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AgentServerTool>,

    #[serde(default)]
    pub dropped: u32,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// When the last discovery started (RFC 3339). Absent until one has run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovered_at: Option<String>,

    /// The `discoveryRequestedAt` of the spec that this discovery answers, as
    /// written there. It is how the control plane tells a discovery of the
    /// server as it is from one of the server as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_requested_at: Option<String>,

    /// The MCP protocol version the server negotiated at discovery. Absent
    /// until one has succeeded; the gateway speaks 2026-07-28 to a server
    /// that is not known to speak anything older.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicyStatus {
    pub id: String,

    pub phase: AgentPolicyPhase,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum WebhookPhase {
    Pending,
    Active,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookStatus {
    pub id: String,

    /// FerrisKey's own id, once the webhook exists there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ferriskey_id: Option<String>,

    pub phase: WebhookPhase,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Counts the signing secrets issued; 0 until the first one is stored.
    #[serde(default)]
    pub secret_version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum WebhookRequestPhase {
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookRequestStatus {
    pub id: String,

    pub phase: WebhookRequestPhase,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum IamPhase {
    Pending,
    Applied,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IamStatus {
    pub phase: IamPhase,

    /// Why the settings are pending or were refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// The branding the provider is showing, as the operator wrote it. Absent
    /// when the provider shows its default theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum IdentityProvider {
    Keycloak,
    Ferriskey,
}

impl Display for IdentityProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keycloak => write!(f, "keycloak"),
            Self::Ferriskey => write!(f, "ferriskey"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FerriskeyConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webapp_url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,

    /// Serves the realms of many tenants, each on its own hostname: the
    /// webapp calls a relative `/api` and the API derives the webapp URL
    /// from the request.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pooled_cell: bool,
}

/// Whether and how an instance is reachable from outside the cluster.
///
/// Named `ingress` from when that is what it built. An instance is served
/// through an `HTTPRoute` attached to the data plane's Gateway now, so only
/// `enabled` is still read; the two below describe an edge the operator no
/// longer creates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngressConfig {
    /// Whether to expose this instance at all. Still read.
    #[serde(default = "default_ingress_enabled")]
    pub enabled: bool,

    /// IGNORED. The edge is the Gateway the data plane declares, and the
    /// operator does not choose between several.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,

    /// IGNORED. TLS terminates on the Gateway listener, which is configured
    /// once for the data plane rather than per instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<IngressTlsConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngressTlsConfig {
    #[serde(default = "default_ingress_tls_enabled")]
    pub enabled: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_issuer: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseConfig {
    #[serde(default)]
    pub mode: DatabaseMode,

    pub managed_cluster: ManagedClusterConfig,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<SharedClusterConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct SharedClusterConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_namespace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum DatabaseMode {
    #[default]
    ManagedCluster,
    Shared,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedClusterConfig {
    #[serde(default = "default_instances")]
    pub instances: i32,

    pub storage: ManagedClusterStorage,

    pub resources: ResourceRequirements,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedClusterStorage {
    pub size: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_class: Option<String>,
}

fn default_instances() -> i32 {
    1
}

fn default_replicas() -> i32 {
    1
}

fn default_ingress_enabled() -> bool {
    true
}

fn default_ingress_tls_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    Serving,
    Failed,
}

impl IdentityInstanceStatus {
    pub fn settled(&self) -> Option<Settled> {
        match self.phase {
            Some(Phase::Running) if self.ready => Some(Settled::Serving),
            Some(Phase::Failed) => Some(Settled::Failed),
            _ => None,
        }
    }
}

impl IdentityInstance {
    pub fn is_ready(&self) -> bool {
        self.status.as_ref().map(|s| s.ready).unwrap_or(false)
    }

    pub fn phase(&self) -> Option<Phase> {
        self.status.as_ref().and_then(|s| s.phase.clone())
    }

    pub fn endpoint(&self) -> Option<String> {
        self.status.as_ref().and_then(|s| s.endpoint.clone())
    }

    pub fn namespace(&self) -> Option<String> {
        self.metadata.namespace.clone()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{AgentPolicyPhase, AgentServerPhase, AgentsPhase, AgentsSpec};
    use crate::common::types::{Phase, ResourceList, ResourceRequirements};
    use crate::v1alpha::identity_instance::{
        DatabaseConfig, DatabaseMode, IdentityInstance, IdentityInstanceSpec,
        IdentityInstanceStatus, IdentityProvider, ManagedClusterConfig, ManagedClusterStorage,
        PortalThemePhase, PortalThemeSpec, SmtpEncryption, SmtpPhase, SmtpSpec, WebhookPhase,
        WebhookRequestKind, WebhookRequestPhase, WebhookSpec,
    };
    use kube::core::ObjectMeta;

    #[test]
    fn test_identity_instance_creation() {
        let spec = IdentityInstanceSpec {
            restore: None,
            iam: None,
            organisation_id: "org-123".to_string(),
            provider: IdentityProvider::Keycloak,
            version: "25.0.0".to_string(),
            hostname: "auth.acme.com".to_string(),
            replicas: 1,
            stopped: false,
            database: DatabaseConfig {
                mode: DatabaseMode::ManagedCluster,
                shared: None,
                managed_cluster: ManagedClusterConfig {
                    instances: 2,
                    storage: ManagedClusterStorage {
                        size: "20Gi".to_string(),
                        storage_class: Some("fast-ssd".to_string()),
                    },
                    resources: ResourceRequirements {
                        requests: Some(ResourceList {
                            cpu: Some("500m".to_string()),
                            memory: Some("1Gi".to_string()),
                        }),
                        limits: Some(ResourceList {
                            cpu: Some("2".to_string()),
                            memory: Some("4Gi".to_string()),
                        }),
                    },
                },
            },
            ferriskey: None,
            ingress: None,
            allowed_cidrs: None,
            backup: None,
            webhooks: Vec::new(),
            webhooks_revision: 0,
            webhook_requests: Vec::new(),
            agents: None,
            smtp: Vec::new(),
            portal_themes: Vec::new(),
        };

        assert_eq!(spec.provider, IdentityProvider::Keycloak);
        assert_eq!(spec.hostname, "auth.acme.com");
        assert_eq!(spec.database.managed_cluster.instances, 2);
    }

    fn spec_json() -> serde_json::Value {
        json!({
            "organisationId": "org-123",
            "provider": "ferriskey",
            "version": "1.0.0",
            "hostname": "auth.example.com",
            "database": {
                "managedCluster": {
                    "storage": { "size": "10Gi" },
                    "resources": {}
                }
            }
        })
    }

    #[test]
    fn a_spec_without_iam_round_trips_without_the_field() {
        let spec: IdentityInstanceSpec = serde_json::from_value(spec_json()).unwrap();

        assert_eq!(spec.iam, None);
        assert!(serde_json::to_value(&spec).unwrap().get("iam").is_none());
    }

    #[test]
    fn a_spec_with_iam_round_trips_in_camel_case() {
        let mut value = spec_json();
        value["iam"] = json!({
            "branding": {
                "colors": { "primaryText": "#ffffff", "pageBackground": "#101010" },
                "radius": 8
            }
        });

        let spec: IdentityInstanceSpec = serde_json::from_value(value.clone()).unwrap();
        let branding = spec.iam.as_ref().unwrap().branding.as_ref().unwrap();

        assert_eq!(branding.radius, Some(8));
        assert_eq!(
            branding.colors.as_ref().unwrap().page_background.as_deref(),
            Some("#101010")
        );
        assert_eq!(serde_json::to_value(&spec).unwrap()["iam"], value["iam"]);
    }

    #[test]
    fn a_spec_without_webhooks_stays_valid_and_serialises_without_them() {
        let spec: IdentityInstanceSpec = serde_json::from_value(spec_json()).unwrap();

        assert!(spec.webhooks.is_empty());
        assert_eq!(spec.webhooks_revision, 0);
        assert!(spec.webhook_requests.is_empty());
        let value = serde_json::to_value(&spec).unwrap();
        for key in ["webhooks", "webhooksRevision", "webhookRequests"] {
            assert!(value.get(key).is_none(), "{key}");
        }
    }

    #[test]
    fn webhooks_and_requests_round_trip_in_the_frozen_shape() {
        let mut value = spec_json();
        value["webhooks"] = json!([{
            "id": "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
            "realm": "master",
            "name": "Audit",
            "description": "to the SIEM",
            "endpoint": "https://example.com/hook",
            "events": ["user.created", "user.deleted"],
            "headerNames": ["Authorization"]
        }]);
        value["webhooksRevision"] = json!(7);
        value["webhookRequests"] = json!([
            { "id": "r1", "kind": "rotateSecret", "webhookId": "w1" },
            { "id": "r2", "kind": "retryDelivery", "webhookId": "w1", "deliveryId": "d1" }
        ]);

        let spec: IdentityInstanceSpec = serde_json::from_value(value.clone()).unwrap();

        assert_eq!(spec.webhooks[0].header_names, vec!["Authorization"]);
        assert_eq!(spec.webhooks_revision, 7);
        assert_eq!(
            spec.webhook_requests[0].kind,
            WebhookRequestKind::RotateSecret
        );
        assert_eq!(spec.webhook_requests[1].delivery_id.as_deref(), Some("d1"));
        let back = serde_json::to_value(&spec).unwrap();
        assert_eq!(back["webhooks"], value["webhooks"]);
        assert_eq!(back["webhooksRevision"], value["webhooksRevision"]);
        assert_eq!(back["webhookRequests"], value["webhookRequests"]);
    }

    #[test]
    fn a_webhook_without_header_names_or_description_is_valid() {
        let webhook: WebhookSpec = serde_json::from_value(json!({
            "id": "w", "realm": "master", "name": "n",
            "endpoint": "https://example.com", "events": []
        }))
        .unwrap();

        assert!(webhook.header_names.is_empty());
        assert_eq!(webhook.description, None);
    }

    #[test]
    fn webhook_status_round_trips_in_the_frozen_shape() {
        let status: IdentityInstanceStatus = serde_json::from_value(json!({
            "webhooks": [
                { "id": "w1", "ferriskeyId": "f1", "phase": "Active", "secretVersion": 2 },
                { "id": "w2", "phase": "Failed", "message": "no", "secretVersion": 0 }
            ],
            "realms": ["master", "acme"],
            "webhookRequests": [
                { "id": "r1", "phase": "Done" },
                { "id": "r2", "phase": "Failed", "message": "gone" }
            ]
        }))
        .unwrap();

        assert_eq!(status.webhooks[0].phase, WebhookPhase::Active);
        assert_eq!(status.webhooks[0].ferriskey_id.as_deref(), Some("f1"));
        assert_eq!(status.webhooks[1].ferriskey_id, None);
        assert_eq!(status.realms, vec!["master", "acme"]);
        assert_eq!(
            status.webhook_requests[1].phase,
            WebhookRequestPhase::Failed
        );
        let back = serde_json::to_value(&status).unwrap();
        assert_eq!(back["webhooks"][0]["secretVersion"], json!(2));
        assert!(back["webhooks"][1].get("ferriskeyId").is_none());
    }

    #[test]
    fn a_status_without_webhooks_stays_valid() {
        let status: IdentityInstanceStatus = serde_json::from_value(json!({})).unwrap();

        assert!(status.webhooks.is_empty());
        assert!(status.realms.is_empty());
        assert!(status.webhook_requests.is_empty());
    }

    #[test]
    fn a_spec_without_agents_stays_valid_and_serialises_without_them() {
        let spec: IdentityInstanceSpec = serde_json::from_value(spec_json()).unwrap();

        assert_eq!(spec.agents, None);
        assert!(serde_json::to_value(&spec).unwrap().get("agents").is_none());
    }

    #[test]
    fn agents_round_trip_in_the_frozen_shape() {
        let mut value = spec_json();
        value["agents"] = json!({
            "enabled": true,
            "realm": "master",
            "policies": [
                { "id": "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11", "name": "Read only", "cedar": "permit (principal, action, resource);" }
            ],
            "policiesRevision": 4
        });

        let spec: IdentityInstanceSpec = serde_json::from_value(value.clone()).unwrap();

        let agents = spec.agents.as_ref().unwrap();
        assert!(agents.enabled);
        assert_eq!(agents.realm, "master");
        assert_eq!(agents.policies[0].name, "Read only");
        assert_eq!(agents.policies_revision, 4);
        assert_eq!(
            serde_json::to_value(&spec).unwrap()["agents"],
            value["agents"]
        );
    }

    #[test]
    fn the_registry_fields_round_trip_in_the_frozen_shape() {
        let value = json!({
            "enabled": true,
            "realm": "acme",
            "access": { "approvalRequired": true, "approved": ["a"], "blocked": ["b"] },
            "autonomous": [
                { "clientId": "agent-x-abc123", "name": "X", "requestId": "r1",
                  "sealingKey": { "kty": "RSA", "n": "AA", "e": "AQAB", "alg": "RSA-OAEP-256" } },
                { "clientId": "agent-y-abc123", "name": "Y", "rotateRequestId": "r2",
                  "sealingKey": { "kty": "RSA", "n": "AA", "e": "AQAB", "alg": "RSA-OAEP-256" } },
                { "clientId": "agent-z-abc123", "name": "Z" }
            ],
            "deletedAgents": ["old"]
        });

        let agents: AgentsSpec = serde_json::from_value(value.clone()).unwrap();

        assert!(agents.access.approval_required);
        assert_eq!(agents.autonomous[0].request_id.as_deref(), Some("r1"));
        assert_eq!(
            agents.autonomous[1].rotate_request_id.as_deref(),
            Some("r2")
        );
        assert_eq!(agents.autonomous[2].sealing_key, None);
        assert_eq!(agents.deleted_agents, vec!["old".to_string()]);
        assert_eq!(serde_json::to_value(&agents).unwrap(), value);
    }

    #[test]
    fn an_open_access_and_no_registry_work_serialise_to_nothing() {
        let agents: AgentsSpec =
            serde_json::from_value(json!({ "enabled": true, "realm": "master" })).unwrap();

        assert_eq!(
            serde_json::to_value(&agents).unwrap(),
            json!({ "enabled": true, "realm": "master" })
        );
    }

    #[test]
    fn the_registry_status_round_trips_and_a_credential_never_prints() {
        let value = json!({
            "phase": "Ready",
            "gatewayClientReady": true,
            "clients": [
                { "clientId": "dcr", "source": "dynamic", "kind": "delegated", "enabled": true,
                  "createdAt": "2026-10-23T10:00:00Z" }
            ],
            "clientsTruncated": true,
            "credentials": [{ "requestId": "r1", "clientId": "agent-x", "sealed": "TOP-SECRET" }],
            "disabledClients": ["b"]
        });

        let status: super::AgentsStatus = serde_json::from_value(value.clone()).unwrap();

        assert_eq!(status.clients[0].source, "dynamic");
        assert!(status.clients_truncated);
        assert_eq!(status.disabled_clients, vec!["b".to_string()]);
        assert_eq!(serde_json::to_value(&status).unwrap(), value);
        assert!(!format!("{status:?}").contains("TOP-SECRET"));
    }

    #[test]
    fn disabled_agents_without_policies_serialise_minimally() {
        let agents: AgentsSpec =
            serde_json::from_value(json!({ "enabled": false, "realm": "master" })).unwrap();

        assert!(agents.policies.is_empty());
        assert_eq!(agents.policies_revision, 0);
        assert_eq!(
            serde_json::to_value(&agents).unwrap(),
            json!({ "enabled": false, "realm": "master" })
        );
    }

    #[test]
    fn agents_status_round_trips_in_the_frozen_shape() {
        let status: IdentityInstanceStatus = serde_json::from_value(json!({
            "agents": {
                "phase": "Ready",
                "gatewayClientReady": true,
                "observedRevision": 4,
                "mcpUrl": "https://auth.example.com/mcp",
                "policies": [
                    { "id": "p1", "phase": "Applied" },
                    { "id": "p2", "phase": "Invalid", "message": "unexpected token" },
                    { "id": "p3", "phase": "Failed", "message": "unreachable" }
                ]
            }
        }))
        .unwrap();

        let agents = status.agents.as_ref().unwrap();
        assert_eq!(agents.phase, AgentsPhase::Ready);
        assert!(agents.gateway_client_ready);
        assert_eq!(agents.observed_revision, 4);
        assert_eq!(agents.policies[1].phase, AgentPolicyPhase::Invalid);
        assert_eq!(agents.policies[2].phase, AgentPolicyPhase::Failed);
        let back = serde_json::to_value(&status).unwrap();
        assert_eq!(back["agents"]["policies"][1]["message"], "unexpected token");
        assert!(back["agents"].get("message").is_none());
    }

    #[test]
    fn customer_servers_round_trip_in_the_frozen_spec_shape() {
        let mut value = spec_json();
        value["agents"] = json!({
            "enabled": true,
            "realm": "master",
            "servers": [
                {
                    "id": "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
                    "slug": "crm",
                    "url": "https://tools.example.com/mcp",
                    "enabled": true,
                    "discoveryRequestedAt": "2026-10-23T10:00:00Z"
                },
                {
                    "id": "7f1c2d3e-0000-4000-8000-00000000000b",
                    "slug": "erp",
                    "url": "https://erp.example.com/mcp",
                    "enabled": false
                }
            ]
        });

        let spec: IdentityInstanceSpec = serde_json::from_value(value.clone()).unwrap();

        let servers = &spec.agents.as_ref().unwrap().servers;
        assert_eq!(servers[0].slug, "crm");
        assert_eq!(
            servers[0].discovery_requested_at.as_deref(),
            Some("2026-10-23T10:00:00Z")
        );
        assert_eq!(servers[1].discovery_requested_at, None);
        assert_eq!(
            serde_json::to_value(&spec).unwrap()["agents"],
            value["agents"]
        );
    }

    #[test]
    fn agents_without_servers_serialise_without_the_field() {
        let agents: AgentsSpec =
            serde_json::from_value(json!({ "enabled": true, "realm": "master" })).unwrap();

        assert!(agents.servers.is_empty());
        assert!(
            serde_json::to_value(&agents)
                .unwrap()
                .get("servers")
                .is_none()
        );
    }

    #[test]
    fn server_status_round_trips_with_its_tools_and_hints() {
        let status: IdentityInstanceStatus = serde_json::from_value(json!({
            "agents": {
                "phase": "Ready",
                "observedRevision": 4,
                "servers": [{
                    "id": "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
                    "slug": "crm",
                    "phase": "Ready",
                    "tools": [
                        { "name": "list_invoices", "readOnly": true },
                        { "name": "delete_invoice", "description": "removes", "destructive": true }
                    ],
                    "dropped": 2,
                    "discoveredAt": "2026-10-23T10:00:01Z",
                    "discoveryRequestedAt": "2026-10-23T10:00:00.000Z"
                }, {
                    "id": "7f1c2d3e-0000-4000-8000-00000000000b",
                    "slug": "erp",
                    "phase": "Refused",
                    "message": "not a public address"
                }]
            }
        }))
        .unwrap();

        let servers = &status.agents.as_ref().unwrap().servers;
        assert_eq!(servers[0].phase, AgentServerPhase::Ready);
        assert!(servers[0].tools[0].read_only);
        assert!(!servers[0].tools[0].destructive);
        assert!(servers[0].tools[1].destructive);
        assert_eq!(servers[0].dropped, 2);
        assert_eq!(
            servers[0].discovery_requested_at.as_deref(),
            Some("2026-10-23T10:00:00.000Z")
        );
        assert_eq!(servers[1].phase, AgentServerPhase::Refused);
        assert!(servers[1].tools.is_empty());
        let back = serde_json::to_value(&status).unwrap();
        assert_eq!(back["agents"]["servers"][1].get("discoveredAt"), None);
        assert_eq!(
            back["agents"]["servers"][0]["tools"][1]["destructive"],
            true
        );
    }

    #[test]
    fn a_status_without_agents_stays_valid() {
        let status: IdentityInstanceStatus = serde_json::from_value(json!({})).unwrap();

        assert_eq!(status.agents, None);
    }

    #[test]
    fn the_schema_names_the_agent_fields() {
        use kube::CustomResourceExt;
        let schema = serde_json::to_value(IdentityInstance::crd()).unwrap();
        let text = schema.to_string();

        for key in [
            "policiesRevision",
            "gatewayClientReady",
            "observedRevision",
            "mcpUrl",
            "Applying",
            "Invalid",
        ] {
            assert!(text.contains(key), "{key}");
        }
    }

    #[test]
    fn the_schema_describes_branding_with_its_constraints() {
        use kube::CustomResourceExt;

        let crd = serde_json::to_value(IdentityInstance::crd()).unwrap();
        let spec = &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]["spec"];
        let branding = &spec["properties"]["iam"]["properties"]["branding"]["properties"];
        let colors = &branding["colors"]["properties"];

        for key in [
            "primary",
            "primaryText",
            "links",
            "pageBackground",
            "widgetBackground",
            "text",
            "error",
        ] {
            assert_eq!(colors[key]["pattern"], json!("^#[0-9a-fA-F]{6}$"), "{key}");
        }
        assert_eq!(branding["radius"]["minimum"].as_f64(), Some(0.0));
        assert_eq!(branding["radius"]["maximum"].as_f64(), Some(24.0));
    }

    #[test]
    fn a_spec_without_smtp_stays_valid_and_serialises_without_it() {
        let spec: IdentityInstanceSpec = serde_json::from_value(spec_json()).unwrap();

        assert!(spec.smtp.is_empty());
        assert!(serde_json::to_value(&spec).unwrap().get("smtp").is_none());
    }

    #[test]
    fn smtp_entries_round_trip_in_the_frozen_shape() {
        let mut value = spec_json();
        value["smtp"] = json!([
            {
                "realm": "acme",
                "revision": 3,
                "host": "smtp.example.com",
                "port": 587,
                "encryption": "starttls",
                "username": "mailer",
                "fromEmail": "no-reply@example.com",
                "fromName": "Acme"
            },
            { "realm": "master", "revision": 9, "remove": true }
        ]);

        let spec: IdentityInstanceSpec = serde_json::from_value(value.clone()).unwrap();

        assert_eq!(spec.smtp[0].port, Some(587));
        assert_eq!(spec.smtp[0].encryption, Some(SmtpEncryption::Starttls));
        assert_eq!(
            spec.smtp[0].from_email.as_deref(),
            Some("no-reply@example.com")
        );
        assert!(!spec.smtp[0].remove);
        assert!(spec.smtp[1].remove);
        assert_eq!(spec.smtp[1].host, None);
        assert_eq!(serde_json::to_value(&spec).unwrap()["smtp"], value["smtp"]);
    }

    #[test]
    fn encryption_is_spelled_in_lowercase() {
        for (encryption, spelled) in [
            (SmtpEncryption::Tls, "tls"),
            (SmtpEncryption::Starttls, "starttls"),
            (SmtpEncryption::None, "none"),
        ] {
            assert_eq!(serde_json::to_value(encryption).unwrap(), json!(spelled));
            assert_eq!(
                serde_json::from_value::<SmtpEncryption>(json!(spelled)).unwrap(),
                encryption
            );
        }
        assert!(serde_json::from_value::<SmtpEncryption>(json!("TLS")).is_err());
    }

    #[test]
    fn a_tombstone_serialises_without_the_settings() {
        let entry = SmtpSpec {
            realm: "acme".to_string(),
            revision: 4,
            remove: true,
            host: None,
            port: None,
            encryption: None,
            username: None,
            from_email: None,
            from_name: None,
        };

        assert_eq!(
            serde_json::to_value(&entry).unwrap(),
            json!({ "realm": "acme", "revision": 4, "remove": true })
        );
    }

    #[test]
    fn smtp_status_round_trips_in_the_frozen_shape() {
        let status: IdentityInstanceStatus = serde_json::from_value(json!({
            "smtp": [
                { "realm": "acme", "revision": 3, "phase": "Applied" },
                { "realm": "beta", "revision": 1, "phase": "Failed", "message": "no password" },
                { "realm": "gamma", "revision": 2, "phase": "Pending", "message": "unreachable" }
            ]
        }))
        .unwrap();

        assert_eq!(status.smtp[0].phase, SmtpPhase::Applied);
        assert_eq!(status.smtp[1].phase, SmtpPhase::Failed);
        assert_eq!(status.smtp[2].phase, SmtpPhase::Pending);
        assert_eq!(status.smtp[1].message.as_deref(), Some("no password"));
        let back = serde_json::to_value(&status).unwrap();
        assert!(back["smtp"][0].get("message").is_none());
        assert_eq!(back["smtp"][2]["revision"], json!(2));
        assert!(
            serde_json::to_value(IdentityInstanceStatus::default())
                .unwrap()
                .get("smtp")
                .is_none()
        );
    }

    #[test]
    fn a_spec_without_portal_themes_stays_valid_and_serialises_without_them() {
        let spec: IdentityInstanceSpec = serde_json::from_value(spec_json()).unwrap();

        assert!(spec.portal_themes.is_empty());
        assert!(
            serde_json::to_value(&spec)
                .unwrap()
                .get("portalThemes")
                .is_none()
        );
    }

    #[test]
    fn portal_themes_round_trip_with_their_config_untouched() {
        let mut value = spec_json();
        value["portalThemes"] = json!([
            {
                "realm": "acme",
                "revision": 3,
                "config": {
                    "colors": { "primaryButton": "#112233" },
                    "fonts": { "baseSize": 16 },
                    "somethingNew": [1, 2, { "deep": true }]
                }
            },
            { "realm": "master", "revision": 9, "config": {} }
        ]);

        let spec: IdentityInstanceSpec = serde_json::from_value(value.clone()).unwrap();

        assert_eq!(spec.portal_themes[0].realm, "acme");
        assert_eq!(spec.portal_themes[0].revision, 3);
        assert_eq!(
            spec.portal_themes[0].config["colors"]["primaryButton"],
            "#112233"
        );
        assert_eq!(spec.portal_themes[1].config, json!({}));
        assert_eq!(
            serde_json::to_value(&spec).unwrap()["portalThemes"],
            value["portalThemes"]
        );
    }

    #[test]
    fn a_portal_theme_needs_its_realm_revision_and_config() {
        for entry in [
            json!({ "revision": 1, "config": {} }),
            json!({ "realm": "acme", "config": {} }),
            json!({ "realm": "acme", "revision": 1 }),
        ] {
            assert!(serde_json::from_value::<PortalThemeSpec>(entry).is_err());
        }
    }

    #[test]
    fn portal_theme_status_round_trips_in_the_frozen_shape() {
        let status: IdentityInstanceStatus = serde_json::from_value(json!({
            "portalThemes": [
                { "realm": "acme", "revision": 3, "phase": "Applied" },
                { "realm": "beta", "revision": 1, "phase": "Failed", "message": "bad colour" },
                { "realm": "gamma", "revision": 2, "phase": "Pending", "message": "unreachable" }
            ]
        }))
        .unwrap();

        assert_eq!(status.portal_themes[0].phase, PortalThemePhase::Applied);
        assert_eq!(status.portal_themes[1].phase, PortalThemePhase::Failed);
        assert_eq!(status.portal_themes[2].phase, PortalThemePhase::Pending);
        assert_eq!(
            status.portal_themes[1].message.as_deref(),
            Some("bad colour")
        );
        let back = serde_json::to_value(&status).unwrap();
        assert!(back["portalThemes"][0].get("message").is_none());
        assert_eq!(back["portalThemes"][2]["revision"], json!(2));
        assert!(
            serde_json::to_value(IdentityInstanceStatus::default())
                .unwrap()
                .get("portalThemes")
                .is_none()
        );
    }

    #[test]
    fn the_schema_keeps_the_theme_config_free_form() {
        use kube::CustomResourceExt;

        let crd = serde_json::to_value(IdentityInstance::crd()).unwrap();
        let schema = &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"];
        let entry = &schema["spec"]["properties"]["portalThemes"]["items"];
        let config = &entry["properties"]["config"];

        assert_eq!(entry["required"], json!(["config", "realm", "revision"]));
        assert_eq!(config["type"], json!("object"));
        assert_eq!(config["x-kubernetes-preserve-unknown-fields"], json!(true));
        assert_eq!(entry["properties"]["revision"]["type"], json!("integer"));

        let reported = &schema["status"]["properties"]["portalThemes"]["items"];
        assert_eq!(reported["required"], json!(["phase", "realm", "revision"]));
        assert_eq!(
            reported["properties"]["phase"]["enum"],
            json!(["Pending", "Applied", "Failed"])
        );
    }

    #[test]
    fn the_schema_describes_smtp_with_its_constraints() {
        use kube::CustomResourceExt;

        let crd = serde_json::to_value(IdentityInstance::crd()).unwrap();
        let schema = &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"];
        let entry = &schema["spec"]["properties"]["smtp"]["items"];
        let properties = &entry["properties"];

        assert_eq!(entry["required"], json!(["realm", "revision"]));
        assert_eq!(properties["port"]["minimum"].as_f64(), Some(1.0));
        assert_eq!(properties["port"]["maximum"].as_f64(), Some(65535.0));
        let encryptions: Vec<_> = properties["encryption"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|value| value.as_str())
            .collect();
        assert_eq!(encryptions, ["tls", "starttls", "none"]);
        assert_eq!(properties["revision"]["type"], json!("integer"));
        for key in ["remove", "host", "username", "fromEmail", "fromName"] {
            assert!(properties.get(key).is_some(), "{key}");
        }

        let reported = &schema["status"]["properties"]["smtp"]["items"];
        assert_eq!(reported["required"], json!(["phase", "realm", "revision"]));
        assert_eq!(
            reported["properties"]["phase"]["enum"],
            json!(["Pending", "Applied", "Failed"])
        );
    }

    #[test]
    fn replicas_default_to_one_when_absent_and_round_trip() {
        let spec: IdentityInstanceSpec = serde_json::from_value(spec_json()).unwrap();
        assert_eq!(spec.replicas, 1);

        let mut value = spec_json();
        value["replicas"] = json!(3);
        let spec: IdentityInstanceSpec = serde_json::from_value(value).unwrap();
        assert_eq!(spec.replicas, 3);
        assert_eq!(serde_json::to_value(&spec).unwrap()["replicas"], json!(3));
    }

    #[test]
    fn stopped_defaults_to_false_when_absent_and_is_omitted_while_false() {
        let spec: IdentityInstanceSpec = serde_json::from_value(spec_json()).unwrap();
        assert!(!spec.stopped);
        assert!(
            serde_json::to_value(&spec)
                .unwrap()
                .get("stopped")
                .is_none()
        );

        let mut value = spec_json();
        value["stopped"] = json!(true);
        let spec: IdentityInstanceSpec = serde_json::from_value(value).unwrap();
        assert!(spec.stopped);
        assert_eq!(serde_json::to_value(&spec).unwrap()["stopped"], json!(true));
    }

    #[test]
    fn the_schema_bounds_replicas_between_one_and_five() {
        use kube::CustomResourceExt;

        let crd = serde_json::to_value(IdentityInstance::crd()).unwrap();
        let spec = &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]["spec"];
        let replicas = &spec["properties"]["replicas"];

        assert_eq!(replicas["minimum"].as_f64(), Some(1.0));
        assert_eq!(replicas["maximum"].as_f64(), Some(5.0));
        assert_eq!(replicas["default"], json!(1));
    }

    #[test]
    fn test_provider_display() {
        assert_eq!(IdentityProvider::Keycloak.to_string(), "keycloak");
        assert_eq!(IdentityProvider::Ferriskey.to_string(), "ferriskey");
    }

    #[test]
    fn test_default_instances() {
        let config = DatabaseConfig {
            mode: DatabaseMode::ManagedCluster,
            shared: None,
            managed_cluster: ManagedClusterConfig {
                instances: super::default_instances(),
                storage: ManagedClusterStorage {
                    size: "10Gi".to_string(),
                    storage_class: None,
                },
                resources: ResourceRequirements {
                    requests: None,
                    limits: None,
                },
            },
        };

        assert_eq!(config.managed_cluster.instances, 1);
    }

    #[test]
    fn test_database_config_deserializes_managed_cluster() {
        let value = json!({
            "mode": "managedCluster",
            "managedCluster": {
                "instances": 3,
                "storage": {
                    "size": "50Gi",
                    "storageClass": "premium-rwo"
                },
                "resources": {
                    "requests": { "cpu": "500m", "memory": "1Gi" },
                    "limits": { "cpu": "2", "memory": "4Gi" }
                }
            }
        });

        let config: DatabaseConfig = serde_json::from_value(value).unwrap();
        assert_eq!(config.mode, DatabaseMode::ManagedCluster);
        assert_eq!(config.managed_cluster.instances, 3);
        assert_eq!(config.managed_cluster.storage.size, "50Gi");
        assert_eq!(config.shared, None);
    }

    #[test]
    fn test_database_mode_absent_is_managed_cluster() {
        let value = json!({
            "managedCluster": { "storage": { "size": "1Gi" }, "resources": {} }
        });

        let config: DatabaseConfig = serde_json::from_value(value).unwrap();
        assert_eq!(config.mode, DatabaseMode::ManagedCluster);
    }

    #[test]
    fn test_database_config_deserializes_shared() {
        let value = json!({
            "mode": "shared",
            "managedCluster": { "storage": { "size": "1Gi" }, "resources": {} },
            "shared": { "clusterName": "free-db", "clusterNamespace": "autharie-system" }
        });

        let config: DatabaseConfig = serde_json::from_value(value).unwrap();
        assert_eq!(config.mode, DatabaseMode::Shared);
        let shared = config.shared.unwrap();
        assert_eq!(shared.cluster_name.as_deref(), Some("free-db"));
        assert_eq!(shared.cluster_namespace.as_deref(), Some("autharie-system"));
    }

    #[test]
    fn test_shared_without_coordinates_leaves_them_to_the_operator() {
        let value = json!({
            "mode": "shared",
            "managedCluster": { "storage": { "size": "1Gi" }, "resources": {} }
        });

        let config: DatabaseConfig = serde_json::from_value(value).unwrap();
        assert_eq!(config.mode, DatabaseMode::Shared);
        assert_eq!(config.shared, None);
    }

    #[test]
    fn test_identity_instance_status_helpers() {
        let instance = IdentityInstance {
            metadata: ObjectMeta {
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: IdentityInstanceSpec {
                webhooks: Vec::new(),
                webhooks_revision: 0,
                webhook_requests: Vec::new(),
                agents: None,
                smtp: Vec::new(),
                portal_themes: Vec::new(),
                restore: None,
                iam: None,
                organisation_id: "org-123".to_string(),
                provider: IdentityProvider::Keycloak,
                version: "25.0.0".to_string(),
                hostname: "auth.acme.com".to_string(),
                replicas: 1,
                stopped: false,
                database: DatabaseConfig {
                    mode: DatabaseMode::ManagedCluster,
                    shared: None,
                    managed_cluster: ManagedClusterConfig {
                        instances: 1,
                        storage: ManagedClusterStorage {
                            size: "10Gi".to_string(),
                            storage_class: None,
                        },
                        resources: ResourceRequirements {
                            requests: None,
                            limits: None,
                        },
                    },
                },
                ferriskey: None,
                ingress: None,
                allowed_cidrs: None,
                backup: None,
            },
            status: Some(super::IdentityInstanceStatus {
                webhooks: Vec::new(),
                realms: Vec::new(),
                webhook_requests: Vec::new(),
                agents: None,
                smtp: Vec::new(),
                portal_themes: Vec::new(),
                phase: Some(Phase::Running),
                ready: true,
                endpoint: Some("https://auth.acme.com".to_string()),
                admin_url: None,
                conditions: vec![],
                last_updated: None,
                error: None,
                iam: None,
            }),
        };

        assert!(instance.is_ready());
        assert_eq!(instance.phase(), Some(Phase::Running));
        assert_eq!(
            instance.endpoint(),
            Some("https://auth.acme.com".to_string())
        );
        assert_eq!(instance.namespace(), Some("default".to_string()));
    }

    #[test]
    fn test_identity_instance_helpers_with_missing_status() {
        let instance = IdentityInstance {
            metadata: ObjectMeta::default(),
            spec: IdentityInstanceSpec {
                webhooks: Vec::new(),
                webhooks_revision: 0,
                webhook_requests: Vec::new(),
                agents: None,
                smtp: Vec::new(),
                portal_themes: Vec::new(),
                restore: None,
                iam: None,
                organisation_id: "org-123".to_string(),
                provider: IdentityProvider::Ferriskey,
                version: "1.0.0".to_string(),
                hostname: "auth.example.com".to_string(),
                replicas: 1,
                stopped: false,
                database: DatabaseConfig {
                    mode: DatabaseMode::ManagedCluster,
                    shared: None,
                    managed_cluster: ManagedClusterConfig {
                        instances: 1,
                        storage: ManagedClusterStorage {
                            size: "10Gi".to_string(),
                            storage_class: None,
                        },
                        resources: ResourceRequirements {
                            requests: None,
                            limits: None,
                        },
                    },
                },
                ferriskey: None,
                ingress: None,
                allowed_cidrs: None,
                backup: None,
            },
            status: None,
        };

        assert!(!instance.is_ready());
        assert_eq!(instance.phase(), None);
        assert_eq!(instance.endpoint(), None);
        assert_eq!(instance.namespace(), None);
    }

    #[test]
    fn test_identity_instance_status_serialization_skips_empty_fields() {
        let status = super::IdentityInstanceStatus::default();
        let value = serde_json::to_value(status).unwrap();

        assert!(value.get("phase").is_none());
        assert_eq!(value["ready"], json!(false));
        assert!(value.get("endpoint").is_none());
        assert!(value.get("adminUrl").is_none());
        assert!(value.get("conditions").is_none());
        assert!(value.get("lastUpdated").is_none());
        assert!(value.get("error").is_none());
    }

    #[test]
    fn test_database_mode_defaults_to_managed_cluster() {
        let value = json!({
            "managedCluster": {
                "instances": 1,
                "storage": { "size": "10Gi" },
                "resources": {}
            }
        });

        let config: DatabaseConfig = serde_json::from_value(value).unwrap();
        assert_eq!(config.mode, DatabaseMode::ManagedCluster);
    }

    #[test]
    fn test_iam_status_is_camel_case_and_skipped_when_absent() {
        use crate::v1alpha::identity_instance::{IamPhase, IamStatus, IdentityInstanceStatus};

        let empty = serde_json::to_value(IdentityInstanceStatus::default()).unwrap();
        assert!(empty.get("iam").is_none());

        let status = IdentityInstanceStatus {
            iam: Some(IamStatus {
                phase: IamPhase::Applied,
                message: None,
                applied: Some("{}".to_string()),
                observed_at: Some("2026-01-01T00:00:00Z".to_string()),
            }),
            ..Default::default()
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(
            value["iam"],
            json!({"phase": "Applied", "applied": "{}", "observedAt": "2026-01-01T00:00:00Z"})
        );
    }

    fn status_at(phase: Phase, ready: bool) -> super::IdentityInstanceStatus {
        super::IdentityInstanceStatus {
            phase: Some(phase),
            ready,
            ..Default::default()
        }
    }

    #[test]
    fn a_running_and_ready_instance_is_serving() {
        assert_eq!(
            status_at(Phase::Running, true).settled(),
            Some(super::Settled::Serving)
        );
    }

    #[test]
    fn running_without_ready_is_not_settled() {
        assert_eq!(status_at(Phase::Running, false).settled(), None);
    }

    #[test]
    fn a_failed_instance_is_settled_as_failed_however_ready_reads() {
        assert_eq!(
            status_at(Phase::Failed, false).settled(),
            Some(super::Settled::Failed)
        );
    }

    #[test]
    fn phases_on_the_way_are_not_settled() {
        for phase in [
            Phase::Pending,
            Phase::DatabaseProvisioning,
            Phase::Deploying,
            Phase::Updating,
            Phase::Upgrading,
            Phase::Deleting,
        ] {
            assert_eq!(status_at(phase.clone(), true).settled(), None, "{phase:?}");
        }
    }
}
