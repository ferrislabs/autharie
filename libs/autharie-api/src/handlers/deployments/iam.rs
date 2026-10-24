use autharie_auth::Identity;
use autharie_core::{
    agents::registry::SealingJwk,
    deployments::DeploymentId,
    iam::{
        Envelope, IamOperation,
        entities::{IamFetch, IamInput},
        ports::IamService,
    },
    organisation::OrganisationId,
};
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/iam/requests")]
pub struct IamRequestsRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/iam/requests/{request_id}"
)]
pub struct IamRequestRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub request_id: Uuid,
}

#[derive(Deserialize, ToSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum IamOperationBody {
    #[serde(rename = "applications.list")]
    ApplicationsList {
        #[schema(minimum = 1, example = 1)]
        page: Option<u32>,
        #[schema(minimum = 1, maximum = 50, example = 25)]
        limit: Option<u32>,
        search: Option<String>,
    },
    #[serde(rename = "applications.get")]
    ApplicationsGet { id: Uuid },
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IamRequestBody {
    #[schema(example = "master")]
    pub realm: String,
    pub operation: IamOperationBody,
    pub sealing_key: SealingJwk,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct IamRequestAccepted {
    pub data: IamRequestData,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct IamRequestData {
    pub request_id: Uuid,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct IamPending {
    pub status: &'static str,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct IamSealedResult {
    pub envelope: Envelope,
}

#[derive(Serialize, ToSchema, PartialEq)]
#[serde(untagged)]
pub enum IamResultResponse {
    Ready { data: IamSealedResult },
    Pending { data: IamPending },
}

fn refused(rejection: JsonRejection) -> ApiError {
    ApiError::BadRequest {
        reason: rejection.body_text(),
    }
}

fn operation_from(body: IamOperationBody) -> Result<IamOperation, ApiError> {
    match body {
        IamOperationBody::ApplicationsList {
            page,
            limit,
            search,
        } => IamOperation::applications_list(page, limit, search.as_deref()).map_err(|error| {
            ApiError::BadRequest {
                reason: error.to_string(),
            }
        }),
        IamOperationBody::ApplicationsGet { id } => Ok(IamOperation::ApplicationsGet { id }),
    }
}

fn input_from(body: IamRequestBody) -> Result<IamInput, ApiError> {
    Ok(IamInput::parse(
        &body.realm,
        operation_from(body.operation)?,
        body.sealing_key,
    )?)
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/iam/requests",
    summary = "ask the data plane for one operation on one realm",
    tag = "iam",
    description = "Records one named operation (`applications.list`, `applications.get`) on one realm of a FerrisKey deployment and hands it to the data plane, which runs it and reports the answer sealed to `sealing_key`, a public RSA JWK (RSA-OAEP-256, at least 2048 bits) the browser generated for this request. Poll `GET /iam/requests/{request_id}` for the answer. The request is audited (who, operation type, realm, request id), never its parameters or its result. A deployment holds at most 10 requests not yet collected or expired. Requires VIEW_INSTANCES.",
    params(IamRequestsRoute),
    request_body = IamRequestBody,
    responses(
        (status = 202, description = "Accepted", body = IamRequestAccepted),
        (status = 400, description = "An unknown operation, a field, the realm or the sealing key is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 429, description = "The deployment already holds 10 requests not yet collected or expired", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_iam_request_handler(
    IamRequestsRoute {
        organisation_id,
        deployment_id,
    }: IamRequestsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    body: Result<Json<IamRequestBody>, JsonRejection>,
) -> Result<Response<IamRequestAccepted>, ApiError> {
    let Json(body) = body.map_err(refused)?;
    let input = input_from(body)?;

    let request_id = state
        .service
        .ask_iam(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            input,
        )
        .await?;

    Ok(Response::Accepted(IamRequestAccepted {
        data: IamRequestData { request_id },
    }))
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/iam/requests/{request_id}",
    summary = "collect the sealed answer of an IAM request",
    tag = "iam",
    description = "202 while the data plane has not answered; 200 with the envelope the first time it is ready, then 410; 404 when the request is unknown, was asked by somebody else, or expired (5 minutes after it was asked); 422 with the data plane's message when the request failed. The envelope is `{ v, alg, ek, iv, ct }`: an AES-256-GCM ciphertext (AAD = the request id) whose key is sealed with RSA-OAEP-256 to the key sent with the request, base64url without padding. Requires VIEW_INSTANCES.",
    params(IamRequestRoute),
    responses(
        (status = 200, description = "The sealed answer", body = IamResultResponse),
        (status = 202, description = "Not ready yet", body = IamResultResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "Unknown or expired request", body = ApiError),
        (status = 410, description = "Already collected", body = ApiError),
        (status = 422, description = "The request failed; `message` says why", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_iam_request_handler(
    IamRequestRoute {
        organisation_id,
        deployment_id,
        request_id,
    }: IamRequestRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<IamResultResponse>, ApiError> {
    let fetched = state
        .service
        .poll_iam(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            request_id,
        )
        .await?;

    Ok(match fetched {
        IamFetch::Ready(envelope) => Response::OK(IamResultResponse::Ready {
            data: IamSealedResult { envelope },
        }),
        IamFetch::Pending => Response::Accepted(IamResultResponse::Pending {
            data: IamPending { status: "pending" },
        }),
    })
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, response::IntoResponse};
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::{Value, json};

    use super::*;

    fn status_of(error: ApiError) -> StatusCode {
        error.into_response().status()
    }

    fn key() -> Value {
        let mut modulus = vec![0xffu8; 256];
        modulus[255] = 0xfd;
        json!({
            "kty": "RSA",
            "alg": "RSA-OAEP-256",
            "e": "AQAB",
            "n": URL_SAFE_NO_PAD.encode(modulus),
        })
    }

    fn body(operation: Value) -> Value {
        json!({ "realm": "master", "operation": operation, "sealing_key": key() })
    }

    fn read(value: Value) -> Result<IamInput, ApiError> {
        input_from(serde_json::from_value(value).expect("a body"))
    }

    #[test]
    fn a_list_without_parameters_is_read_with_its_defaults() {
        let input = read(body(json!({ "type": "applications.list" }))).expect("accepted");

        assert_eq!(
            input.operation,
            IamOperation::applications_list(None, None, None).unwrap()
        );
        assert_eq!(input.realm.as_str(), "master");
    }

    #[test]
    fn a_list_carries_its_page_limit_and_search() {
        let input = read(body(json!({
            "type": "applications.list", "page": 2, "limit": 50, "search": " web "
        })))
        .expect("accepted");

        assert_eq!(
            input.operation,
            IamOperation::applications_list(Some(2), Some(50), Some("web")).unwrap()
        );
    }

    #[test]
    fn a_get_names_one_application() {
        let id = Uuid::new_v4();
        let input = read(body(json!({ "type": "applications.get", "id": id }))).expect("accepted");

        assert_eq!(input.operation, IamOperation::ApplicationsGet { id });
    }

    #[test]
    fn spec_iam_5_an_operation_that_does_not_exist_is_not_read() {
        for operation in [
            json!({ "type": "applications.delete", "id": Uuid::new_v4() }),
            json!({ "type": "GET /realms/master/clients" }),
            json!({ "type": "applications.get" }),
            json!({ "type": "applications.get", "id": "not-a-uuid" }),
            json!({ "type": "applications.list", "path": "/admin" }),
            json!({ "type": "applications.list", "id": Uuid::new_v4() }),
            json!({}),
        ] {
            assert!(
                serde_json::from_value::<IamRequestBody>(body(operation.clone())).is_err(),
                "{operation}"
            );
        }
    }

    #[test]
    fn spec_iam_5_a_page_or_limit_out_of_range_is_a_400() {
        for operation in [
            json!({ "type": "applications.list", "page": 0 }),
            json!({ "type": "applications.list", "limit": 0 }),
            json!({ "type": "applications.list", "limit": 51 }),
            json!({ "type": "applications.list", "search": "a".repeat(101) }),
        ] {
            assert_eq!(
                status_of(read(body(operation)).expect_err("refused")),
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[test]
    fn spec_iam_6_a_weak_or_wrong_key_is_a_400() {
        let operation = json!({ "type": "applications.list" });
        let mut weak = body(operation.clone());
        weak["sealing_key"]["n"] = json!(URL_SAFE_NO_PAD.encode(vec![0xffu8; 255]));
        let mut ec = body(operation.clone());
        ec["sealing_key"] = json!({ "kty": "EC", "alg": "ES256", "n": "AA", "e": "AQAB" });
        let mut private = body(operation);
        private["sealing_key"]["d"] = json!("AAAA");

        for bad in [weak, ec, private] {
            assert_eq!(
                status_of(read(bad).expect_err("refused")),
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[test]
    fn a_bad_realm_is_a_400() {
        let mut bad = body(json!({ "type": "applications.list" }));
        bad["realm"] = json!("a/b");

        assert_eq!(
            status_of(read(bad).expect_err("refused")),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn an_unknown_field_is_not_read() {
        let mut extra = body(json!({ "type": "applications.list" }));
        extra["path"] = json!("/admin/realms");

        assert!(serde_json::from_value::<IamRequestBody>(extra).is_err());
    }

    #[test]
    fn a_pending_answer_says_so_under_data() {
        let pending = serde_json::to_value(IamResultResponse::Pending {
            data: IamPending { status: "pending" },
        })
        .unwrap();

        assert_eq!(pending, json!({ "data": { "status": "pending" } }));
    }
}
