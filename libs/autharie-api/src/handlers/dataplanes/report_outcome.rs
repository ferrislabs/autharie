use autharie_auth::Identity;
use autharie_core::{
    agents::{AgentError, commands::AgentsOutcome, ports::AgentOutcomeService},
    dataplane::{credential::SecretString, ports::DataPlaneService, value_objects::DataPlaneId},
    deployments::{DeploymentId, commands::ReportDeploymentOutcomeCommand},
    webhooks::{
        RequestPhase, WebhookId,
        commands::{RequestReport, SecretReport, WebhookReport, WebhooksOutcome},
        ports::WebhookOutcomeService,
    },
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/dataplanes/{dataplane_id}/deployments/{deployment_id}/outcome")]
pub struct ReportOutcomeRoute {
    pub dataplane_id: DataPlaneId,
    pub deployment_id: DeploymentId,
}

#[derive(Deserialize, ToSchema)]
pub struct WebhookStateReport {
    pub id: Uuid,
    pub ferriskey_id: Option<String>,
    /// `Pending`, `Active` or `Failed`.
    pub phase: String,
    pub message: Option<String>,
    #[serde(default)]
    pub secret_version: u64,
}

#[derive(Deserialize, ToSchema)]
pub struct WebhookSecretReport {
    pub id: Uuid,
    pub version: u64,
    pub secret: String,
}

#[derive(Deserialize, ToSchema)]
pub struct WebhookRequestReport {
    pub id: Uuid,
    /// `Pending`, `Done` or `Failed`.
    pub phase: String,
    pub message: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct AgentPolicyReport {
    pub id: Uuid,
    /// `Applied`, `Invalid` or `Failed`.
    pub phase: String,
    pub message: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct ReportOutcomeRequest {
    /// `webhooks` for a report of a deployment's webhooks, `deployment.agents`
    /// for a report of its agents; absent for the outcome of the deployment
    /// itself.
    #[serde(default)]
    pub kind: Option<String>,

    /// The revision of the `deployment.webhooks` or `deployment.agents` action
    /// this report reflects. Required with either kind.
    #[serde(default)]
    pub revision: Option<u64>,

    /// Must be the deployment of the route when sent.
    #[serde(default)]
    pub deployment_id: Option<Uuid>,

    /// `Applying`, `Ready` or `Failed`. Only with `kind: deployment.agents`.
    #[serde(default)]
    pub phase: Option<String>,

    /// Whether the gateway's client exists in the realm. Only with `kind: deployment.agents`.
    #[serde(default)]
    pub gateway_client_ready: bool,

    /// Where the deployment's MCP endpoint answers. Only with `kind: deployment.agents`.
    #[serde(default)]
    pub mcp_url: Option<String>,

    /// Why the agents are not ready, when the data plane knows. Only with `kind: deployment.agents`.
    #[serde(default)]
    pub message: Option<String>,

    /// What became of each enabled policy. Only with `kind: deployment.agents`.
    #[serde(default)]
    pub policies: Vec<AgentPolicyReport>,

    /// What the data plane observed: `running`, `failed` or `deleted`.
    /// Not sent with `kind: webhooks` or `kind: deployment.agents`.
    ///
    /// All three are things a component saw rather than inferred -- the
    /// operator wrote `Running`/`ready` or `Failed` onto the resource, and
    /// deletion is reported by whatever removed it. None of them is a timeout,
    /// which is why none needs a policy about when to give up.
    #[serde(default)]
    pub outcome: String,

    #[serde(default)]
    pub webhooks: Vec<WebhookStateReport>,

    /// Only secret versions the control plane has not been given yet.
    #[serde(default)]
    pub secrets: Vec<WebhookSecretReport>,

    #[serde(default)]
    pub realms: Vec<String>,

    #[serde(default)]
    pub requests: Vec<WebhookRequestReport>,

    /// The version the data plane sees running, when it reports one.
    ///
    /// Optional because it is new: a data plane built before this field sends
    /// nothing, and refusing its report would have it retry something that can
    /// never succeed. It is what separates an upgrade that landed from one
    /// that has not started, since an instance answers on its old version
    /// until the rollout replaces it.
    #[serde(default)]
    pub version: Option<String>,

    /// Why the data plane gave up, sent with `failed` when it knows.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct ReportOutcomeResponseData {
    /// `false` when the report changed nothing -- an unknown deployment, or one
    /// whose state does not accept this outcome.
    ///
    /// Not an error: reports are at-least-once, so a redelivered one is
    /// expected and must not be answered with a failure that makes a data plane
    /// retry something already recorded.
    pub recorded: bool,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct ReportOutcomeResponse {
    data: ReportOutcomeResponseData,
}

fn webhooks_outcome(
    deployment_id: DeploymentId,
    request: ReportOutcomeRequest,
) -> Result<WebhooksOutcome, ApiError> {
    let bad = |error: autharie_core::webhooks::WebhookError| ApiError::BadRequest {
        reason: error.to_string(),
    };

    if request
        .deployment_id
        .is_some_and(|reported| reported != deployment_id.0)
    {
        return Err(ApiError::BadRequest {
            reason: "the report names another deployment than the route".to_string(),
        });
    }

    let revision = request.revision.ok_or_else(|| ApiError::BadRequest {
        reason: "a webhooks report needs the revision it reflects".to_string(),
    })?;

    Ok(WebhooksOutcome {
        revision,
        webhooks: request
            .webhooks
            .into_iter()
            .map(|report| {
                WebhookReport::parse(
                    report.id,
                    &report.phase,
                    report.ferriskey_id,
                    report.message,
                )
            })
            .collect::<Result<_, _>>()
            .map_err(bad)?,
        secrets: request
            .secrets
            .into_iter()
            .map(|report| SecretReport {
                id: WebhookId(report.id),
                version: report.version,
                secret: SecretString::new(report.secret),
            })
            .collect(),
        realms: request.realms,
        requests: request
            .requests
            .into_iter()
            .map(|report| {
                Ok(RequestReport {
                    id: report.id,
                    phase: RequestPhase::from_report(&report.phase)?,
                    message: report.message,
                })
            })
            .collect::<Result<_, _>>()
            .map_err(bad)?,
    })
}

fn agents_outcome(
    deployment_id: DeploymentId,
    request: ReportOutcomeRequest,
) -> Result<AgentsOutcome, ApiError> {
    if request
        .deployment_id
        .is_some_and(|reported| reported != deployment_id.0)
    {
        return Err(ApiError::BadRequest {
            reason: "the report names another deployment than the route".to_string(),
        });
    }

    let revision = request.revision.ok_or_else(|| ApiError::BadRequest {
        reason: "an agents report needs the revision it reflects".to_string(),
    })?;
    let phase = request.phase.ok_or_else(|| ApiError::BadRequest {
        reason: "an agents report needs a phase".to_string(),
    })?;

    AgentsOutcome::parse(
        revision,
        &phase,
        request.gateway_client_ready,
        request.mcp_url,
        request.message,
        request
            .policies
            .into_iter()
            .map(|report| (report.id, report.phase, report.message))
            .collect(),
    )
    .map_err(|error: AgentError| ApiError::BadRequest {
        reason: error.to_string(),
    })
}

#[utoipa::path(
    post,
    path = "/{dataplane_id}/deployments/{deployment_id}/outcome",
    summary = "report what a data plane did with a deployment",
    tag = "dataplanes",
    description = "Records an outcome the data plane observed directly. This is the only \
                   path by which the control plane learns what happened to a deployment \
                   after it was handed over. With `kind: webhooks` it records the state of \
                   the deployment's webhooks, new signing secrets, its realms and the result \
                   of rotation and retry requests instead; with `kind: deployment.agents` it \
                   records the status of the deployment's agents and the phase of each policy. \
                   A report older than the one already applied changes nothing.",
    params(ReportOutcomeRoute),
    request_body = ReportOutcomeRequest,
    responses(
        (status = 200, description = "Outcome recorded", body = ReportOutcomeResponse),
        (status = 400, description = "Unknown outcome", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "Caller is not herald", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn report_outcome_handler(
    ReportOutcomeRoute {
        dataplane_id,
        deployment_id,
    }: ReportOutcomeRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<ReportOutcomeRequest>,
) -> Result<Response<ReportOutcomeResponse>, ApiError> {
    let recorded = match request.kind.as_deref() {
        None => {
            let command = ReportDeploymentOutcomeCommand::parse(
                dataplane_id,
                deployment_id,
                request.outcome.as_str(),
                request.version.as_deref(),
            )
            .map_err(|reason| ApiError::BadRequest { reason })?
            .with_reason(request.reason.as_deref());

            state.service.report_outcome(identity, command).await?
        }
        Some("webhooks") => {
            let outcome = webhooks_outcome(deployment_id, request)?;

            state
                .service
                .report_webhooks_outcome(identity, deployment_id, outcome)
                .await?
        }
        Some("deployment.agents") => {
            let outcome = agents_outcome(deployment_id, request)?;

            state
                .service
                .report_agents_outcome(identity, deployment_id, outcome)
                .await?
        }
        Some(other) => {
            return Err(ApiError::BadRequest {
                reason: format!("'{other}' is not a kind of outcome"),
            });
        }
    };

    Ok(Response::OK(ReportOutcomeResponse {
        data: ReportOutcomeResponseData { recorded },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An outcome the control plane does not know is a caller mistake, not a
    /// deployment that failed. Rejecting it stops a typo being recorded as
    /// something that happened.
    #[test]
    fn an_unknown_outcome_is_rejected() {
        let result = ReportDeploymentOutcomeCommand::parse(
            DataPlaneId(uuid::Uuid::new_v4()),
            DeploymentId(uuid::Uuid::new_v4()),
            "exploded",
            None,
        );

        assert!(result.is_err());
    }

    fn request(body: serde_json::Value) -> ReportOutcomeRequest {
        serde_json::from_value(body).expect("a request")
    }

    #[test]
    fn a_report_without_a_kind_is_the_deployments_own_outcome() {
        let body = request(serde_json::json!({ "outcome": "running", "version": "26.0.1" }));

        assert!(body.kind.is_none());
        assert_eq!(body.outcome, "running");
    }

    #[test]
    fn a_webhooks_report_is_read_with_its_secrets_and_requests() {
        let deployment = DeploymentId(Uuid::new_v4());
        let webhook = Uuid::new_v4();
        let outcome = webhooks_outcome(
            deployment,
            request(serde_json::json!({
                "deployment_id": deployment.0,
                "kind": "webhooks",
                "revision": 4,
                "webhooks": [{
                    "id": webhook, "ferriskey_id": "fk", "phase": "Active", "secret_version": 1
                }],
                "secrets": [{ "id": webhook, "version": 1, "secret": "whsec" }],
                "realms": ["master"],
                "requests": [{ "id": Uuid::new_v4(), "phase": "Done" }]
            })),
        )
        .expect("accepted");

        assert_eq!(outcome.revision, 4);
        assert_eq!(outcome.webhooks.len(), 1);
        assert_eq!(outcome.secrets[0].version, 1);
        assert_eq!(outcome.realms, vec!["master".to_string()]);
        assert_eq!(outcome.requests[0].phase, RequestPhase::Done);
    }

    #[test]
    fn a_webhooks_report_needs_a_revision_a_known_phase_and_its_own_deployment() {
        let deployment = DeploymentId(Uuid::new_v4());

        for body in [
            serde_json::json!({ "kind": "webhooks" }),
            serde_json::json!({ "kind": "webhooks", "revision": 1,
                "webhooks": [{ "id": Uuid::new_v4(), "phase": "Exploded" }] }),
            serde_json::json!({ "kind": "webhooks", "revision": 1,
                "requests": [{ "id": Uuid::new_v4(), "phase": "Exploded" }] }),
            serde_json::json!({ "kind": "webhooks", "revision": 1,
                "deployment_id": Uuid::new_v4() }),
        ] {
            assert!(matches!(
                webhooks_outcome(deployment, request(body)),
                Err(ApiError::BadRequest { .. })
            ),);
        }
    }

    #[test]
    fn every_outcome_the_data_plane_can_report_is_accepted() {
        for outcome in ["deleted", "running", "failed"] {
            assert!(
                ReportDeploymentOutcomeCommand::parse(
                    DataPlaneId(uuid::Uuid::new_v4()),
                    DeploymentId(uuid::Uuid::new_v4()),
                    outcome,
                    None,
                )
                .is_ok(),
                "{outcome}"
            );
        }
    }

    #[test]
    fn an_agents_report_is_read_as_the_data_plane_sends_it() {
        let deployment = DeploymentId(Uuid::new_v4());
        let policy = Uuid::new_v4();

        let outcome = agents_outcome(
            deployment,
            request(serde_json::json!({
                "deployment_id": deployment.0,
                "kind": "deployment.agents",
                "revision": 4,
                "phase": "Ready",
                "gateway_client_ready": true,
                "mcp_url": "https://auth.example.com/mcp",
                "policies": [
                    { "id": policy, "phase": "Invalid", "message": "unexpected token" }
                ]
            })),
        )
        .expect("accepted");

        assert_eq!(outcome.revision, 4);
        assert!(outcome.gateway_client_ready);
        assert_eq!(outcome.policies.len(), 1);
        assert_eq!(
            outcome.policies[0].message.as_deref(),
            Some("unexpected token")
        );
    }

    #[test]
    fn an_agents_report_needs_a_revision_a_known_phase_and_its_own_deployment() {
        let deployment = DeploymentId(Uuid::new_v4());

        for body in [
            serde_json::json!({ "kind": "deployment.agents", "phase": "Ready" }),
            serde_json::json!({ "kind": "deployment.agents", "revision": 1 }),
            serde_json::json!({ "kind": "deployment.agents", "revision": 1, "phase": "Exploded" }),
            serde_json::json!({ "kind": "deployment.agents", "revision": 1, "phase": "Ready",
                "policies": [{ "id": Uuid::new_v4(), "phase": "Pending" }] }),
            serde_json::json!({ "kind": "deployment.agents", "revision": 1, "phase": "Ready",
                "deployment_id": Uuid::new_v4() }),
        ] {
            assert!(matches!(
                agents_outcome(deployment, request(body)),
                Err(ApiError::BadRequest { .. })
            ));
        }
    }
}
