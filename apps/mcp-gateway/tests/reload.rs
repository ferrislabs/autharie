use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use mcp_gateway::{
    app::router,
    build_state,
    decision::TracingSink,
    reload::{self, Poll},
};
use reqwest::Client;
use tokio::net::TcpListener;

const METADATA: &str = "/.well-known/oauth-protected-resource/mcp";

fn upstream(canonical: &str) -> String {
    format!(
        r#"
        [[upstreams]]
        canonical_uri = "{canonical}"
        issuer = "http://auth.invalid/realms/r"
        upstream_url = "http://up.invalid/mcp"
        server_name = "s"

        [upstreams.pdp]
        evaluation_url = "http://pdp.invalid/e"
        evaluations_url = "http://pdp.invalid/es"
        client_id = "c"
        client_secret = "s"

        [upstreams.exchange]
        token_url = "http://auth.invalid/token"
        client_id = "c"
        client_secret = "s"
        audience = "a"
        "#
    )
}

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("mcp-gateway-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("config.toml")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

async fn status(base: &str, host: &str, path: &str) -> u16 {
    Client::new()
        .get(format!("{base}{path}"))
        .header("host", host)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn serve(path: &Path) -> (String, mcp_gateway::app::AppState<TracingSink>, Option<u64>) {
    let (config, digest) = reload::initial(path).unwrap();
    let state = build_state(config, TracingSink).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), state, digest)
}

#[tokio::test]
async fn a_missing_file_starts_with_no_upstream_and_a_new_file_is_picked_up() {
    let dir = Dir::new();
    let (base, state, mut digest) = serve(&dir.file()).await;
    assert_eq!(digest, None);
    assert_eq!(status(&base, "a.example", METADATA).await, 404);
    assert_eq!(
        reload::poll(&state, &dir.file(), &mut digest),
        Poll::Unchanged
    );

    fs::write(dir.file(), upstream("https://a.example/mcp")).unwrap();
    assert_eq!(
        reload::poll(&state, &dir.file(), &mut digest),
        Poll::Reloaded(1)
    );
    assert_eq!(status(&base, "a.example", METADATA).await, 200);
}

#[tokio::test]
async fn an_unchanged_file_is_not_rebuilt() {
    let dir = Dir::new();
    fs::write(dir.file(), upstream("https://a.example/mcp")).unwrap();
    let (_, state, mut digest) = serve(&dir.file()).await;
    assert_eq!(
        reload::poll(&state, &dir.file(), &mut digest),
        Poll::Unchanged
    );
}

#[tokio::test]
async fn an_invalid_config_keeps_the_previous_one() {
    let dir = Dir::new();
    fs::write(dir.file(), upstream("https://a.example/mcp")).unwrap();
    let (base, state, mut digest) = serve(&dir.file()).await;
    assert_eq!(status(&base, "a.example", METADATA).await, 200);

    fs::write(dir.file(), "upstreams = [ not toml").unwrap();
    assert_eq!(
        reload::poll(&state, &dir.file(), &mut digest),
        Poll::Rejected
    );
    assert_eq!(status(&base, "a.example", METADATA).await, 200);
    assert_eq!(
        reload::poll(&state, &dir.file(), &mut digest),
        Poll::Unchanged
    );

    fs::write(dir.file(), upstream("https://b.example/mcp")).unwrap();
    assert_eq!(
        reload::poll(&state, &dir.file(), &mut digest),
        Poll::Reloaded(1)
    );
    assert_eq!(status(&base, "a.example", METADATA).await, 404);
    assert_eq!(status(&base, "b.example", METADATA).await, 200);
}

#[tokio::test]
async fn an_empty_upstream_list_is_applied_and_answers_404() {
    let dir = Dir::new();
    fs::write(dir.file(), upstream("https://a.example/mcp")).unwrap();
    let (base, state, mut digest) = serve(&dir.file()).await;

    fs::write(dir.file(), "upstreams = []\n").unwrap();
    assert_eq!(
        reload::poll(&state, &dir.file(), &mut digest),
        Poll::Reloaded(0)
    );
    assert_eq!(status(&base, "a.example", METADATA).await, 404);
    assert_eq!(status(&base, "a.example", "/mcp").await, 404);
}

#[tokio::test]
async fn a_symlink_swap_like_a_secret_volume_update_is_detected() {
    let dir = Dir::new();
    let one = dir.0.join("one.toml");
    let two = dir.0.join("two.toml");
    fs::write(&one, upstream("https://a.example/mcp")).unwrap();
    fs::write(&two, upstream("https://b.example/mcp")).unwrap();
    let link = dir.file();
    std::os::unix::fs::symlink(&one, &link).unwrap();
    let (base, state, mut digest) = serve(&link).await;
    assert_eq!(status(&base, "a.example", METADATA).await, 200);

    let staged = dir.0.join("staged");
    std::os::unix::fs::symlink(&two, &staged).unwrap();
    fs::rename(&staged, &link).unwrap();
    assert_eq!(reload::poll(&state, &link, &mut digest), Poll::Reloaded(1));
    assert_eq!(status(&base, "b.example", METADATA).await, 200);
}

#[tokio::test]
async fn the_watcher_applies_a_change_on_its_own() {
    let dir = Dir::new();
    let (base, state, digest) = serve(&dir.file()).await;
    let path = dir.file();
    tokio::spawn(reload::watch(
        state,
        path,
        digest,
        Duration::from_millis(20),
    ));

    fs::write(dir.file(), upstream("https://a.example/mcp")).unwrap();
    for _ in 0..100 {
        if status(&base, "a.example", METADATA).await == 200 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the watcher did not apply the new config");
}
