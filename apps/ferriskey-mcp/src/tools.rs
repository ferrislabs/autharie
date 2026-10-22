use http::request::Parts;
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::Extension, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::{
    api::{ApiClient, Fault},
    token_realm,
    upstream::Instance,
};

pub const TOOL_NAMES: [&str; 24] = [
    "list_realms",
    "list_users",
    "get_user",
    "get_user_roles",
    "list_roles",
    "list_clients",
    "get_client",
    "list_organisations",
    "list_groups",
    "list_group_members",
    "list_webhooks",
    "list_security_events",
    "list_user_sessions",
    "create_user",
    "update_user",
    "disable_user",
    "enable_user",
    "require_user_actions",
    "unlock_user",
    "assign_role",
    "unassign_role",
    "add_group_member",
    "remove_group_member",
    "revoke_user_session",
];

const MAX_LIMIT: u32 = 100;
const SECRET_KEYS: [&str; 2] = ["secret", "client_secret"];

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Scope {
    #[schemars(
        description = "Realm to act on. Defaults to the realm the access token was issued for."
    )]
    pub realm: Option<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Paging {
    #[schemars(description = "Page number, from 1. Default 1.")]
    pub page: Option<u32>,
    #[schemars(description = "Items per page, 1 to 100. Default 20.")]
    pub limit: Option<u32>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ListArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[serde(flatten)]
    pub paging: Paging,
    #[schemars(description = "Free text matched against the item's names.")]
    pub search: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ListUsersArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[serde(flatten)]
    pub paging: Paging,
    #[schemars(
        description = "Matched case-insensitively anywhere in username, email, first name or last name."
    )]
    pub search: Option<String>,
    #[schemars(description = "Username contains this text.")]
    pub username: Option<String>,
    #[schemars(description = "Email contains this text.")]
    pub email: Option<String>,
    #[schemars(description = "Keep only enabled (true) or disabled (false) users.")]
    pub enabled: Option<bool>,
    #[schemars(description = "UUID of a role; keep only the users holding it.")]
    pub role_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UserArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[schemars(description = "UUID of the user; see list_users.")]
    pub user_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UserPageArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[serde(flatten)]
    pub paging: Paging,
    #[schemars(description = "UUID of the user; see list_users.")]
    pub user_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ListClientsArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[serde(flatten)]
    pub paging: Paging,
    #[schemars(description = "Matched case-insensitively anywhere in the name or the client_id.")]
    pub search: Option<String>,
    #[schemars(description = "Keep only enabled (true) or disabled (false) clients.")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ClientArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[schemars(description = "UUID of the client (its id, not its client_id); see list_clients.")]
    pub client_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ListGroupsArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[serde(flatten)]
    pub paging: Paging,
    #[schemars(description = "UUID of the organisation; see list_organisations.")]
    pub organisation_id: String,
    #[schemars(description = "Matched case-insensitively against the group name.")]
    pub search: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ListGroupMembersArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[serde(flatten)]
    pub paging: Paging,
    #[schemars(description = "UUID of the organisation; see list_organisations.")]
    pub organisation_id: String,
    #[schemars(description = "UUID of the group; see list_groups.")]
    pub group_id: String,
    #[schemars(description = "Matched case-insensitively against username or email.")]
    pub search: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct GroupMemberArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[schemars(description = "UUID of the organisation; see list_organisations.")]
    pub organisation_id: String,
    #[schemars(description = "UUID of the group; see list_groups.")]
    pub group_id: String,
    #[schemars(description = "UUID of the user; see list_users.")]
    pub user_id: String,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "lowercase")]
pub enum EventStatus {
    Success,
    Failure,
}

impl EventStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SecurityEventsArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[serde(flatten)]
    pub paging: Paging,
    #[schemars(
        description = "Event types to keep, for example login_failure, session_revoked, role_assigned, user_created."
    )]
    pub event_types: Option<Vec<String>>,
    #[schemars(description = "success or failure.")]
    pub status: Option<EventStatus>,
    #[schemars(description = "UUID of the user or client that acted.")]
    pub actor_id: Option<String>,
    #[schemars(description = "Kind of the target, for example client or user.")]
    pub target_type: Option<String>,
    #[schemars(description = "Earliest event time, RFC 3339 with an offset.")]
    pub from_timestamp: Option<String>,
    #[schemars(description = "Latest event time, RFC 3339 with an offset.")]
    pub to_timestamp: Option<String>,
    #[schemars(description = "IP address contains this text.")]
    pub ip_address: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct CreateUserArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[schemars(description = "Username of the new user; must be unique in the realm.")]
    pub username: String,
    pub firstname: Option<String>,
    pub lastname: Option<String>,
    pub email: Option<String>,
    #[schemars(description = "Whether the email counts as verified. Default false.")]
    pub email_verified: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UpdateUserArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[schemars(description = "UUID of the user; see list_users.")]
    pub user_id: String,
    pub firstname: Option<String>,
    pub lastname: Option<String>,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum RequiredAction {
    ConfigureOtp,
    VerifyEmail,
    UpdatePassword,
    ConfigurePasskey,
}

