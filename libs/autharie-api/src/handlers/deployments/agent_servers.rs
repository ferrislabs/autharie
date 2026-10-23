use autharie_auth::Identity;
use autharie_core::{
    agents::{
        server_ports::AgentServerService,
        servers::{CustomerServer, ServerId, ServerInput},
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
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/agents/servers")]
pub struct AgentServersRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/agents/servers/{server_id}"
)]
pub struct AgentServerRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub server_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/agents/servers/{server_id}/discover"
)]
pub struct AgentServerDiscoverRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub server_id: Uuid,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterAgentServerRequest {
    /// Short name, unique in the deployment: `^[a-z][a-z0-9-]{1,30}$`, never `ferriskey` or `autharie`.
    /// It prefixes the server's tools (`<slug>/<tool>`) and its gateway path (`/mcp/<slug>`).
    pub slug: String,
    pub name: String,
    /// A public `https://` URL, without user name or password.
    pub url: String,
    /// `true` when left out.
    pub enabled: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeAgentServerRequest {
    pub name: String,
    pub url: String,
    pub enabled: bool,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct AgentServerResponse {
    pub data: CustomerServer,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct AgentServersResponse {
    pub data: Vec<CustomerServer>,
}

#[derive(Serialize, PartialEq)]
pub struct AgentServerDeleted {}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct AgentServerDiscoveryAsked {
    pub asked: bool,
}

fn register_input(body: RegisterAgentServerRequest) -> Result<ServerInput, ApiError> {
    Ok(
        ServerInput::register(&body.slug, &body.name, &body.url, body.enabled)
            .map_err(autharie_core::agents::AgentError::from)?,
    )
}

fn change_input(body: ChangeAgentServerRequest) -> Result<ServerInput, ApiError> {
    Ok(ServerInput::change(&body.name, &body.url, body.enabled)
        .map_err(autharie_core::agents::AgentError::from)?)
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/servers",
    summary = "list the MCP servers registered on a deployment",
    tag = "agents",
    description = "Each server carries the tools the data plane last discovered on it, with their read-only and destructive hints, and the URL agents reach it at through the gateway. Requires VIEW_INSTANCES.",
    params(AgentServersRoute),
    responses(
        (status = 200, description = "The servers", body = AgentServersResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_agent_servers_handler(
    AgentServersRoute {
        organisation_id,
        deployment_id,
    }: AgentServersRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentServersResponse>, ApiError> {
    let data = state
        .service
        .list_agent_servers(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
        )
        .await?;

    Ok(Response::OK(AgentServersResponse { data }))
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/servers",
    summary = "register an MCP server on a deployment",
    tag = "agents",
    description = "Refuses anything but a public https URL, stores the server, asks the data plane to expose it through the gateway and to discover its tools. At most 10 per deployment. FerrisKey deployments only. Requires MANAGE_INSTANCES.",
    params(AgentServersRoute),
    request_body = RegisterAgentServerRequest,
    responses(
        (status = 201, description = "Registered, and being applied", body = AgentServerResponse),
        (status = 400, description = "A field is not valid; the message names it", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The slug is taken, the deployment holds 10 servers, or it is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn register_agent_server_handler(
    AgentServersRoute {
        organisation_id,
        deployment_id,
    }: AgentServersRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<RegisterAgentServerRequest>,
) -> Result<Response<AgentServerResponse>, ApiError> {
    let input = register_input(body)?;

    let data = state
        .service
        .register_agent_server(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            input,
        )
        .await?;

    Ok(Response::Created(AgentServerResponse { data }))
}

#[utoipa::path(
    put,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/servers/{server_id}",
    summary = "change an MCP server",
    tag = "agents",
    description = "Changes the name, URL and whether the server is on. The slug never changes. A new URL clears the discovered tools and asks for a new discovery. Requires MANAGE_INSTANCES.",
    params(AgentServerRoute),
    request_body = ChangeAgentServerRequest,
    responses(
        (status = 200, description = "Stored, and being applied", body = AgentServerResponse),
        (status = 400, description = "A field is not valid; the message names it", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment or server", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_agent_server_handler(
    AgentServerRoute {
        organisation_id,
        deployment_id,
        server_id,
    }: AgentServerRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<ChangeAgentServerRequest>,
) -> Result<Response<AgentServerResponse>, ApiError> {
    let input = change_input(body)?;

    let data = state
        .service
        .update_agent_server(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            ServerId(server_id),
            input,
        )
        .await?;

    Ok(Response::OK(AgentServerResponse { data }))
}

#[utoipa::path(
    delete,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/servers/{server_id}",
    summary = "remove an MCP server",
    tag = "agents",
    description = "Removes the server, and with it its gateway upstream, route and audience client. The policies that name it stay. Requires MANAGE_INSTANCES.",
    params(AgentServerRoute),
    responses(
        (status = 204, description = "Removed"),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment or server", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn remove_agent_server_handler(
    AgentServerRoute {
        organisation_id,
        deployment_id,
        server_id,
    }: AgentServerRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentServerDeleted>, ApiError> {
    state
        .service
        .remove_agent_server(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            ServerId(server_id),
        )
        .await?;

    Ok(Response::NoContent)
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/servers/{server_id}/discover",
    summary = "ask the data plane to list a server's tools again",
    tag = "agents",
    description = "Answers at once; the new tools show on the server once the data plane has reported them. Requires MANAGE_INSTANCES.",
    params(AgentServerDiscoverRoute),
    responses(
        (status = 202, description = "Asked", body = AgentServerDiscoveryAsked),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment or server", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn discover_agent_server_handler(
    AgentServerDiscoverRoute {
        organisation_id,
        deployment_id,
        server_id,
    }: AgentServerDiscoverRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentServerDiscoveryAsked>, ApiError> {
    state
        .service
        .discover_agent_server(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            ServerId(server_id),
        )
        .await?;

    Ok(Response::Accepted(AgentServerDiscoveryAsked {
        asked: true,
    }))
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, response::IntoResponse};
    use serde_json::json;

    use super::*;

    fn register(value: serde_json::Value) -> RegisterAgentServerRequest {
        serde_json::from_value(value).expect("a body")
    }

    fn status_of(error: ApiError) -> StatusCode {
        error.into_response().status()
    }

    #[test]
    fn a_public_https_server_is_read() {
        let input = register_input(register(json!({
            "slug": "crm", "name": "CRM", "url": "https://tools.example.com/mcp"
        })))
        .expect("accepted");

        assert_eq!(input.slug.unwrap().as_str(), "crm");
        assert!(input.enabled);
    }

    #[test]
    fn a_url_slug_or_name_that_is_refused_is_a_400_naming_the_field() {
        for (body, field) in [
            (
                json!({ "slug": "crm", "name": "CRM", "url": "http://tools.example.com/" }),
                "url",
            ),
            (
                json!({ "slug": "crm", "name": "CRM", "url": "https://10.0.0.5/mcp" }),
                "url",
            ),
            (
                json!({ "slug": "crm", "name": "CRM", "url": "https://user:pw@x.example/" }),
                "url",
            ),
            (
                json!({ "slug": "ferriskey", "name": "CRM", "url": "https://x.example/" }),
                "slug",
            ),
            (
                json!({ "slug": "crm", "name": " ", "url": "https://x.example/" }),
                "name",
            ),
        ] {
            let error = register_input(register(body)).expect_err("refused");
            let message = error.to_string();

            assert!(message.contains(field), "{message}");
            assert_eq!(status_of(error), StatusCode::BAD_REQUEST);
        }
    }

    #[test]
    fn an_unknown_field_is_not_accepted() {
        let parsed = serde_json::from_value::<RegisterAgentServerRequest>(json!({
            "slug": "crm", "name": "CRM", "url": "https://x.example/", "token": "t"
        }));

        assert!(parsed.is_err());
    }
}
