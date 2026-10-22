use std::net::{SocketAddr, ToSocketAddrs};

use autharie_auth::{Identity, Token};
use autharie_core::{
    CoreError,
    auth::ports::AuthService,
    organisation::{OrganisationId, ports::OrganisationMcpSwitch},
    user::{commands::CreateUserCommand, ports::UserService},
};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::{IntoResponse, Response},
};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::{
    args::LogArgs,
    errors::{ApiError, ApiErrorResponse},
};

pub mod action_stuck_signal;
pub mod args;
pub mod auth;
pub mod backup_signal;
pub mod billing;
pub mod billing_periods;
pub mod certificate;
pub mod delegated;
pub mod deployment_reachability;
pub mod dns;
pub mod drill;
pub mod drill_signal;
pub mod errors;
pub mod handlers;
pub mod heartbeat_signal;
pub mod keys;
pub mod objectstore;
pub mod openapi;
pub mod operators;
pub mod purge;
pub mod quickwit;
pub mod response;
pub mod router;
pub mod state;

#[cfg(test)]
pub(crate) mod test_helpers {
    use std::{sync::Arc, time::Duration};

    use autharie_auth::{Identity, User};
    use autharie_core::AutharieService;
    use sqlx::postgres::PgPoolOptions;

    use crate::{args, state::AppState};

    pub fn app_state() -> AppState {
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(50))
            .connect_lazy("postgres://user:pass@127.0.0.1:1/db")
            .expect("valid database url");

        AppState {
            args: Arc::new(args::Args::default()),
            service: AutharieService::new(pool),
            certificate_source: None,
            quickwit_search: None,
            quickwit_traces: None,
            billing: None,
        }
    }

    pub fn user_identity(id: &str) -> Identity {
        Identity::User(User {
            scopes: Vec::new(),
            act: None,
            id: id.to_string(),
            username: "user".to_string(),
            email: None,
            name: None,
            roles: vec![],
        })
    }
}

pub fn init_logger(args: &LogArgs) {
    let filter = EnvFilter::try_new(&args.filter).unwrap_or_else(|err| {
        eprint!("invalid log filter: {err}");
        eprint!("using default log filter: info");
        EnvFilter::new("info")
    });
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    if args.json {
        subscriber.json().init();
    } else {
        subscriber.init();
    }
}

pub async fn get_addr(host: &str, port: u16) -> Result<SocketAddr, ApiError> {
    let addrs = format!("{}:{}", host, port)
        .to_socket_addrs()
        .map_err(|e| ApiError::InternalServerError {
            reason: format!("Failed to resolve address: {}", e),
        })?
        .collect::<Vec<SocketAddr>>();

    let socket = match addrs.first() {
        Some(addr) => *addr,
        None => {
            return Err(ApiError::InternalServerError {
                reason: "No socket adresses found".into(),
            });
        }
    };

    Ok(socket)
}

pub async fn run_server(addr: SocketAddr, router: Router) {
    info!("listening on {addr}");

    axum_server::bind(addr)
        .serve(router.into_make_service())
        .await
        .expect("error when start server")
}

#[derive(Debug)]
pub enum MiddlewareError {
    MissingAuthHeader,
    InvalidAuthHeader,
    AuthenticationFailed(CoreError),
    DelegatedRouteForbidden,
    OrganisationMcpDisabled,
    DelegationCheckFailed(CoreError),
}

