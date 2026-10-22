pub mod app;
pub mod authzen;
pub mod config;
pub mod decision;
pub mod exchange;
pub mod logging;
pub mod rpc;
pub mod upstream;
pub mod verifier;

use std::time::Duration;

use reqwest::{Client, redirect::Policy};

use crate::{
    app::{AppState, Gateway},
    config::GatewayConfig,
    decision::DecisionSink,
    upstream::{Routes, Timing, Upstream},
};

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("http client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("upstream url: {0}")]
    Url(#[from] url::ParseError),
}

pub fn build_state<S: DecisionSink>(
    config: GatewayConfig,
    sink: S,
) -> Result<AppState<S>, BuildError> {
    let http = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .build()?;
    let control = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?;
    let timing = Timing {
        jwks_refresh: Duration::from_secs(config.jwks_refresh_secs),
        pdp_timeout: Duration::from_millis(config.pdp_timeout_ms),
        unknown_kid_cooldown: Duration::from_secs(5),
    };
    let upstreams = config
        .upstreams
        .into_iter()
        .map(|upstream| Upstream::new(upstream, &control, &timing))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AppState::new(Gateway {
        routes: Routes::new(upstreams),
        sink,
        http,
    }))
}
