use axum::{Router, middleware::from_fn_with_state};
use axum_extra::routing::RouterExt;
use utoipa::OpenApi;

use crate::{router::service_auth_middleware, state::AppState};

pub mod delete_credential;
pub mod estimate_cluster_cost;
pub mod list_credentials;
pub mod list_offers;
pub mod profile;
pub mod register_credential;

use delete_credential::{__path_delete_cloud_credential_handler, delete_cloud_credential_handler};
use estimate_cluster_cost::{
    __path_estimate_cluster_profile_handler, estimate_cluster_profile_handler,
};
use list_credentials::{__path_list_cloud_credentials_handler, list_cloud_credentials_handler};
use list_offers::{__path_list_provider_offers_handler, list_provider_offers_handler};
use register_credential::{
    __path_register_cloud_credential_handler, register_cloud_credential_handler,
};

#[derive(OpenApi)]
#[openapi(
    paths(
        register_cloud_credential_handler,
        list_cloud_credentials_handler,
        delete_cloud_credential_handler,
        list_provider_offers_handler,
        estimate_cluster_profile_handler,
    ),
    tags(
        (name = "cloud-credentials", description = "Customer cloud provider credentials and cluster profiles scoped to organisations.")
    )
)]
pub struct CloudCredentialApiDoc;

pub fn cloud_credential_router() -> Router<AppState> {
    Router::new()
        .typed_post(register_cloud_credential_handler)
        .typed_get(list_cloud_credentials_handler)
        .typed_delete(delete_cloud_credential_handler)
        .typed_get(list_provider_offers_handler)
        .typed_post(estimate_cluster_profile_handler)
}

pub fn cloud_credential_routes(app_state: AppState) -> Router<AppState> {
    cloud_credential_router().layer(from_fn_with_state(app_state, service_auth_middleware))
}

#[cfg(test)]
mod tests {
    use super::cloud_credential_routes;
    use crate::test_helpers::app_state;

    #[tokio::test]
    async fn cloud_credential_routes_builds() {
        let _router = cloud_credential_routes(app_state());
    }
}