impl RequiredAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::ConfigureOtp => "configure_otp",
            Self::VerifyEmail => "verify_email",
            Self::UpdatePassword => "update_password",
            Self::ConfigurePasskey => "configure_passkey",
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RequiredActionsArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[schemars(description = "UUID of the user; see list_users.")]
    pub user_id: String,
    #[schemars(
        description = "The complete list of actions the user must perform at next sign-in: configure_otp, verify_email, update_password, configure_passkey. It replaces the current list; an empty list clears it."
    )]
    pub actions: Vec<RequiredAction>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UserRoleArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[schemars(description = "UUID of the user; see list_users.")]
    pub user_id: String,
    #[schemars(description = "UUID of the role; see list_roles.")]
    pub role_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SessionArgs {
    #[serde(flatten)]
    pub scope: Scope,
    #[schemars(description = "UUID of the user; see list_users.")]
    pub user_id: String,
    #[schemars(description = "UUID of the session; see list_user_sessions.")]
    pub session_id: String,
}

#[derive(Clone)]
pub struct FerrisKeyTools {
    api: ApiClient,
    tool_router: ToolRouter<Self>,
}

impl FerrisKeyTools {
    pub fn new(api: ApiClient) -> Self {
        Self {
            api,
            tool_router: Self::tool_router(),
        }
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    fn session<'a>(&'a self, parts: &'a Parts, scope: &Scope) -> Result<Session<'a>, Fault> {
        let token = token(parts)?;
        let instance = parts
            .extensions
            .get::<Instance>()
            .ok_or_else(|| Fault::new("no FerrisKey instance was selected"))?;
        let realm = match scope.realm.as_deref() {
            Some(realm) => crate::valid_realm(realm)
                .ok_or_else(|| Fault::new("realm must be a plain realm name"))?
                .to_owned(),
            None => token_realm(token)
                .ok_or_else(|| Fault::new("realm is required: the token does not name one"))?,
        };
        Ok(Session {
            api: &self.api,
            instance,
            token,
            realm,
        })
    }
}

struct Session<'a> {
    api: &'a ApiClient,
    instance: &'a Instance,
    token: &'a str,
    realm: String,
}

impl Session<'_> {
    fn path<'b>(&'b self, rest: &[&'b str]) -> Vec<&'b str> {
        let mut segments = vec!["realms", self.realm.as_str()];
        segments.extend_from_slice(rest);
        segments
    }

    async fn get(&self, rest: &[&str], query: &[(&str, String)]) -> Result<Value, Fault> {
        self.api
            .get(self.instance, self.token, &self.path(rest), query)
            .await
    }

    async fn post(&self, rest: &[&str], body: Option<&Value>) -> Result<Value, Fault> {
        self.api
            .post(self.instance, self.token, &self.path(rest), body)
            .await
    }

    async fn put(&self, rest: &[&str], body: &Value) -> Result<Value, Fault> {
        self.api
            .put(self.instance, self.token, &self.path(rest), body)
            .await
    }

    async fn delete(&self, rest: &[&str]) -> Result<Value, Fault> {
        self.api
            .delete(self.instance, self.token, &self.path(rest))
            .await
    }

    async fn user_form(&self, user: &str) -> Result<Map<String, Value>, Fault> {
        let found = self.get(&["users", user], &[]).await?;
        let current = found
            .get("data")
            .and_then(Value::as_object)
            .ok_or_else(|| Fault::new("FerrisKey did not return the user"))?;
        let mut form = Map::new();
        for key in ["firstname", "lastname", "email"] {
            form.insert(
                key.to_owned(),
                current.get(key).cloned().unwrap_or(Value::Null),
            );
        }
        form.insert(
            "email_verified".to_owned(),
            current
                .get("email_verified")
                .cloned()
                .unwrap_or(Value::Bool(false)),
        );
        form.insert(
            "enabled".to_owned(),
            current.get("enabled").cloned().unwrap_or(Value::Bool(true)),
        );
        Ok(form)
    }
}