impl From<MiddlewareError> for StatusCode {
    fn from(value: MiddlewareError) -> Self {
        match value {
            MiddlewareError::MissingAuthHeader => StatusCode::UNAUTHORIZED,
            MiddlewareError::InvalidAuthHeader => StatusCode::UNAUTHORIZED,
            MiddlewareError::AuthenticationFailed(_) => StatusCode::UNAUTHORIZED,
            MiddlewareError::DelegatedRouteForbidden | MiddlewareError::OrganisationMcpDisabled => {
                StatusCode::FORBIDDEN
            }
            MiddlewareError::DelegationCheckFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for MiddlewareError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            MiddlewareError::MissingAuthHeader => (
                StatusCode::UNAUTHORIZED,
                "E_MISSING_AUTH_HEADER",
                "Authorization header is missing",
            ),
            MiddlewareError::InvalidAuthHeader => (
                StatusCode::UNAUTHORIZED,
                "E_INVALID_AUTH_HEADER",
                "Invalid authorization header",
            ),
            MiddlewareError::AuthenticationFailed(_) => (
                StatusCode::UNAUTHORIZED,
                "E_AUTHENTICATION_FAILED",
                "Authentication failed",
            ),
            MiddlewareError::DelegatedRouteForbidden => (
                StatusCode::FORBIDDEN,
                "E_DELEGATED_ROUTE_FORBIDDEN",
                "This route is not available to AI agents",
            ),
            MiddlewareError::OrganisationMcpDisabled => (
                StatusCode::FORBIDDEN,
                "E_ORGANISATION_MCP_DISABLED",
                "AI access is turned off for this organisation",
            ),
            MiddlewareError::DelegationCheckFailed(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "E_INTERNAL_SERVER_ERROR",
                "Could not check this organisation's AI access",
            ),
        };

        (status, Json(ApiErrorResponse::new(code, status, message))).into_response()
    }
}

pub async fn extract_token_from_bearer(auth_header: &HeaderValue) -> Result<Token, ApiError> {
    let auth_str = auth_header.to_str().map_err(|_| ApiError::TokenNotFound)?;

    if !auth_str.starts_with("Bearer ") {
        return Err(ApiError::TokenNotFound);
    }

    let token = auth_str
        .strip_prefix("Bearer ")
        .ok_or(ApiError::TokenNotFound)?;

    Ok(Token::new(token.to_string()))
}

pub async fn auth_middleware<T>(
    State(state): State<T>,
    req: Request,
    next: Next,
) -> Result<Response, MiddlewareError>
where
    T: AuthService + UserService + OrganisationMcpSwitch + Clone + Send + Sync + 'static,
{
    auth_middleware_impl(State(state), req, |req| next.run(req)).await
}

pub(crate) async fn auth_middleware_impl<T, F, Fut>(
    State(state): State<T>,
    mut req: Request,
    next: F,
) -> Result<Response, MiddlewareError>
where
    T: AuthService + UserService + OrganisationMcpSwitch + Clone + Send + Sync + 'static,
    F: FnOnce(Request) -> Fut,
    Fut: std::future::Future<Output = Response> + Send,
{
    let auth_header = req
        .headers()
        .get(AUTHORIZATION)
        .ok_or(MiddlewareError::MissingAuthHeader)?;

    let token = extract_token_from_bearer(auth_header)
        .await
        .map_err(|_| MiddlewareError::InvalidAuthHeader)?;

    let identity = state.get_identity(token.as_str()).await.map_err(|e| {
        error!("Auth middleware: failed to identify user {:?}", e);
        MiddlewareError::AuthenticationFailed(e)
    })?;

    if identity.is_delegated() {
        admit_delegated(&state, req.method(), req.uri().path()).await?;
    }

    if let Identity::User(user) = &identity {
        let name = user.name.clone().unwrap_or_else(|| user.username.clone());
        let email = user
            .email
            .clone()
            .unwrap_or_else(|| format!("{}@local", user.username));

        let command = CreateUserCommand {
            name,
            email,
            sub: user.id.clone(),
        };
        if let Err(err) = state.create_user(command).await {
            error!("Auth middleware: failed to create user {:?}", err);
        }
    }

    req.extensions_mut().insert(identity);

    Ok(next(req).await)
}

