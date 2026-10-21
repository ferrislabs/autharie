use axum::{Router, middleware::from_fn_with_state};
use axum_extra::routing::RouterExt;
use utoipa::OpenApi;

use crate::{
    handlers::billing::{
        account::{
            __path_get_billing_account_handler, __path_put_billing_account_handler,
            get_billing_account_handler, put_billing_account_handler,
        },
        payment_method_session::{
            __path_payment_method_session_handler, payment_method_session_handler,
        },
        provider_events::{__path_provider_events_handler, provider_events_handler},
        statements::{
            __path_get_statement_handler, __path_list_statements_handler, get_statement_handler,
            list_statements_handler,
        },
        tiers::{__path_list_tiers_handler, list_tiers_handler},
    },
    router::service_auth_middleware,
    state::AppState,
};

pub mod account;
pub mod payment_method_session;
pub mod provider_events;
pub mod statements;
pub mod tiers;

#[derive(OpenApi)]
#[openapi(
    paths(provider_events_handler),
    tags(
        (name = "billing", description = "Billing endpoints.")
    )
)]
pub struct BillingApiDoc;

#[derive(OpenApi)]
#[openapi(paths(
    list_tiers_handler,
    get_billing_account_handler,
    put_billing_account_handler,
    payment_method_session_handler,
    list_statements_handler,
    get_statement_handler
))]
pub struct OrganisationBillingApiDoc;

fn routes() -> Router<AppState> {
    Router::new().typed_post(provider_events_handler)
}

fn organisation_routes() -> Router<AppState> {
    Router::new()
        .typed_get(list_tiers_handler)
        .typed_get(get_billing_account_handler)
        .typed_put(put_billing_account_handler)
        .typed_post(payment_method_session_handler)
        .typed_get(list_statements_handler)
        .typed_get(get_statement_handler)
}

pub fn billing_routes(app_state: AppState) -> Router<AppState> {
    routes()
        .merge(organisation_routes().layer(from_fn_with_state(app_state, service_auth_middleware)))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::{billing_routes, routes};
    use crate::{handlers::deployments::deployment_routes, test_helpers::app_state};

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

    #[tokio::test]
    async fn every_route_of_an_organisation_asks_for_a_token() {
        let state = app_state();
        let organisation = "00000000-0000-0000-0000-000000000001";
        let deployment = "00000000-0000-0000-0000-000000000002";
        let routes = [
            (
                "GET",
                format!("/organisations/{organisation}/billing/tiers"),
            ),
            (
                "GET",
                format!("/organisations/{organisation}/billing/account"),
            ),
            (
                "PUT",
                format!("/organisations/{organisation}/billing/account"),
            ),
            (
                "POST",
                format!("/organisations/{organisation}/billing/payment-method-session"),
            ),
            (
                "GET",
                format!("/organisations/{organisation}/billing/statements"),
            ),
            (
                "GET",
                format!("/organisations/{organisation}/billing/statements/2026-11"),
            ),
            (
                "GET",
                format!("/organisations/{organisation}/deployments/{deployment}/subscription"),
            ),
            (
                "PUT",
                format!("/organisations/{organisation}/deployments/{deployment}/subscription"),
            ),
            (
                "GET",
                format!("/organisations/{organisation}/deployments/{deployment}/features"),
            ),
        ];
        let router = billing_routes(state.clone())
            .merge(deployment_routes(state.clone()))
            .with_state(state);

        for (method, uri) in routes {
            let answer = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(&uri)
                        .body(Body::empty())
                        .expect("a request"),
                )
                .await
                .expect("the router answered");

            assert_eq!(answer.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn the_provider_still_reports_without_a_token_through_the_merged_routes() {
        let state = app_state();
        let router = billing_routes(state.clone()).with_state(state);

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
