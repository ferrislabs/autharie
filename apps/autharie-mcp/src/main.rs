use std::net::SocketAddr;

use autharie_mcp::{ApiClient, router};
use clap::Parser;

#[derive(Parser)]
#[command(about = "Autharie MCP server: private, reached only through the MCP gateway")]
struct Args {
    #[arg(long, env = "AUTHARIE_MCP_LISTEN", default_value = "0.0.0.0:8081")]
    listen: SocketAddr,
    #[arg(
        long,
        env = "AUTHARIE_MCP_API_URL",
        default_value = "http://localhost:3333"
    )]
    api_url: String,
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
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(listen = %args.listen, api = %args.api_url, "autharie-mcp started");
    axum::serve(listener, router(ApiClient::new(args.api_url)))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}
