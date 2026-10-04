use autharie_auth::Identity;
use autharie_core::{
    CoreError,
    cloud_credentials::{CloudProviderService, ProfileSpec},
    dataplane::{credential::CloudCredentialId, value_objects::Region},
    deployments::{
        Deployment, DeploymentKind, DeploymentName,
        commands::CreateDeploymentCommand,
        distribution::{Distribution, DistributionError},
        environment::Environment,
        ports::DeploymentService,
    },
    offers::Offer,
    user::UserId,
    version::Version,
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    errors::ApiError, handlers::cloud_credentials::profile::ClusterProfileRequest,
    response::Response, state::AppState,
};

#[derive(Deserialize, ToSchema)]
pub struct CreateDeploymentRequest {
    pub name: String,
    pub kind: String,
    pub version: String,

    /// `production`, `staging` or `development`.
    pub environment: String,

    /// Refused, and present only so that it is.
    ///
    /// Where a deployment runs is the installation's to decide: a region is
    /// the fleet seen from outside, and choosing one is choosing which
    /// cluster serves you. Dropping the field would let a caller that still
    /// sends one believe it was honoured, and a request that asked for one
    /// country and was quietly given another is the worst way to find out.
    ///
    /// It comes back the day regions are a product surface rather than a view
    /// of the fleet, and it comes back as a promise about where data lives.
    #[serde(default)]
    pub region: Option<String>,

    /// What they are buying: `sandbox`, `standard`, `scale` or `private`.
    ///
    /// The size and whether the deployment gets a cluster of its own are read
    /// from it. There is no field for either, deliberately: a request that
    /// could send both could contradict the offer it named, and nothing would
    /// be able to say which half was meant.
    pub offer: String,

    #[serde(default)]
    pub distribution: Option<DistributionRequest>,
}

#[derive(Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DistributionRequest {
    Shared,
    SelfHosted,
    CustomerCloud {
        credential_id: Uuid,
        region: String,
        profile: ClusterProfileRequest,
    },
}

