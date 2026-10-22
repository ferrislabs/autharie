use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;
use mcp_gateway::{
    app::router, build_state, config::GatewayConfig, decision::TracingSink, logging,
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
    let source = match (&args.config_toml, &args.config) {
        (Some(inline), _) => inline.clone(),
        (None, Some(path)) => std::fs::read_to_string(path)?,
        (None, None) => return Err("set MCP_GATEWAY_CONFIG or MCP_GATEWAY_CONFIG_TOML".into()),
    };
    let config = GatewayConfig::from_toml(&source)?;
    let listen = args.listen.unwrap_or(config.listen);
    let upstreams = config.upstreams.len();
    let app = router(build_state(config, TracingSink)?);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(%listen, upstreams, "mcp gateway started");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}