pub fn bearer(parts: &Parts) -> Option<&str> {
    let header = parts
        .headers
        .get(http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, token) = header.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then_some(token.trim())
}

fn token(parts: &Parts) -> Result<&str, Fault> {
    bearer(parts).ok_or_else(|| Fault::new("access token missing"))
}

fn id(label: &str, value: &str) -> Result<String, Fault> {
    Uuid::parse_str(value.trim())
        .map(|uuid| uuid.to_string())
        .map_err(|_| Fault::new(format!("{label} must be a UUID")))
}

fn paging(paging: &Paging) -> Result<Vec<(&'static str, String)>, Fault> {
    let mut query = Vec::new();
    if let Some(page) = paging.page {
        if page == 0 {
            return Err(Fault::new("page starts at 1"));
        }
        query.push(("page", page.to_string()));
    }
    if let Some(limit) = paging.limit {
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(Fault::new("limit must be between 1 and 100"));
        }
        query.push(("limit", limit.to_string()));
    }
    Ok(query)
}

fn put_text(query: &mut Vec<(&'static str, String)>, key: &'static str, value: Option<String>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        query.push((key, value));
    }
}

fn put_flag(query: &mut Vec<(&'static str, String)>, key: &'static str, value: Option<bool>) {
    if let Some(value) = value {
        query.push((key, value.to_string()));
    }
}

fn event_types(types: &[String]) -> Result<String, Fault> {
    let plain = |kind: &String| {
        !kind.is_empty()
            && kind
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    if types.iter().all(plain) {
        Ok(types.join(","))
    } else {
        Err(Fault::new(
            "event_types are lowercase names such as login_failure",
        ))
    }
}

fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for key in SECRET_KEYS {
                map.remove(key);
            }
            map.values_mut().for_each(redact);
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}

fn finish(outcome: Result<Value, Fault>) -> CallToolResult {
    match outcome {
        Ok(Value::Null) => {
            CallToolResult::success(vec![ContentBlock::text(json!({"ok": true}).to_string())])
        }
        Ok(value) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
        Err(fault) => CallToolResult::error(vec![ContentBlock::text(fault.to_string())]),
    }
}

fn without_secrets(outcome: Result<Value, Fault>) -> Result<Value, Fault> {
    outcome.map(|mut value| {
        redact(&mut value);
        value
    })
}

#[tool_router(router = tool_router)]
impl FerrisKeyTools {
    #[tool(
        description = "List the realms the caller may access on this FerrisKey instance.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_realms(
        &self,
        Parameters(args): Parameters<ListArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let mut query = paging(&args.paging)?;
                put_text(&mut query, "search", args.search);
                session.get(&["users", "@me", "realms"], &query).await
            }
            .await,
        )
    }

    #[tool(
        description = "List or search the users of a realm. Returns one page with each user's id, username, name, email, enabled flag and required actions.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_users(
        &self,
        Parameters(args): Parameters<ListUsersArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let mut query = paging(&args.paging)?;
                put_text(&mut query, "search", args.search);
                put_text(&mut query, "username", args.username);
                put_text(&mut query, "email", args.email);
                put_flag(&mut query, "enabled", args.enabled);
                if let Some(role) = args.role_id {
                    query.push(("role_id", id("role_id", &role)?));
                }
                session.get(&["users"], &query).await
            }
            .await,
        )
    }

    #[tool(
        description = "Get one user of a realm by id.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_user(
        &self,
        Parameters(args): Parameters<UserArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let user = id("user_id", &args.user_id)?;
                session.get(&["users", &user], &[]).await
            }
            .await,
        )
    }

    #[tool(
        description = "List the roles assigned to a user.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_user_roles(
        &self,
        Parameters(args): Parameters<UserArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let user = id("user_id", &args.user_id)?;
                session.get(&["users", &user, "roles"], &[]).await
            }
            .await,
        )
    }

    #[tool(
        description = "List or search the roles of a realm, realm roles and client roles, with their ids.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_roles(
        &self,
        Parameters(args): Parameters<ListArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let mut query = paging(&args.paging)?;
                put_text(&mut query, "search", args.search);
                session.get(&["roles"], &query).await
            }
            .await,
        )
    }

    #[tool(
        description = "List or search the clients of a realm. Client secrets are never returned.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_clients(
        &self,
        Parameters(args): Parameters<ListClientsArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let mut query = paging(&args.paging)?;
                put_text(&mut query, "search", args.search);
                put_flag(&mut query, "enabled", args.enabled);
                without_secrets(session.get(&["clients"], &query).await)
            }
            .await,
        )
    }

    #[tool(
        description = "Get one client of a realm by id. Its secret is never returned.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_client(
        &self,
        Parameters(args): Parameters<ClientArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let client = id("client_id", &args.client_id)?;
                without_secrets(session.get(&["clients", &client], &[]).await)
            }
            .await,
        )
    }

    #[tool(
        description = "List the organisations of a realm. The id is the organisation_id argument of the group tools.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_organisations(
        &self,
        Parameters(args): Parameters<ListArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let mut query = paging(&args.paging)?;
                put_text(&mut query, "search", args.search);
                session.get(&["organizations"], &query).await
            }
            .await,
        )
    }

    #[tool(
        description = "List the groups of an organisation.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_groups(
        &self,
        Parameters(args): Parameters<ListGroupsArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let organisation = id("organisation_id", &args.organisation_id)?;
                let mut query = paging(&args.paging)?;
                put_text(&mut query, "search", args.search);
                session
                    .get(&["organizations", &organisation, "groups"], &query)
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "List the members of a group with their user identity.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_group_members(
        &self,
        Parameters(args): Parameters<ListGroupMembersArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let organisation = id("organisation_id", &args.organisation_id)?;
                let group = id("group_id", &args.group_id)?;
                let mut query = paging(&args.paging)?;
                put_text(&mut query, "search", args.search);
                session
                    .get(
                        &["organizations", &organisation, "groups", &group, "members"],
                        &query,
                    )
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "List the webhooks of a realm with their endpoint and subscribed events. Secrets and header values are never returned.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_webhooks(
        &self,
        Parameters(args): Parameters<ListArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let mut query = paging(&args.paging)?;
                put_text(&mut query, "search", args.search);
                session.get(&["webhooks"], &query).await
            }
            .await,
        )
    }

    #[tool(
        description = "List the security events (audit trail) of a realm, newest first: sign-ins and failures, password resets, role changes, session revocations. Filter by type, status, actor, target type, time window or IP address.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_security_events(
        &self,
        Parameters(args): Parameters<SecurityEventsArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let mut query = paging(&args.paging)?;
                query.push(("order", "desc".to_owned()));
                if let Some(types) = args.event_types.filter(|types| !types.is_empty()) {
                    query.push(("event_types", event_types(&types)?));
                }
                if let Some(status) = args.status {
                    query.push(("status", status.as_str().to_owned()));
                }
                if let Some(actor) = args.actor_id {
                    query.push(("actor_id", id("actor_id", &actor)?));
                }
                put_text(&mut query, "target_type", args.target_type);
                put_text(&mut query, "from_timestamp", args.from_timestamp);
                put_text(&mut query, "to_timestamp", args.to_timestamp);
                put_text(&mut query, "ip_address", args.ip_address);
                session
                    .get(&["seawatch", "v1", "security-events"], &query)
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "List the active sessions of a user with their user agent, IP address and times.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_user_sessions(
        &self,
        Parameters(args): Parameters<UserPageArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let user = id("user_id", &args.user_id)?;
                let query = paging(&args.paging)?;
                session.get(&["users", &user, "sessions"], &query).await
            }
            .await,
        )
    }

    #[tool(
        description = "Create a user in a realm. No password is set; use require_user_actions with update_password to make the user choose one at first sign-in.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_user(
        &self,
        Parameters(args): Parameters<CreateUserArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let mut body = Map::new();
                body.insert("username".to_owned(), Value::String(args.username));
                for (key, value) in [
                    ("firstname", args.firstname),
                    ("lastname", args.lastname),
                    ("email", args.email),
                ] {
                    if let Some(value) = value {
                        body.insert(key.to_owned(), Value::String(value));
                    }
                }
                if let Some(verified) = args.email_verified {
                    body.insert("email_verified".to_owned(), Value::Bool(verified));
                }
                session.post(&["users"], Some(&Value::Object(body))).await
            }
            .await,
        )
    }

    #[tool(
        description = "Change a user's first name, last name or email. Only the fields given change; the others, and whether the account is enabled, stay as they are.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_user(
        &self,
        Parameters(args): Parameters<UpdateUserArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let user = id("user_id", &args.user_id)?;
                let changes = [
                    ("firstname", args.firstname.map(Value::String)),
                    ("lastname", args.lastname.map(Value::String)),
                    ("email", args.email.map(Value::String)),
                    ("email_verified", args.email_verified.map(Value::Bool)),
                ];
                if changes.iter().all(|(_, value)| value.is_none()) {
                    return Err(Fault::new("give at least one field to change"));
                }
                let mut form = session.user_form(&user).await?;
                for (key, value) in changes {
                    if let Some(value) = value {
                        form.insert(key.to_owned(), value);
                    }
                }
                session.put(&["users", &user], &Value::Object(form)).await
            }
            .await,
        )
    }

    #[tool(
        description = "Disable a user's account. The user can no longer sign in and every token it holds is revoked at once.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn disable_user(
        &self,
        Parameters(args): Parameters<UserArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(self.set_enabled(&parts, &args, false).await)
    }

    #[tool(
        description = "Enable a user's account again so the user can sign in.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn enable_user(
        &self,
        Parameters(args): Parameters<UserArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(self.set_enabled(&parts, &args, true).await)
    }

    #[tool(
        description = "Set the actions a user must perform at next sign-in (for example update_password to force a password reset, configure_otp, verify_email). The list replaces the current one; an empty list clears it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn require_user_actions(
        &self,
        Parameters(args): Parameters<RequiredActionsArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let user = id("user_id", &args.user_id)?;
                let mut form = session.user_form(&user).await?;
                let actions: Vec<Value> = args
                    .actions
                    .iter()
                    .map(|action| Value::String(action.as_str().to_owned()))
                    .collect();
                form.insert("required_actions".to_owned(), Value::Array(actions));
                session.put(&["users", &user], &Value::Object(form)).await
            }
            .await,
        )
    }

    #[tool(
        description = "Unlock a user's account after too many failed sign-ins.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn unlock_user(
        &self,
        Parameters(args): Parameters<UserArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let user = id("user_id", &args.user_id)?;
                session.post(&["users", &user, "unlock"], None).await
            }
            .await,
        )
    }

    #[tool(
        description = "Assign a role to a user.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn assign_role(
        &self,
        Parameters(args): Parameters<UserRoleArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let user = id("user_id", &args.user_id)?;
                let role = id("role_id", &args.role_id)?;
                session.post(&["users", &user, "roles", &role], None).await
            }
            .await,
        )
    }

    #[tool(
        description = "Remove a role from a user. The user loses the access the role gave.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn unassign_role(
        &self,
        Parameters(args): Parameters<UserRoleArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let user = id("user_id", &args.user_id)?;
                let role = id("role_id", &args.role_id)?;
                session.delete(&["users", &user, "roles", &role]).await
            }
            .await,
        )
    }

    #[tool(
        description = "Add a user to a group of an organisation.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_group_member(
        &self,
        Parameters(args): Parameters<GroupMemberArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let organisation = id("organisation_id", &args.organisation_id)?;
                let group = id("group_id", &args.group_id)?;
                let user = id("user_id", &args.user_id)?;
                session
                    .post(
                        &["organizations", &organisation, "groups", &group, "members"],
                        Some(&json!({"user_id": user})),
                    )
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "Remove a user from a group of an organisation. The user loses what the group granted.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn remove_group_member(
        &self,
        Parameters(args): Parameters<GroupMemberArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let organisation = id("organisation_id", &args.organisation_id)?;
                let group = id("group_id", &args.group_id)?;
                let user = id("user_id", &args.user_id)?;
                session
                    .delete(&[
                        "organizations",
                        &organisation,
                        "groups",
                        &group,
                        "members",
                        &user,
                    ])
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "Revoke one session of a user. Its tokens stop working at once and the user must sign in again on that device.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn revoke_user_session(
        &self,
        Parameters(args): Parameters<SessionArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let session = self.session(&parts, &args.scope)?;
                let user = id("user_id", &args.user_id)?;
                let target = id("session_id", &args.session_id)?;
                session.delete(&["users", &user, "sessions", &target]).await
            }
            .await,
        )
    }
}

impl FerrisKeyTools {
    async fn set_enabled(
        &self,
        parts: &Parts,
        args: &UserArgs,
        enabled: bool,
    ) -> Result<Value, Fault> {
        let session = self.session(parts, &args.scope)?;
        let user = id("user_id", &args.user_id)?;
        let mut form = session.user_form(&user).await?;
        form.insert("enabled".to_owned(), Value::Bool(enabled));
        session.put(&["users", &user], &Value::Object(form)).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FerrisKeyTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "FerrisKey administration tools for one instance. Every tool acts on the token's realm unless it is given a realm. Call list_users, list_roles or list_organisations first to find the ids the other tools take.",
        )
    }
}
