use std::net::SocketAddr;

use clap::Parser;
use ferriskey_mcp::{Upstream, router};

#[derive(Parser)]
#[command(about = "FerrisKey MCP server: private, reached only through the MCP gateway")]
struct Args {
    #[arg(long, env = "FERRISKEY_MCP_LISTEN", default_value = "0.0.0.0:8080")]
    listen: SocketAddr,
    #[arg(
        long,
        env = "FERRISKEY_MCP_API_URL_TEMPLATE",
        default_value = "http://{name}-api.{namespace}.svc:3333"
    )]
    api_url_template: String,
    #[arg(long, env = "FERRISKEY_MCP_ROOT_PATH", default_value = "/api")]
    root_path: String,
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let upstream = Upstream::new(&args.api_url_template, &args.root_path)?;
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(
        listen = %args.listen,
        template = %args.api_url_template,
        root = %args.root_path,
        "ferriskey-mcp started"
    );
    axum::serve(listener, router(upstream))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}
