pub mod app;
pub mod authzen;
pub mod config;
pub mod decision;
pub mod exchange;
pub mod legacy;
pub mod logging;
pub mod reload;
pub mod rpc;
pub mod upstream;
pub mod verifier;

use std::time::Duration;

use autharie_egress::{GuardedClient, Settings};
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

fn control_client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
}

pub fn build_upstreams(config: GatewayConfig) -> Result<Vec<Upstream>, BuildError> {
    let control = control_client()?;
    let timing = Timing {
        jwks_refresh: Duration::from_secs(config.jwks_refresh_secs),
        pdp_timeout: Duration::from_millis(config.pdp_timeout_ms),
        unknown_kid_cooldown: Duration::from_secs(5),
    };
    config
        .upstreams
        .into_iter()
        .map(|upstream| Upstream::new(upstream, &control, &timing).map_err(BuildError::from))
        .collect()
}

pub fn public_client() -> Result<GuardedClient, BuildError> {
    Ok(GuardedClient::new(Settings::default())?)
}

pub fn build_state<S: DecisionSink>(
    config: GatewayConfig,
    sink: S,
) -> Result<AppState<S>, BuildError> {
    build_state_with(config, sink, public_client()?)
}

pub fn build_state_with<S: DecisionSink>(
    config: GatewayConfig,
    sink: S,
    public: GuardedClient,
) -> Result<AppState<S>, BuildError> {
    let http = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .build()?;
    Ok(AppState::new(Gateway {
        routes: Routes::new(build_upstreams(config)?),
        sink,
        http,
        public,
        sessions: legacy::Sessions::default(),
    }))
}
