use axum::{
    Router,
    extract::{Path, Request},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};

use crate::{
    api::ApiClient,
    tools::{FerrisKeyTools, bearer},
    upstream::Upstream,
};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;

pub fn router(upstream: Upstream) -> Router {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_max_request_body_bytes(MAX_REQUEST_BYTES)
        .disable_allowed_hosts();
    let api = ApiClient::new();
    let service = StreamableHttpService::new(
        move || Ok(FerrisKeyTools::new(api.clone())),
        std::sync::Arc::new(NeverSessionManager::default()),
        config,
    );
    let resolver = upstream.clone();
    Router::new()
        .route_service("/instances/{namespace}/{name}/mcp", service)
        .route_layer(middleware::from_fn(require_bearer))
        .route_layer(middleware::from_fn(
            move |Path((namespace, name)): Path<(String, String)>,
                  mut request: Request,
                  next: Next| {
                let instance = resolver.resolve(&namespace, &name);
                async move {
                    let Some(instance) = instance else {
                        return StatusCode::NOT_FOUND.into_response();
                    };
                    request.extensions_mut().insert(instance);
                    next.run(request).await
                }
            },
        ))
        .route("/healthz", get(|| async { StatusCode::OK }))
}

async fn require_bearer(request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    if bearer(&parts).is_none() {
        let mut response = StatusCode::UNAUTHORIZED.into_response();
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return response;
    }
    next.run(Request::from_parts(parts, body)).await
}