async fn admit_delegated<T>(
    state: &T,
    method: &axum::http::Method,
    path: &str,
) -> Result<(), MiddlewareError>
where
    T: OrganisationMcpSwitch,
{
    match delegated::classify(method, path).ok_or(MiddlewareError::DelegatedRouteForbidden)? {
        delegated::Target::UserOrganisations => Ok(()),
        delegated::Target::Organisation(id) => {
            let enabled = state
                .mcp_enabled(OrganisationId(id))
                .await
                .map_err(MiddlewareError::DelegationCheckFailed)?;

            if enabled {
                Ok(())
            } else {
                Err(MiddlewareError::OrganisationMcpDisabled)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autharie_auth::{Identity, User};
    use autharie_core::CoreError;
    use axum::body::Body;
    use axum::http::HeaderValue;
    use axum::response::IntoResponse;
    use std::future::Future;

    #[tokio::test]
    async fn extract_token_from_bearer_success() {
        let header = HeaderValue::from_str("Bearer token-123").unwrap();
        let token = extract_token_from_bearer(&header).await.unwrap();
        assert_eq!(token.as_str(), "token-123");
    }

    #[tokio::test]
    async fn extract_token_from_bearer_rejects_missing_prefix() {
        let header = HeaderValue::from_str("Token abc").unwrap();
        let result = extract_token_from_bearer(&header).await;
        assert!(matches!(result, Err(ApiError::TokenNotFound)));
    }

    #[tokio::test]
    async fn extract_token_from_bearer_rejects_invalid_header() {
        let header = HeaderValue::from_bytes(b"\xFF").unwrap();
        let result = extract_token_from_bearer(&header).await;
        assert!(matches!(result, Err(ApiError::TokenNotFound)));
    }

    #[tokio::test]
    async fn get_addr_rejects_invalid_host() {
        let result = get_addr("invalid host", 1234).await;
        assert!(matches!(result, Err(ApiError::InternalServerError { .. })));
    }

    #[tokio::test]
    async fn get_addr_accepts_valid_host() {
        let result = get_addr("127.0.0.1", 0).await;
        assert!(result.is_ok());
    }

    #[test]
    fn middleware_error_status_codes() {
        assert_eq!(
            StatusCode::from(MiddlewareError::MissingAuthHeader),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            StatusCode::from(MiddlewareError::InvalidAuthHeader),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn middleware_error_into_response_is_unauthorized() {
        let response = MiddlewareError::MissingAuthHeader.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = MiddlewareError::InvalidAuthHeader.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[derive(Clone)]
    struct FakeAuthService {
        fail: bool,
        delegated: bool,
        mcp_enabled: bool,
    }

    impl FakeAuthService {
        fn new(fail: bool) -> Self {
            Self {
                fail,
                delegated: false,
                mcp_enabled: false,
            }
        }

        fn delegated(mcp_enabled: bool) -> Self {
            Self {
                fail: false,
                delegated: true,
                mcp_enabled,
            }
        }
    }

    impl OrganisationMcpSwitch for FakeAuthService {
        fn mcp_enabled(
            &self,
            _id: OrganisationId,
        ) -> impl Future<Output = Result<bool, CoreError>> + Send {
            let enabled = self.mcp_enabled;
            Box::pin(async move { Ok(enabled) })
        }
    }

    impl AuthService for FakeAuthService {
        fn get_identity(
            &self,
            _token: &str,
        ) -> impl Future<Output = Result<Identity, CoreError>> + Send {
            let fail = self.fail;
            let delegated = self.delegated;
            Box::pin(async move {
                if fail {
                    Err(CoreError::InvalidIdentity)
                } else {
                    Ok(Identity::User(User {
                        scopes: Vec::new(),
                        act: delegated.then(autharie_auth::Actor::default),
                        id: "user-1".to_string(),
                        username: "user".to_string(),
                        email: None,
                        name: None,
                        roles: vec![],
                    }))
                }
            })
        }
    }

    impl UserService for FakeAuthService {
        fn create_user(
            &self,
            _command: CreateUserCommand,
        ) -> impl Future<Output = Result<autharie_core::user::User, CoreError>> + Send {
            Box::pin(async move {
                Err(CoreError::DatabaseError {
                    message: "not implemented".to_string(),
                })
            })
        }
    }

    #[tokio::test]
    async fn auth_middleware_impl_rejects_missing_header() {
        let state = FakeAuthService::new(false);
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();

        let result = auth_middleware_impl(State(state), req, |_req| async move {
            StatusCode::OK.into_response()
        })
        .await;

        assert!(matches!(result, Err(MiddlewareError::MissingAuthHeader)));
    }

    #[tokio::test]
    async fn auth_middleware_impl_rejects_invalid_token() {
        let state = FakeAuthService::new(false);
        let req = Request::builder()
            .uri("/")
            .header(AUTHORIZATION, "Token abc")
            .body(Body::empty())
            .unwrap();

        let result = auth_middleware_impl(State(state), req, |_req| async move {
            StatusCode::OK.into_response()
        })
        .await;

        assert!(matches!(result, Err(MiddlewareError::InvalidAuthHeader)));
    }

    #[tokio::test]
    async fn auth_middleware_impl_rejects_failed_identity() {
        let state = FakeAuthService::new(true);
        let req = Request::builder()
            .uri("/")
            .header(AUTHORIZATION, "Bearer token")
            .body(Body::empty())
            .unwrap();

        let result = auth_middleware_impl(State(state), req, |_req| async move {
            StatusCode::OK.into_response()
        })
        .await;

        assert!(matches!(
            result,
            Err(MiddlewareError::AuthenticationFailed(_))
        ));
    }

    #[tokio::test]
    async fn auth_middleware_impl_inserts_identity_on_success() {
        let state = FakeAuthService::new(false);
        let req = Request::builder()
            .uri("/")
            .header(AUTHORIZATION, "Bearer token")
            .body(Body::empty())
            .unwrap();

        let result = auth_middleware_impl(State(state), req, |req| async move {
            let identity = req.extensions().get::<Identity>().cloned();
            let response = if identity.is_some() {
                StatusCode::OK
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            response.into_response()
        })
        .await;

        assert!(matches!(result, Ok(response) if response.status() == StatusCode::OK));
    }

    fn delegated_request(method: &str, path: &str) -> Request {
        Request::builder()
            .method(method)
            .uri(path)
            .header(AUTHORIZATION, "Bearer token")
            .body(Body::empty())
            .unwrap()
    }

    async fn call(state: FakeAuthService, req: Request) -> Result<Response, MiddlewareError> {
        auth_middleware_impl(State(state), req, |_req| async move {
            StatusCode::OK.into_response()
        })
        .await
    }

    const ORG: &str = "6f1c1b0e-5a53-4b9b-8d1e-0a5f3f1f9c11";

    #[tokio::test]
    async fn a_delegated_call_to_an_organisation_with_the_switch_on_passes() {
        let result = call(
            FakeAuthService::delegated(true),
            delegated_request("GET", &format!("/organisations/{ORG}/deployments")),
        )
        .await;

        assert!(matches!(result, Ok(response) if response.status() == StatusCode::OK));
    }

    #[tokio::test]
    async fn a_delegated_call_to_an_organisation_with_the_switch_off_is_refused() {
        let result = call(
            FakeAuthService::delegated(false),
            delegated_request("GET", &format!("/organisations/{ORG}/deployments")),
        )
        .await;

        let Err(error) = result else {
            panic!("the call was let through");
        };
        assert!(matches!(error, MiddlewareError::OrganisationMcpDisabled));
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["code"], "E_ORGANISATION_MCP_DISABLED");
        assert_eq!(
            body["message"],
            "AI access is turned off for this organisation"
        );
    }

    #[tokio::test]
    async fn a_delegated_call_cannot_patch_the_organisation_even_with_the_switch_on() {
        let result = call(
            FakeAuthService::delegated(true),
            delegated_request("PATCH", &format!("/organisations/{ORG}")),
        )
        .await;

        let Err(error) = result else {
            panic!("the call was let through");
        };
        assert!(matches!(error, MiddlewareError::DelegatedRouteForbidden));
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["code"], "E_DELEGATED_ROUTE_FORBIDDEN");
    }

    #[tokio::test]
    async fn a_delegated_call_to_a_route_off_the_allowlist_is_refused() {
        let result = call(
            FakeAuthService::delegated(true),
            delegated_request("GET", &format!("/organisations/{ORG}/members")),
        )
        .await;

        assert!(matches!(
            result,
            Err(MiddlewareError::DelegatedRouteForbidden)
        ));
    }

    #[tokio::test]
    async fn the_organisation_list_does_not_need_a_switch() {
        let result = call(
            FakeAuthService::delegated(false),
            delegated_request("GET", "/users/@me/organisations"),
        )
        .await;

        assert!(matches!(result, Ok(response) if response.status() == StatusCode::OK));
    }

    #[tokio::test]
    async fn an_ordinary_call_is_not_subject_to_the_switch_or_the_allowlist() {
        let result = call(
            FakeAuthService::new(false),
            delegated_request("PATCH", &format!("/organisations/{ORG}")),
        )
        .await;

        assert!(matches!(result, Ok(response) if response.status() == StatusCode::OK));
    }
}
