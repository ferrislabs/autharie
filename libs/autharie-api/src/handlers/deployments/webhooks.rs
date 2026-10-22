use autharie_auth::Identity;
use autharie_core::{
    CoreError,
    deployments::DeploymentId,
    organisation::OrganisationId,
    webhooks::{
        DeliveryStatus, WebhookEventInfo, WebhookRequest, WebhookView,
        commands::{RawWebhookInput, WebhookInput},
        delivery::{DeliveryQuery, MAX_DELIVERIES, WebhookDelivery, WebhookDeliveryIndex},
        known_events,
        ports::WebhookService,
        value_objects::WebhookId,
    },
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use axum_extra::routing::TypedPath;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/webhooks")]
pub struct WebhooksRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}")]
pub struct WebhookRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub webhook_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}/secret/rotate"
)]
pub struct RotateSecretRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub webhook_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}/secret"
)]
pub struct WebhookSecretRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub webhook_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}/deliveries"
)]
pub struct DeliveriesRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub webhook_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}/deliveries/{delivery_id}/retry"
)]
pub struct RetryDeliveryRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub webhook_id: Uuid,
    pub delivery_id: String,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/webhook-realms")]
pub struct WebhookRealmsRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/webhook-events")]
pub struct WebhookEventsRoute;

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WebhookHeaderRequest {
    /// A header name. Case is kept as sent; two names differing only by case
    /// are the same header.
    pub name: String,
    /// The value. Left out on a change, the value already stored for this
    /// name is kept: values are never returned, so a form cannot send them
    /// back.
    pub value: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WebhookRequestBody {
    /// A realm of the deployment. `master` when left out.
    pub realm: Option<String>,
    pub name: String,
    pub description: Option<String>,
    /// An absolute `https://` URL.
    pub endpoint: String,
    /// FerrisKey event names, 1 to 50, no repeats. See `GET /webhook-events`.
    pub events: Vec<String>,
    /// Up to 10 custom headers sent with every delivery.
    #[serde(default)]
    pub headers: Vec<WebhookHeaderRequest>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct WebhookResponse {
    pub data: WebhookView,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct WebhooksResponse {
    pub data: Vec<WebhookView>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct WebhookSecretData {
    pub secret: String,
    pub version: u64,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct WebhookSecretResponse {
    pub data: WebhookSecretData,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct WebhookRequestResponse {
    pub data: WebhookRequest,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct WebhookDeliveriesResponse {
    pub data: Vec<WebhookDelivery>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct WebhookRealmsResponse {
    pub data: Vec<String>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct WebhookEventsResponse {
    pub data: Vec<WebhookEventInfo>,
}

#[derive(Serialize, PartialEq)]
pub struct WebhookDeleted {}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeliveriesQuery {
    /// One of `pending`, `delivering`, `succeeded`, `failed`.
    pub status: Option<String>,
    /// Start of the range, inclusive.
    pub from: Option<DateTime<Utc>>,
    /// End of the range, exclusive.
    pub until: Option<DateTime<Utc>>,
}

fn input_from(body: WebhookRequestBody) -> Result<WebhookInput, ApiError> {
    WebhookInput::parse(RawWebhookInput {
        realm: body.realm,
        name: body.name,
        description: body.description,
        endpoint: body.endpoint,
        events: body.events,
        headers: body
            .headers
            .into_iter()
            .map(|header| (header.name, header.value))
            .collect(),
    })
    .map_err(|error| ApiError::BadRequest {
        reason: error.to_string(),
    })
}

fn delivery_query(webhook_id: Uuid, query: DeliveriesQuery) -> Result<DeliveryQuery, ApiError> {
    let status = query
        .status
        .as_deref()
        .map(|status| {
            DeliveryStatus::parse(status).ok_or_else(|| ApiError::BadRequest {
                reason: format!("'{status}' is not a delivery status"),
            })
        })
        .transpose()?;

    if let (Some(from), Some(until)) = (query.from, query.until)
        && until <= from
    {
        return Err(ApiError::BadRequest {
            reason: "until must be after from".to_string(),
        });
    }

    Ok(DeliveryQuery {
        deployment_id: DeploymentId(Uuid::nil()),
        webhook_id: WebhookId(webhook_id),
        status,
        from: query.from,
        until: query.until,
        limit: MAX_DELIVERIES,
    })
}

struct NoDeliveries;

impl WebhookDeliveryIndex for NoDeliveries {
    async fn deliveries(
        &self,
        _: OrganisationId,
        _: DeliveryQuery,
    ) -> Result<Vec<WebhookDelivery>, CoreError> {
        Ok(vec![])
    }
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/webhooks",
    summary = "list a deployment's webhooks",
    tag = "webhooks",
    description = "Header values and signing secrets are never part of the answer; header names are. A webhook is `Pending` until the data plane reports it applied.",
    params(WebhooksRoute),
    responses(
        (status = 200, description = "The deployment's webhooks", body = WebhooksResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_webhooks_handler(
    WebhooksRoute {
        organisation_id,
        deployment_id,
    }: WebhooksRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<WebhooksResponse>, ApiError> {
    let data = state
        .service
        .list_webhooks(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
        )
        .await?;

    Ok(Response::OK(WebhooksResponse { data }))
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/webhooks",
    summary = "create a webhook on a deployment",
    tag = "webhooks",
    description = "Stores the webhook as `Pending` and asks the data plane to apply the deployment's whole set of webhooks. FerrisKey deployments only. A Free plan holds 5 webhooks per deployment, a paid plan 25.",
    params(WebhooksRoute),
    request_body = WebhookRequestBody,
    responses(
        (status = 201, description = "Created, and being applied", body = WebhookResponse),
        (status = 400, description = "The endpoint, an event, a header, the realm or a length is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one, or already holds as many webhooks as its plan allows", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_webhook_handler(
    WebhooksRoute {
        organisation_id,
        deployment_id,
    }: WebhooksRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<WebhookRequestBody>,
) -> Result<Response<WebhookResponse>, ApiError> {
    let input = input_from(body)?;

    let data = state
        .service
        .create_webhook(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            input,
        )
        .await?;

    Ok(Response::Created(WebhookResponse { data }))
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}",
    summary = "read one webhook",
    tag = "webhooks",
    description = "Includes the latest rotation and retry requests made on it and what the data plane answered.",
    params(WebhookRoute),
    responses(
        (status = 200, description = "The webhook", body = WebhookResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment or webhook", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_webhook_handler(
    WebhookRoute {
        organisation_id,
        deployment_id,
        webhook_id,
    }: WebhookRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<WebhookResponse>, ApiError> {
    let data = state
        .service
        .get_webhook(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            WebhookId(webhook_id),
        )
        .await?;

    Ok(Response::OK(WebhookResponse { data }))
}

#[utoipa::path(
    put,
    path = "/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}",
    summary = "replace a webhook",
    tag = "webhooks",
    description = "Replaces the webhook's settings, puts it back to `Pending` and asks the data plane to apply the deployment's whole set. A header sent without a value keeps the value stored for its name; a header left out is removed.",
    params(WebhookRoute),
    request_body = WebhookRequestBody,
    responses(
        (status = 200, description = "Stored, and being applied", body = WebhookResponse),
        (status = 400, description = "The endpoint, an event, a header, the realm or a length is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment or webhook", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_webhook_handler(
    WebhookRoute {
        organisation_id,
        deployment_id,
        webhook_id,
    }: WebhookRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<WebhookRequestBody>,
) -> Result<Response<WebhookResponse>, ApiError> {
    let input = input_from(body)?;

    let data = state
        .service
        .update_webhook(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            WebhookId(webhook_id),
            input,
        )
        .await?;

    Ok(Response::OK(WebhookResponse { data }))
}

#[utoipa::path(
    delete,
    path = "/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}",
    summary = "delete a webhook",
    tag = "webhooks",
    description = "Removes the webhook and asks the data plane to apply the deployment's remaining set, which removes it from FerrisKey.",
    params(WebhookRoute),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment or webhook", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_webhook_handler(
    WebhookRoute {
        organisation_id,
        deployment_id,
        webhook_id,
    }: WebhookRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<WebhookDeleted>, ApiError> {
    state
        .service
        .delete_webhook(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            WebhookId(webhook_id),
        )
        .await?;

    Ok(Response::NoContent)
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}/secret/rotate",
    summary = "rotate a webhook's signing secret",
    tag = "webhooks",
    description = "Asks the data plane to have FerrisKey generate a new secret. Answers 202 with the request just made; the new secret, with a higher version, becomes readable once the data plane reports it.",
    params(RotateSecretRoute),
    responses(
        (status = 202, description = "The request was made", body = WebhookRequestResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment or webhook", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn rotate_webhook_secret_handler(
    RotateSecretRoute {
        organisation_id,
        deployment_id,
        webhook_id,
    }: RotateSecretRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<WebhookRequestResponse>, ApiError> {
    let data = state
        .service
        .rotate_webhook_secret(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            WebhookId(webhook_id),
        )
        .await?;

    Ok(Response::Accepted(WebhookRequestResponse { data }))
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}/secret",
    summary = "reveal a webhook's signing secret",
    tag = "webhooks",
    description = "The only place a secret is returned. Every call is written to the organisation's audit trail as `deployment.webhook.secret_revealed`. Requires MANAGE_INSTANCES.",
    params(WebhookSecretRoute),
    responses(
        (status = 200, description = "The current secret and its version", body = WebhookSecretResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not manage this deployment", body = ApiError),
        (status = 404, description = "No such deployment or webhook, or no secret was received yet", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn reveal_webhook_secret_handler(
    WebhookSecretRoute {
        organisation_id,
        deployment_id,
        webhook_id,
    }: WebhookSecretRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<WebhookSecretResponse>, ApiError> {
    let revealed = state
        .service
        .reveal_webhook_secret(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            WebhookId(webhook_id),
        )
        .await?;

    Ok(Response::OK(WebhookSecretResponse {
        data: WebhookSecretData {
            secret: revealed.secret.expose().to_string(),
            version: revealed.version,
        },
    }))
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}/deliveries",
    summary = "list a webhook's recent deliveries",
    tag = "webhooks",
    description = "Read from the organisation's log index, where the data plane records them. Newest first, at most 100. Empty when the index holds none.",
    params(DeliveriesRoute, DeliveriesQuery),
    responses(
        (status = 200, description = "The deliveries", body = WebhookDeliveriesResponse),
        (status = 400, description = "The status or the range is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment or webhook", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_webhook_deliveries_handler(
    DeliveriesRoute {
        organisation_id,
        deployment_id,
        webhook_id,
    }: DeliveriesRoute,
    Query(query): Query<DeliveriesQuery>,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<WebhookDeliveriesResponse>, ApiError> {
    let query = delivery_query(webhook_id, query)?;
    let organisation_id = OrganisationId(organisation_id);
    let deployment_id = DeploymentId(deployment_id);

    let data = match state.quickwit_search.as_deref() {
        Some(index) => {
            state
                .service
                .webhook_deliveries(identity, organisation_id, deployment_id, query, index)
                .await?
        }
        None => {
            state
                .service
                .webhook_deliveries(
                    identity,
                    organisation_id,
                    deployment_id,
                    query,
                    NoDeliveries,
                )
                .await?
        }
    };

    Ok(Response::OK(WebhookDeliveriesResponse { data }))
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/webhooks/{webhook_id}/deliveries/{delivery_id}/retry",
    summary = "retry a delivery",
    tag = "webhooks",
    description = "Asks the data plane to have FerrisKey send the delivery again. Answers 202 with the request just made.",
    params(RetryDeliveryRoute),
    responses(
        (status = 202, description = "The request was made", body = WebhookRequestResponse),
        (status = 400, description = "The delivery id is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment or webhook", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn retry_webhook_delivery_handler(
    RetryDeliveryRoute {
        organisation_id,
        deployment_id,
        webhook_id,
        delivery_id,
    }: RetryDeliveryRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<WebhookRequestResponse>, ApiError> {
    let data = state
        .service
        .retry_webhook_delivery(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            WebhookId(webhook_id),
            delivery_id,
        )
        .await?;

    Ok(Response::Accepted(WebhookRequestResponse { data }))
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/webhook-realms",
    summary = "list the realms a webhook can be created in",
    tag = "webhooks",
    description = "The realms the data plane last reported, or `master` alone until it reports any.",
    params(WebhookRealmsRoute),
    responses(
        (status = 200, description = "The realm names", body = WebhookRealmsResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_webhook_realms_handler(
    WebhookRealmsRoute {
        organisation_id,
        deployment_id,
    }: WebhookRealmsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<WebhookRealmsResponse>, ApiError> {
    let data = state
        .service
        .webhook_realms(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
        )
        .await?;

    Ok(Response::OK(WebhookRealmsResponse { data }))
}

#[utoipa::path(
    get,
    path = "/webhook-events",
    summary = "list the events a webhook can listen to",
    tag = "webhooks",
    description = "FerrisKey's event names, each with the resource group it belongs to.",
    responses(
        (status = 200, description = "The known events", body = WebhookEventsResponse),
        (status = 401, description = "Unauthorized", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_webhook_events_handler(
    _route: WebhookEventsRoute,
) -> Response<WebhookEventsResponse> {
    Response::OK(WebhookEventsResponse {
        data: known_events(),
    })
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, response::IntoResponse};
    use serde_json::json;

    use super::*;

    fn body(value: serde_json::Value) -> WebhookRequestBody {
        serde_json::from_value(value).expect("a body")
    }

    fn status_of(error: ApiError) -> StatusCode {
        error.into_response().status()
    }

    #[test]
    fn a_valid_body_is_read() {
        let input = input_from(body(json!({
            "name": "orders",
            "endpoint": "https://example.com/hook",
            "events": ["user.created"],
            "headers": [{ "name": "x-api-key", "value": "k" }]
        })))
        .expect("accepted");

        assert_eq!(input.realm.as_str(), "master");
        assert_eq!(input.headers.len(), 1);
    }

    #[test]
    fn an_http_endpoint_is_a_400() {
        let error = input_from(body(json!({
            "name": "orders",
            "endpoint": "http://example.com/hook",
            "events": ["user.created"]
        })))
        .expect_err("refused");

        assert_eq!(status_of(error), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn an_unknown_event_is_a_400() {
        let error = input_from(body(json!({
            "name": "orders",
            "endpoint": "https://example.com/hook",
            "events": ["user.exploded"]
        })))
        .expect_err("refused");

        assert_eq!(status_of(error), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn unknown_fields_are_not_read() {
        assert!(
            serde_json::from_value::<WebhookRequestBody>(json!({
                "name": "orders",
                "endpoint": "https://example.com/hook",
                "events": ["user.created"],
                "secret": "mine"
            }))
            .is_err()
        );
    }

    #[test]
    fn a_delivery_query_reads_its_status_and_range() {
        let ok = delivery_query(
            Uuid::nil(),
            DeliveriesQuery {
                status: Some("failed".to_string()),
                from: None,
                until: None,
            },
        )
        .expect("accepted");
        assert_eq!(ok.status, Some(DeliveryStatus::Failed));
        assert_eq!(ok.limit, 100);

        let bad_status = delivery_query(
            Uuid::nil(),
            DeliveriesQuery {
                status: Some("lost".to_string()),
                from: None,
                until: None,
            },
        );
        assert!(matches!(bad_status, Err(ApiError::BadRequest { .. })));

        let now = Utc::now();
        let backwards = delivery_query(
            Uuid::nil(),
            DeliveriesQuery {
                status: None,
                from: Some(now),
                until: Some(now),
            },
        );
        assert!(matches!(backwards, Err(ApiError::BadRequest { .. })));
    }

    #[tokio::test]
    async fn the_events_come_with_their_group() {
        let Response::OK(events) = list_webhook_events_handler(WebhookEventsRoute).await else {
            panic!("expected a 200");
        };

        assert!(
            events
                .data
                .iter()
                .any(|event| event.name == "user.created" && event.group == "user")
        );
    }
}
