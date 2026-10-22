use std::{net::SocketAddr, path::PathBuf, time::Duration};

use clap::Parser;
use mcp_gateway::{
    app::router, build_state, config::GatewayConfig, decision::TracingSink, logging, reload,
};
use tokio::signal::unix::{SignalKind, signal};

#[derive(Debug, Parser)]
#[command(name = "mcp-gateway")]
struct Args {
    #[arg(long, env = "MCP_GATEWAY_CONFIG", conflicts_with = "config_toml")]
    config: Option<PathBuf>,

    #[arg(long, env = "MCP_GATEWAY_CONFIG_TOML", hide_env_values = true)]
    config_toml: Option<String>,

    #[arg(long, env = "MCP_GATEWAY_LISTEN")]
    listen: Option<SocketAddr>,

    #[arg(long, env = "MCP_GATEWAY_RELOAD_SECS", default_value_t = 5)]
    reload_interval_secs: u64,
}

async fn shutdown() {
    let Ok(mut term) = signal(SignalKind::terminate()) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    logging::init();
    let args = Args::parse();
    let (config, digest) = match (&args.config_toml, &args.config) {
        (Some(inline), _) => (GatewayConfig::from_toml(inline)?, None),
        (None, Some(path)) => reload::initial(path)?,
        (None, None) => return Err("set MCP_GATEWAY_CONFIG or MCP_GATEWAY_CONFIG_TOML".into()),
    };
    let listen = args.listen.unwrap_or(config.listen);
    let upstreams = config.upstreams.len();
    let state = build_state(config, TracingSink)?;
    if let (None, Some(path)) = (&args.config_toml, args.config) {
        tokio::spawn(reload::watch(
            state.clone(),
            path,
            digest,
            Duration::from_secs(args.reload_interval_secs.max(1)),
        ));
    }
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(%listen, upstreams, "mcp gateway started");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}