enum RequestedDistribution {
    Shared,
    SelfHosted,
    CustomerCloud {
        credential_id: CloudCredentialId,
        region: Region,
        spec: ProfileSpec,
    },
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct CreateDeploymentResponse {
    data: Deployment,
}

struct ParsedCreateDeploymentRequest {
    name: String,
    kind: DeploymentKind,
    version: Version,
    environment: Environment,
    region: Region,
    offer: Offer,
    distribution: RequestedDistribution,
}

/// What a request naming a region is told.
const REGION_IS_NOT_YOURS: &str = "where a deployment runs is decided by the platform;                                    remove `region` from the request";

impl ParsedCreateDeploymentRequest {
    fn parse(request: CreateDeploymentRequest, default_region: &str) -> Result<Self, ApiError> {
        let refused = |reason: String| ApiError::BadRequest { reason };

        let kind =
            DeploymentKind::try_from(request.kind.as_str()).map_err(|e| refused(e.to_string()))?;

        let version = Version::parse(&request.version).map_err(|e| refused(e.to_string()))?;

        let environment = request
            .environment
            .parse::<Environment>()
            .map_err(|e| refused(e.to_string()))?;

        let offer = request
            .offer
            .parse::<Offer>()
            .map_err(|e| refused(e.to_string()))?;

        // Refused rather than ignored. A caller that named a region and was
        // silently given another would find out from where their data is.
        if request.region.is_some() {
            return Err(refused(REGION_IS_NOT_YOURS.to_string()));
        }

        let (distribution, region) = match request.distribution {
            None | Some(DistributionRequest::Shared) => (
                RequestedDistribution::Shared,
                Region::new(default_region.to_string()),
            ),
            Some(DistributionRequest::SelfHosted) => (
                RequestedDistribution::SelfHosted,
                Region::new(default_region.to_string()),
            ),
            Some(DistributionRequest::CustomerCloud {
                credential_id,
                region,
                profile,
            }) => {
                if kind != DeploymentKind::Ferriskey {
                    return Err(ApiError::from(CoreError::from(
                        DistributionError::NotAllowedForKind { kind },
                    )));
                }

                if region.trim().is_empty() {
                    return Err(refused("a customer cloud region is required".to_string()));
                }

                let region = Region::new(region);
                (
                    RequestedDistribution::CustomerCloud {
                        credential_id: CloudCredentialId(credential_id),
                        region: region.clone(),
                        spec: profile.into_spec()?,
                    },
                    region,
                )
            }
        };

        Ok(Self {
            name: request.name,
            kind,
            version,
            environment,
            region,
            offer,
            distribution,
        })
    }
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments")]
pub struct CreateDeploymentRoute {
    pub organisation_id: Uuid,
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments",
    summary = "create deployment",
    tag = "deployments",
    description = "Create a deployment within the specified organisation. Where it runs is \
                   the installation's decision, not the caller's: a request that names a region \
                   is refused rather than having it quietly substituted.",
    request_body = CreateDeploymentRequest,
    params(CreateDeploymentRoute),
    responses(
        (status = 201, description = "Deployment created successfully", body = CreateDeploymentResponse),
        (status = 400, description = "Invalid request data", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 422, description = "The distribution or the cluster profile is refused", body = ApiError),
        (status = 502, description = "The provider refused or could not serve the request", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn create_deployment_handler(
    CreateDeploymentRoute { organisation_id }: CreateDeploymentRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<CreateDeploymentRequest>,
) -> Result<Response<CreateDeploymentResponse>, ApiError> {
    let organisation_id = organisation_id.into();
    let created_by =
        identity
            .id()
            .parse::<UserId>()
            .map_err(|e| ApiError::InternalServerError {
                reason: e.to_string(),
            })?;
    let parsed =
        ParsedCreateDeploymentRequest::parse(request, &state.args.dataplane.default_region)?;

    let distribution = match parsed.distribution {
        RequestedDistribution::Shared => Distribution::Shared,
        RequestedDistribution::SelfHosted => Distribution::SelfHosted,
        RequestedDistribution::CustomerCloud {
            credential_id,
            region,
            spec,
        } => {
            state
                .service
                .resolve_customer_cloud(
                    identity.clone(),
                    organisation_id,
                    credential_id,
                    region,
                    spec,
                )
                .await?
        }
    };

    let command = CreateDeploymentCommand::new(
        organisation_id,
        DeploymentName(parsed.name),
        parsed.kind,
        parsed.version,
        created_by,
        parsed.environment,
        parsed.region,
        parsed.offer,
    )
    .with_distribution(distribution)
    .map_err(CoreError::from)?;

    let deployment = state.service.create_deployment(identity, command).await?;

    // Best-effort and never awaited: a deployment is created whether or not
    // it can be given a DNS record yet, and a data plane with no address yet
    // is caught up by the reconciliation sweep instead.
    tokio::spawn(crate::dns::reconcile_deployment(
        state.clone(),
        deployment.clone(),
    ));

    Ok(Response::Created(CreateDeploymentResponse {
        data: deployment,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_deployment_rejects_invalid_kind() {
        assert!(matches!(
            parse(CreateDeploymentRequest {
                kind: "invalid".to_string(),
                ..a_request()
            }),
            Err(ApiError::BadRequest { .. })
        ));
    }

    #[tokio::test]
    async fn create_deployment_rejects_an_environment_nobody_named() {
        assert!(matches!(
            parse(CreateDeploymentRequest {
                environment: "preprod".to_string(),
                ..a_request()
            }),
            Err(ApiError::BadRequest { .. })
        ));
    }

    #[tokio::test]
    async fn create_deployment_rejects_an_offer_nobody_sells() {
        assert!(matches!(
            parse(CreateDeploymentRequest {
                offer: "enterprise-plus".to_string(),
                ..a_request()
            }),
            Err(ApiError::BadRequest { .. })
        ));
    }

    /// A region is the fleet seen from outside, and choosing one is choosing
    /// which cluster serves you. Refused rather than dropped: a caller that
    /// asked for one country and was quietly given another would find out
    /// from where their data is.
    #[tokio::test]
    async fn a_request_that_names_a_region_is_refused_rather_than_ignored() {
        let Err(ApiError::BadRequest { reason }) = parse(CreateDeploymentRequest {
            region: Some("fr-par".to_string()),
            ..a_request()
        }) else {
            panic!("a caller placed their own deployment");
        };
        assert!(
            reason.contains("region"),
            "and it says which field: {reason}"
        );
    }

    #[tokio::test]
    async fn a_request_that_names_none_runs_where_the_installation_says() {
        assert_eq!(
            parse(a_request()).expect("a valid request").region.as_str(),
            "somewhere-else"
        );
    }

    /// What the offer carries is not something the request can contradict,
    /// because there is nowhere to put a contradiction.
    #[tokio::test]
    async fn the_size_and_the_isolation_are_read_from_the_offer() {
        let parsed = parse(CreateDeploymentRequest {
            offer: "private".to_string(),
            ..a_request()
        })
        .expect("a valid request");

        assert_eq!(parsed.offer, Offer::Private);
        assert_eq!(
            parsed.offer.mode(),
            autharie_core::dataplane::value_objects::DataPlaneMode::Dedicated
        );
    }

    #[tokio::test]
    async fn a_keycloak_deployment_cannot_ask_for_the_customer_cloud_spec_ccp_9() {
        let Err(refused) = parse(CreateDeploymentRequest {
            kind: "keycloak".to_string(),
            distribution: customer_cloud("fr-par"),
            ..a_request()
        }) else {
            panic!("a keycloak deployment was placed in the customer's cloud");
        };

        assert!(
            matches!(refused, ApiError::Unprocessable { .. }),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn a_ferriskey_deployment_runs_in_the_region_of_its_cluster() {
        let parsed = parse(CreateDeploymentRequest {
            kind: "ferriskey".to_string(),
            distribution: customer_cloud("fr-par"),
            ..a_request()
        })
        .expect("a valid request");

        assert_eq!(parsed.region.as_str(), "fr-par");
        assert!(matches!(
            parsed.distribution,
            RequestedDistribution::CustomerCloud { .. }
        ));
    }

    #[tokio::test]
    async fn a_request_without_a_distribution_is_shared() {
        let parsed = parse(a_request()).expect("a valid request");

        assert!(matches!(parsed.distribution, RequestedDistribution::Shared));
    }

    #[test]
    fn the_distribution_wire_shape_is_tagged() {
        let shared: DistributionRequest =
            serde_json::from_str(r#"{"type":"shared"}"#).expect("shared");
        let hosted: DistributionRequest =
            serde_json::from_str(r#"{"type":"self_hosted"}"#).expect("self hosted");
        let cloud: DistributionRequest = serde_json::from_str(
            r#"{"type":"customer_cloud","credential_id":"00000000-0000-0000-0000-000000000000","region":"fr-par","profile":{"mode":"dev","control_plane_id":"c","node_type":"n","min_nodes":1,"max_nodes":1,"replication":1}}"#,
        )
        .expect("customer cloud");

        assert!(matches!(shared, DistributionRequest::Shared));
        assert!(matches!(hosted, DistributionRequest::SelfHosted));
        assert!(matches!(cloud, DistributionRequest::CustomerCloud { .. }));
    }

    #[tokio::test]
    async fn a_customer_cloud_without_a_region_is_a_bad_request() {
        let Err(refused) = parse(CreateDeploymentRequest {
            kind: "ferriskey".to_string(),
            distribution: customer_cloud("  "),
            ..a_request()
        }) else {
            panic!("accepted");
        };

        assert!(matches!(refused, ApiError::BadRequest { .. }));
    }

    fn a_request() -> CreateDeploymentRequest {
        CreateDeploymentRequest {
            name: "deployment".to_string(),
            kind: "keycloak".to_string(),
            version: "1.0.0".to_string(),
            environment: "production".to_string(),
            region: None,
            offer: "standard".to_string(),
            distribution: None,
        }
    }

    fn customer_cloud(region: &str) -> Option<DistributionRequest> {
        Some(DistributionRequest::CustomerCloud {
            credential_id: Uuid::new_v4(),
            region: region.to_string(),
            profile: ClusterProfileRequest {
                mode: autharie_core::dataplane::cluster_profile::ClusterMode::Dev,
                control_plane_id: "mutualized".to_string(),
                node_type: "small".to_string(),
                min_nodes: 1,
                max_nodes: 1,
                replication: 1,
            },
        })
    }

    fn parse(request: CreateDeploymentRequest) -> Result<ParsedCreateDeploymentRequest, ApiError> {
        ParsedCreateDeploymentRequest::parse(request, "somewhere-else")
    }
}
