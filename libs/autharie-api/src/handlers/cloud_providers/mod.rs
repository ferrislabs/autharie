use axum::{Router, middleware::from_fn_with_state};
use axum_extra::routing::RouterExt;
use utoipa::OpenApi;

use crate::handlers::cloud_providers::list_cloud_providers::{
    __path_list_cloud_providers_handler, list_cloud_providers_handler,
};
use crate::{router::service_auth_middleware, state::AppState};

pub mod list_cloud_providers;

#[derive(OpenApi)]
#[openapi(paths(list_cloud_providers_handler))]
pub struct CloudProviderApiDoc;

pub fn cloud_providers_routes(app_state: AppState) -> Router<AppState> {
    Router::new()
        .typed_get(list_cloud_providers_handler)
        .layer(from_fn_with_state(
            app_state.clone(),
            service_auth_middleware,
        ))
}
