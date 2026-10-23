use autharie_auth::Identity;
use autharie_core::{
    agents::{
        AgentDecision, AgentPolicy, AgentsView, PolicyId,
        commands::{AgentSettingsInput, PolicyInput},
        decision::DecisionQuery,
        ports::AgentService,
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use axum_extra::routing::TypedPath;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/agents")]
pub struct AgentsRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/agents/policies")]
pub struct AgentPoliciesRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/agents/policies/{policy_id}"
)]
pub struct AgentPolicyRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub policy_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/agents/decisions")]
pub struct AgentDecisionsRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PutAgentsRequest {
    pub enabled: bool,
    /// A realm of the deployment. `master` when left out.
    pub realm: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPolicyRequest {
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    /// The console's builder model. Opaque to the API: stored and returned as sent.
    #[schema(value_type = Option<Object>)]
    pub rule: Option<Value>,
    /// The Cedar source. It must parse, otherwise the answer is a 422 carrying the parser's message.
    pub cedar: String,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct AgentsResponse {
    pub data: AgentsView,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct AgentPolicyResponse {
    pub data: AgentPolicy,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct AgentPoliciesResponse {
    pub data: Vec<AgentPolicy>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct AgentDecisionsResponse {
    pub data: Vec<AgentDecision>,
}

#[derive(Serialize, PartialEq)]
pub struct AgentPolicyDeleted {}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AgentDecisionsQuery {
    /// Start of the range, inclusive.
    pub from: Option<DateTime<Utc>>,
    /// End of the range, exclusive.
    pub to: Option<DateTime<Utc>>,
    /// `allow` or `deny`.
    pub decision: Option<String>,
    /// Exact tool name.
    pub tool: Option<String>,
    /// Exact subject id.
    pub subject: Option<String>,
    pub agent: Option<String>,
    /// At most 200, 50 when left out.
    pub limit: Option<usize>,
}

fn policy_input(body: AgentPolicyRequest) -> Result<PolicyInput, ApiError> {
    Ok(PolicyInput::parse(
        &body.name,
        body.description.as_deref(),
        body.enabled,
        body.rule,
        &body.cedar,
    )?)
}

fn decision_query(query: AgentDecisionsQuery) -> Result<DecisionQuery, ApiError> {
    Ok(DecisionQuery::parse(
        query.decision.as_deref(),
        query.tool.as_deref(),
        query.subject.as_deref(),
        query.from,
        query.to,
        query.limit,
    )?
    .with_agent(query.agent.as_deref()))
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/agents",
    summary = "read a deployment's agent settings",
    tag = "agents",
    description = "Whether AI agents may reach the deployment through MCP, the realm that decides, the MCP URL, whether the organisation's AI switch is on, and what the data plane last reported. Requires VIEW_INSTANCES.",
    params(AgentsRoute),
    responses(
        (status = 200, description = "The agent settings", body = AgentsResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_agents_handler(
    AgentsRoute {
        organisation_id,
        deployment_id,
    }: AgentsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentsResponse>, ApiError> {
    let data = state
        .service
        .get_agents(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
        )
        .await?;

    Ok(Response::OK(AgentsResponse { data }))
}

#[utoipa::path(
    put,
    path = "/{organisation_id}/deployments/{deployment_id}/agents",
    summary = "change a deployment's agent settings",
    tag = "agents",
    description = "Stores the settings and asks the data plane to apply them. Agents only run while the organisation's AI switch is on as well. FerrisKey deployments only. Requires MANAGE_INSTANCES.",
    params(AgentsRoute),
    request_body = PutAgentsRequest,
    responses(
        (status = 200, description = "Stored, and being applied", body = AgentsResponse),
        (status = 400, description = "The realm is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn put_agents_handler(
    AgentsRoute {
        organisation_id,
        deployment_id,
    }: AgentsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<PutAgentsRequest>,
) -> Result<Response<AgentsResponse>, ApiError> {
    let input = AgentSettingsInput::parse(body.enabled, body.realm.as_deref())?;

    let data = state
        .service
        .set_agents(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            input,
        )
        .await?;

    Ok(Response::OK(AgentsResponse { data }))
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/policies",
    summary = "list a deployment's agent policies",
    tag = "agents",
    description = "Each policy carries the phase the data plane last reported for it. Requires VIEW_INSTANCES.",
    params(AgentPoliciesRoute),
    responses(
        (status = 200, description = "The policies", body = AgentPoliciesResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_agent_policies_handler(
    AgentPoliciesRoute {
        organisation_id,
        deployment_id,
    }: AgentPoliciesRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentPoliciesResponse>, ApiError> {
    let data = state
        .service
        .list_agent_policies(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
        )
        .await?;

    Ok(Response::OK(AgentPoliciesResponse { data }))
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/policies",
    summary = "create an agent policy",
    tag = "agents",
    description = "Checks that the Cedar parses, stores the policy and asks the data plane to apply the deployment's enabled policies. Requires MANAGE_INSTANCES.",
    params(AgentPoliciesRoute),
    request_body = AgentPolicyRequest,
    responses(
        (status = 201, description = "Created, and being applied", body = AgentPolicyResponse),
        (status = 400, description = "A field is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one, or already holds 100 policies", body = ApiError),
        (status = 422, description = "The Cedar does not parse; the message is the parser's", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_agent_policy_handler(
    AgentPoliciesRoute {
        organisation_id,
        deployment_id,
    }: AgentPoliciesRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<AgentPolicyRequest>,
) -> Result<Response<AgentPolicyResponse>, ApiError> {
    let input = policy_input(body)?;

    let data = state
        .service
        .create_agent_policy(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            input,
        )
        .await?;

    Ok(Response::Created(AgentPolicyResponse { data }))
}

#[utoipa::path(
    put,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/policies/{policy_id}",
    summary = "replace an agent policy",
    tag = "agents",
    description = "Replaces the policy, puts it back to `Pending` and asks the data plane to apply the deployment's enabled policies. Requires MANAGE_INSTANCES.",
    params(AgentPolicyRoute),
    request_body = AgentPolicyRequest,
    responses(
        (status = 200, description = "Stored, and being applied", body = AgentPolicyResponse),
        (status = 400, description = "A field is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment or policy", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 422, description = "The Cedar does not parse; the message is the parser's", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_agent_policy_handler(
    AgentPolicyRoute {
        organisation_id,
        deployment_id,
        policy_id,
    }: AgentPolicyRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<AgentPolicyRequest>,
) -> Result<Response<AgentPolicyResponse>, ApiError> {
    let input = policy_input(body)?;

    let data = state
        .service
        .update_agent_policy(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            PolicyId(policy_id),
            input,
        )
        .await?;

    Ok(Response::OK(AgentPolicyResponse { data }))
}

#[utoipa::path(
    delete,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/policies/{policy_id}",
    summary = "delete an agent policy",
    tag = "agents",
    description = "Removes the policy and asks the data plane to apply the remaining ones, which removes it from FerrisKey. Requires MANAGE_INSTANCES.",
    params(AgentPolicyRoute),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment or policy", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_agent_policy_handler(
    AgentPolicyRoute {
        organisation_id,
        deployment_id,
        policy_id,
    }: AgentPolicyRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentPolicyDeleted>, ApiError> {
    state
        .service
        .delete_agent_policy(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            PolicyId(policy_id),
        )
        .await?;

    Ok(Response::NoContent)
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/agents/decisions",
    summary = "search the decisions taken for a deployment's agents",
    tag = "agents",
    description = "Read from the organisation's log index, where the gateway's decision records are shipped (`kind: agent_decision`). Newest first. Requires VIEW_INSTANCES.",
    params(AgentDecisionsRoute, AgentDecisionsQuery),
    responses(
        (status = 200, description = "The decisions", body = AgentDecisionsResponse),
        (status = 400, description = "A filter is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "This installation has no Quickwit configured", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_agent_decisions_handler(
    AgentDecisionsRoute {
        organisation_id,
        deployment_id,
    }: AgentDecisionsRoute,
    Query(query): Query<AgentDecisionsQuery>,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AgentDecisionsResponse>, ApiError> {
    let query = decision_query(query)?;

    let index = state
        .quickwit_search
        .as_deref()
        .ok_or_else(|| ApiError::Conflict {
            reason: "log search is not configured on this installation".to_string(),
        })?;

    let data = state
        .service
        .agent_decisions(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            query,
            index,
        )
        .await?;

    Ok(Response::OK(AgentDecisionsResponse { data }))
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, response::IntoResponse};
    use serde_json::json;

    use super::*;

    fn body(value: serde_json::Value) -> AgentPolicyRequest {
        serde_json::from_value(value).expect("a body")
    }

    fn status_of(error: ApiError) -> StatusCode {
        error.into_response().status()
    }

    #[test]
    fn a_valid_body_is_read() {
        let input = policy_input(body(json!({
            "name": "read-only",
            "enabled": true,
            "rule": { "effect": "permit" },
            "cedar": "permit(principal, action, resource);"
        })))
        .expect("accepted");

        assert_eq!(input.name, "read-only");
        assert!(input.rule.is_some());
    }

    #[test]
    fn a_blank_name_is_a_400() {
        let error = policy_input(body(json!({
            "name": " ", "enabled": true, "cedar": "permit(principal, action, resource);"
        })))
        .expect_err("refused");

        assert_eq!(status_of(error), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn cedar_that_does_not_parse_is_a_422_carrying_the_message() {
        let error = policy_input(body(json!({
            "name": "p", "enabled": true, "cedar": "permit(principal,"
        })))
        .expect_err("refused");

        assert_eq!(status_of(error), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn an_unknown_field_is_not_accepted() {
        let parsed = serde_json::from_value::<AgentPolicyRequest>(json!({
            "name": "p", "enabled": true, "cedar": "x", "owner": "me"
        }));

        assert!(parsed.is_err());
    }

    #[test]
    fn a_decision_filter_that_is_not_one_is_a_400() {
        let error = decision_query(AgentDecisionsQuery {
            from: None,
            to: None,
            decision: Some("maybe".to_string()),
            tool: None,
            subject: None,
            agent: None,
            limit: None,
        })
        .expect_err("refused");

        assert_eq!(status_of(error), StatusCode::BAD_REQUEST);
    }
}
