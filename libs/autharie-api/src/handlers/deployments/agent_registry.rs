use autharie_auth::Identity;
use autharie_core::{
    agents::{
        registry::{
            AgentClient, AgentClientId, AutonomousInput, CredentialFetch, DeliveredCredential,
            RegistryView, SealingJwk, SealingKey,
        },
        registry_ports::AgentRegistryService,
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/agents/clients")]
pub struct AgentClientsRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/agents/clients/{client_id}"
)]
pub struct AgentClientRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub client_id: String,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/agents/clients/{client_id}/approve"
)]
pub struct ApproveAgentRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub client_id: String,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/agents/clients/{client_id}/block"
)]
pub struct BlockAgentRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub client_id: String,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/agents/clients/{client_id}/unblock"
)]
pub struct UnblockAgentRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub client_id: String,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/agents/access")]
pub struct AgentAccessRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/agents/autonomous")]
pub struct AutonomousAgentsRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/agents/autonomous/{client_id}/rotate"
)]
pub struct RotateAutonomousRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub client_id: String,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/agents/credentials/{request_id}"
)]
pub struct AgentCredentialRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub request_id: Uuid,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PutAgentAccessRequest {
    pub approval_required: bool,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAutonomousRequest {
    pub name: String,
    pub description: Option<String>,
    pub sealing_key: SealingJwk,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RotateAutonomousRequest {
    pub sealing_key: SealingJwk,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct AgentAccessResponse {
    pub data: AgentAccessData,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct AgentAccessData {
    pub approval_required: bool,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct AgentClientsResponse {
    pub approval_required: bool,
    pub data: Vec<AgentClient>,
    pub truncated: bool,
}

impl From<RegistryView> for AgentClientsResponse {
    fn from(view: RegistryView) -> Self {
        Self {
            approval_required: view.approval_required,
            data: view.data,
            truncated: view.truncated,
        }
    }
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct CredentialRequestResponse {
    pub data: CredentialRequestData,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct CredentialRequestData {
    pub request_id: Uuid,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct PendingCredential {
    pub status: &'static str,
}

#[derive(Serialize, ToSchema, PartialEq)]
#[serde(untagged)]
pub enum AgentCredentialResponse {
    Delivered { data: DeliveredCredential },
    Pending { data: PendingCredential },
}

#[derive(Serialize, PartialEq)]
pub struct AgentChanged {}

fn client_id(raw: &str) -> Result<AgentClientId, ApiError> {
    Ok(AgentClientId::try_from(raw)?)
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/clients",
    summary = "list the agents connected to a deployment",
    tag = "agents",
    description = "The clients the data plane reported as agents (registered through DCR or CIMD, or created by Autharie as autonomous agents), the clients only seen calling the gateway, each with its source, kind, access state and last activity over 30 days. Requires VIEW_INSTANCES.",
    params(AgentClientsRoute),
    responses(
        (status = 200, description = "The registry", body = AgentClientsResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_agent_clients_handler(
    AgentClientsRoute {
        organisation_id,
        deployment_id,
    }: AgentClientsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentClientsResponse>, ApiError> {
    let view = state
        .service
        .list_agent_clients(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            state.quickwit_search.as_deref(),
        )
        .await?;

    Ok(Response::OK(view.into()))
}

#[utoipa::path(
    put,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/access",
    summary = "require approval for new agents",
    tag = "agents",
    description = "With approval required, only approved agents get past the gateway; blocked agents are always refused. Requires MANAGE_INSTANCES.",
    params(AgentAccessRoute),
    request_body = PutAgentAccessRequest,
    responses(
        (status = 200, description = "Stored, and being applied", body = AgentAccessResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn put_agent_access_handler(
    AgentAccessRoute {
        organisation_id,
        deployment_id,
    }: AgentAccessRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<PutAgentAccessRequest>,
) -> Result<Response<AgentAccessResponse>, ApiError> {
    state
        .service
        .set_approval_mode(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            body.approval_required,
        )
        .await?;

    Ok(Response::OK(AgentAccessResponse {
        data: AgentAccessData {
            approval_required: body.approval_required,
        },
    }))
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/clients/{client_id}/approve",
    summary = "approve an agent",
    tag = "agents",
    description = "Lets the agent past the gateway when approval is required, and lifts a block. Requires MANAGE_INSTANCES.",
    params(ApproveAgentRoute),
    responses(
        (status = 204, description = "Approved, and being applied"),
        (status = 400, description = "The client id is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn approve_agent_handler(
    ApproveAgentRoute {
        organisation_id,
        deployment_id,
        client_id: raw,
    }: ApproveAgentRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentChanged>, ApiError> {
    state
        .service
        .approve_agent(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            client_id(&raw)?,
        )
        .await?;

    Ok(Response::NoContent)
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/clients/{client_id}/block",
    summary = "block an agent",
    tag = "agents",
    description = "The gateway refuses the agent before asking the PDP, whatever the policies, and its FerrisKey client is disabled. Requires MANAGE_INSTANCES.",
    params(BlockAgentRoute),
    responses(
        (status = 204, description = "Blocked, and being applied"),
        (status = 400, description = "The client id is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn block_agent_handler(
    BlockAgentRoute {
        organisation_id,
        deployment_id,
        client_id: raw,
    }: BlockAgentRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentChanged>, ApiError> {
    state
        .service
        .block_agent(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            client_id(&raw)?,
        )
        .await?;

    Ok(Response::NoContent)
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/clients/{client_id}/unblock",
    summary = "unblock an agent",
    tag = "agents",
    description = "Lifts the block and re-enables the FerrisKey client. The agent is approved again only if it was approved before. Requires MANAGE_INSTANCES.",
    params(UnblockAgentRoute),
    responses(
        (status = 204, description = "Unblocked, and being applied"),
        (status = 400, description = "The client id is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn unblock_agent_handler(
    UnblockAgentRoute {
        organisation_id,
        deployment_id,
        client_id: raw,
    }: UnblockAgentRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentChanged>, ApiError> {
    state
        .service
        .unblock_agent(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            client_id(&raw)?,
        )
        .await?;

    Ok(Response::NoContent)
}

#[utoipa::path(
    delete,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/clients/{client_id}",
    summary = "delete an agent",
    tag = "agents",
    description = "Asks the data plane to delete the agent's FerrisKey client, only if it is an agent client, and forgets its access. Requires MANAGE_INSTANCES.",
    params(AgentClientRoute),
    responses(
        (status = 204, description = "Deleted, and being applied"),
        (status = 400, description = "The client id is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_agent_handler(
    AgentClientRoute {
        organisation_id,
        deployment_id,
        client_id: raw,
    }: AgentClientRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentChanged>, ApiError> {
    state
        .service
        .delete_agent(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            client_id(&raw)?,
        )
        .await?;

    Ok(Response::NoContent)
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/autonomous",
    summary = "create an autonomous agent",
    tag = "agents",
    description = "Asks the data plane to create a client for an autonomous agent (a workflow, a bot) and to seal its secret to the supplied public key. Poll `GET /credentials/{request_id}` for the sealed secret, which is handed out once and kept 15 minutes at most. Requires MANAGE_INSTANCES.",
    params(AutonomousAgentsRoute),
    request_body = CreateAutonomousRequest,
    responses(
        (status = 202, description = "Accepted", body = CredentialRequestResponse),
        (status = 400, description = "A field or the sealing key is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "Agents are not enabled, or the limit is reached", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_autonomous_agent_handler(
    AutonomousAgentsRoute {
        organisation_id,
        deployment_id,
    }: AutonomousAgentsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<CreateAutonomousRequest>,
) -> Result<Response<CredentialRequestResponse>, ApiError> {
    let input = AutonomousInput::parse(&body.name, body.description.as_deref(), body.sealing_key)?;

    let request_id = state
        .service
        .create_autonomous_agent(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            input,
        )
        .await?;

    Ok(Response::Accepted(CredentialRequestResponse {
        data: CredentialRequestData { request_id },
    }))
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/autonomous/{client_id}/rotate",
    summary = "rotate an autonomous agent's secret",
    tag = "agents",
    description = "FerrisKey cannot regenerate a secret, so the data plane recreates the client under the same client id; the old secret stops working. Poll `GET /credentials/{request_id}` as for a new agent. Requires MANAGE_INSTANCES.",
    params(RotateAutonomousRoute),
    request_body = RotateAutonomousRequest,
    responses(
        (status = 202, description = "Accepted", body = CredentialRequestResponse),
        (status = 400, description = "The sealing key is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment or autonomous agent", body = ApiError),
        (status = 409, description = "Agents are not enabled", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn rotate_autonomous_secret_handler(
    RotateAutonomousRoute {
        organisation_id,
        deployment_id,
        client_id: raw,
    }: RotateAutonomousRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<RotateAutonomousRequest>,
) -> Result<Response<CredentialRequestResponse>, ApiError> {
    let sealing_key = SealingKey::try_from(body.sealing_key)?;

    let request_id = state
        .service
        .rotate_autonomous_secret(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            client_id(&raw)?,
            sealing_key,
        )
        .await?;

    Ok(Response::Accepted(CredentialRequestResponse {
        data: CredentialRequestData { request_id },
    }))
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/credentials/{request_id}",
    summary = "collect an autonomous agent's sealed secret",
    tag = "agents",
    description = "200 with the sealed secret the first time it is ready, then 410; 202 while the data plane has not produced it; 404 when the request is unknown or expired (15 minutes). The secret is sealed with RSA-OAEP-256 to the key sent with the request, base64url. Requires MANAGE_INSTANCES.",
    params(AgentCredentialRoute),
    responses(
        (status = 200, description = "The sealed secret", body = AgentCredentialResponse),
        (status = 202, description = "Not ready yet", body = AgentCredentialResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "Unknown or expired request", body = ApiError),
        (status = 410, description = "Already delivered", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_agent_credential_handler(
    AgentCredentialRoute {
        organisation_id,
        deployment_id,
        request_id,
    }: AgentCredentialRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentCredentialResponse>, ApiError> {
    let fetched = state
        .service
        .fetch_agent_credential(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            request_id,
        )
        .await?;

    Ok(match fetched {
        CredentialFetch::Delivered(data) => {
            Response::OK(AgentCredentialResponse::Delivered { data })
        }
        CredentialFetch::Pending => Response::Accepted(AgentCredentialResponse::Pending {
            data: PendingCredential { status: "pending" },
        }),
    })
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, response::IntoResponse};
    use serde_json::json;

    use super::*;

    fn status_of(error: ApiError) -> StatusCode {
        error.into_response().status()
    }

    fn small_key() -> SealingJwk {
        serde_json::from_value(json!({
            "kty": "RSA", "alg": "RSA-OAEP-256", "e": "AQAB",
            "n": "sXchDaQebHnPiGvyDOAT4saGEUetSyo9MKLOoWFsueri23bOdgWp4Dy1Wl"
        }))
        .unwrap()
    }

    #[test]
    fn spec_ar_6_a_key_under_2048_bits_is_a_400() {
        let error = AutonomousInput::parse("n8n", None, small_key()).map_err(ApiError::from);

        assert_eq!(
            status_of(error.expect_err("refused")),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn spec_ar_6_a_key_that_is_not_rsa_is_a_400() {
        let ec: SealingJwk = serde_json::from_value(json!({
            "kty": "EC", "alg": "ES256", "n": "AA", "e": "AQAB"
        }))
        .unwrap();

        let error = SealingKey::try_from(ec).map_err(ApiError::from);

        assert_eq!(
            status_of(error.expect_err("refused")),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn a_client_id_with_a_space_is_a_400() {
        assert_eq!(
            status_of(client_id("a b").expect_err("refused")),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn an_unknown_field_is_not_accepted() {
        assert!(
            serde_json::from_value::<PutAgentAccessRequest>(
                json!({ "approval_required": true, "extra": 1 })
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<CreateAutonomousRequest>(
                json!({ "name": "n", "sealing_key": {"kty": "RSA", "n": "A", "e": "A"}, "x": 1 })
            )
            .is_err()
        );
    }

    #[test]
    fn a_delivered_credential_serialises_under_data_and_a_pending_one_says_so() {
        let pending = serde_json::to_value(AgentCredentialResponse::Pending {
            data: PendingCredential { status: "pending" },
        })
        .unwrap();

        assert_eq!(pending, json!({ "data": { "status": "pending" } }));
    }
}
