use axum::Router;
use axum_extra::routing::RouterExt;
use utoipa::OpenApi;

use crate::{
    handlers::billing::provider_events::{__path_provider_events_handler, provider_events_handler},
    state::AppState,
};

pub mod provider_events;

#[derive(OpenApi)]
#[openapi(
    paths(provider_events_handler),
    tags(
        (name = "billing", description = "Billing endpoints.")
    )
)]
pub struct BillingApiDoc;

fn routes() -> Router<AppState> {
    Router::new().typed_post(provider_events_handler)
}

pub fn billing_routes(_app_state: AppState) -> Router<AppState> {
    routes()
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::{billing_routes, routes};
    use crate::test_helpers::app_state;

    #[tokio::test]
    async fn billing_routes_builds() {
        let state = app_state();
        let _router = billing_routes(state);
    }

    #[tokio::test]
    async fn the_provider_reports_without_a_token() {
        let router = routes().with_state(app_state());

        let answer = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/billing/provider-events")
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("the router answered");

        assert_ne!(answer.status(), StatusCode::UNAUTHORIZED);
        assert_ne!(answer.status(), StatusCode::NOT_FOUND);
        assert_ne!(answer.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
