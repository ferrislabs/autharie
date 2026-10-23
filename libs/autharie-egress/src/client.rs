use std::{
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use bytes::{Bytes, BytesMut};
use reqwest::{
    Client, Method, RequestBuilder, Response,
    dns::{Addrs, Name, Resolve, Resolving},
    redirect,
};
use url::{Host, Url};

use crate::address::is_public;

#[derive(Debug, Clone, thiserror::Error)]
pub enum EgressError {
    #[error("{host} resolves to {address}, which is not a public address")]
    Blocked { host: String, address: IpAddr },
    #[error("only https URLs to a named host are allowed")]
    Url,
    #[error("{host} did not resolve")]
    Unresolved { host: String },
    #[error("the response is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("the server did not answer in time")]
    Timeout,
    #[error("the server could not be reached")]
    Unreachable,
}

impl EgressError {
    pub fn is_refusal(&self) -> bool {
        matches!(self, Self::Blocked { .. } | Self::Url)
    }

    pub fn classify(error: &reqwest::Error) -> Self {
        let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
        while let Some(current) = source {
            if let Some(egress) = current.downcast_ref::<GuardRefusal>() {
                return egress.0.clone();
            }
            source = current.source();
        }
        if error.is_timeout() {
            Self::Timeout
        } else {
            Self::Unreachable
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct GuardRefusal(EgressError);

pub trait Lookup: Send + Sync + 'static {
    fn lookup(&self, host: &str) -> impl Future<Output = io::Result<Vec<IpAddr>>> + Send;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemLookup;

impl Lookup for SystemLookup {
    async fn lookup(&self, host: &str) -> io::Result<Vec<IpAddr>> {
        let resolved = tokio::net::lookup_host((host, 0)).await?;
        Ok(resolved.map(|address| address.ip()).collect())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    https_only: bool,
    private_addresses: bool,
}

impl Policy {
    pub const PUBLIC: Self = Self {
        https_only: true,
        private_addresses: false,
    };

    #[cfg(feature = "test-support")]
    pub const fn local_testing() -> Self {
        Self {
            https_only: false,
            private_addresses: true,
        }
    }

    fn allows(&self, address: IpAddr) -> bool {
        self.private_addresses || is_public(address)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub connect_timeout: Duration,
    pub timeout: Option<Duration>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            timeout: None,
        }
    }
}

struct Guard<L> {
    lookup: L,
    policy: Policy,
}

impl<L: Lookup> Guard<L> {
    async fn checked(&self, host: &str) -> Result<Vec<IpAddr>, EgressError> {
        let addresses = self
            .lookup
            .lookup(host)
            .await
            .map_err(|_| EgressError::Unresolved {
                host: host.to_string(),
            })?;
        if addresses.is_empty() {
            return Err(EgressError::Unresolved {
                host: host.to_string(),
            });
        }
        if let Some(address) = addresses
            .iter()
            .find(|address| !self.policy.allows(**address))
        {
            return Err(EgressError::Blocked {
                host: host.to_string(),
                address: *address,
            });
        }
        Ok(addresses)
    }
}

struct GuardedResolver<L>(Arc<Guard<L>>);

impl<L: Lookup> Resolve for GuardedResolver<L> {
    fn resolve(&self, name: Name) -> Resolving {
        let guard = Arc::clone(&self.0);
        Box::pin(async move {
            let addresses = guard.checked(name.as_str()).await.map_err(|error| {
                Box::new(GuardRefusal(error)) as Box<dyn std::error::Error + Send + Sync>
            })?;
            let addrs: Addrs = Box::new(
                addresses
                    .into_iter()
                    .map(|address| SocketAddr::new(address, 0))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            Ok(addrs)
        })
    }
}

#[derive(Clone)]
pub struct GuardedClient {
    client: Client,
    policy: Policy,
}

impl GuardedClient {
    pub fn new(settings: Settings) -> Result<Self, reqwest::Error> {
        Self::with(SystemLookup, Policy::PUBLIC, settings)
    }

    pub fn with<L: Lookup>(
        lookup: L,
        policy: Policy,
        settings: Settings,
    ) -> Result<Self, reqwest::Error> {
        let mut builder = Client::builder()
            .redirect(redirect::Policy::none())
            .no_proxy()
            .connect_timeout(settings.connect_timeout)
            .dns_resolver(Arc::new(GuardedResolver(Arc::new(Guard {
                lookup,
                policy,
            }))));
        if let Some(timeout) = settings.timeout {
            builder = builder.timeout(timeout);
        }
        Ok(Self {
            client: builder.build()?,
            policy,
        })
    }

    pub fn request(&self, method: Method, url: &str) -> Result<RequestBuilder, EgressError> {
        let parsed = Url::parse(url).map_err(|_| EgressError::Url)?;
        let scheme_ok =
            parsed.scheme() == "https" || (!self.policy.https_only && parsed.scheme() == "http");
        if !scheme_ok || !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(EgressError::Url);
        }
        match parsed.host() {
            None => return Err(EgressError::Url),
            Some(Host::Ipv4(address)) => self.literal(IpAddr::V4(address))?,
            Some(Host::Ipv6(address)) => self.literal(IpAddr::V6(address))?,
            Some(Host::Domain(_)) => {}
        }
        Ok(self.client.request(method, parsed))
    }

    fn literal(&self, address: IpAddr) -> Result<(), EgressError> {
        if self.policy.allows(address) {
            Ok(())
        } else {
            Err(EgressError::Blocked {
                host: address.to_string(),
                address,
            })
        }
    }
}

pub async fn read_capped(mut response: Response, limit: usize) -> Result<Bytes, EgressError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(EgressError::TooLarge { limit });
    }
    let mut body = BytesMut::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| EgressError::classify(&error))?
    {
        if body.len() + chunk.len() > limit {
            return Err(EgressError::TooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use axum::{Router, http::StatusCode, response::IntoResponse, routing::get};
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Clone, Default)]
    struct Stub {
        hosts: Arc<Mutex<HashMap<String, Vec<IpAddr>>>>,
        calls: Arc<AtomicUsize>,
    }

    impl Stub {
        fn with(host: &str, addresses: &[&str]) -> Self {
            let stub = Self::default();
            stub.hosts.lock().unwrap().insert(
                host.to_string(),
                addresses.iter().map(|a| a.parse().unwrap()).collect(),
            );
            stub
        }
    }

    impl Lookup for Stub {
        async fn lookup(&self, host: &str) -> io::Result<Vec<IpAddr>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.hosts
                .lock()
                .unwrap()
                .get(host)
                .cloned()
                .ok_or_else(|| io::Error::other("unknown host"))
        }
    }

    const OPEN: Policy = Policy {
        https_only: false,
        private_addresses: true,
    };
    const PLAIN_HTTP: Policy = Policy {
        https_only: false,
        private_addresses: false,
    };

    async fn serve(app: Router) -> (u16, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&hits);
        let app = app.layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let counted = Arc::clone(&counted);
                async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                    next.run(request).await
                }
            },
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (port, hits)
    }

    fn client(stub: Stub, policy: Policy) -> GuardedClient {
        GuardedClient::with(
            stub,
            policy,
            Settings {
                connect_timeout: Duration::from_secs(2),
                timeout: Some(Duration::from_millis(800)),
            },
        )
        .unwrap()
    }

    async fn get_text(client: &GuardedClient, url: &str) -> Result<String, EgressError> {
        let response = client
            .request(Method::GET, url)?
            .send()
            .await
            .map_err(|error| EgressError::classify(&error))?;
        let body = read_capped(response, 1024 * 1024).await?;
        Ok(String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn a_host_resolving_to_loopback_is_never_connected_to() {
        let (port, hits) = serve(Router::new().route("/", get(|| async { "secret" }))).await;
        let client = client(Stub::with("evil.example", &["127.0.0.1"]), PLAIN_HTTP);

        let error = get_text(&client, &format!("http://evil.example:{port}/"))
            .await
            .expect_err("refused");

        assert!(matches!(error, EgressError::Blocked { .. }), "{error}");
        assert!(error.is_refusal());
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_host_resolving_to_the_cloud_metadata_address_is_refused() {
        let client = client(Stub::with("meta.example", &["169.254.169.254"]), PLAIN_HTTP);

        let error = get_text(&client, "http://meta.example/latest/meta-data")
            .await
            .expect_err("refused");

        assert!(matches!(
            error,
            EgressError::Blocked { address, .. } if address == "169.254.169.254".parse::<IpAddr>().unwrap()
        ));
    }

    #[tokio::test]
    async fn one_private_address_among_public_ones_refuses_the_host() {
        let (port, hits) = serve(Router::new().route("/", get(|| async { "x" }))).await;
        let client = client(
            Stub::with("mixed.example", &["93.184.216.34", "127.0.0.1"]),
            PLAIN_HTTP,
        );

        let error = get_text(&client, &format!("http://mixed.example:{port}/"))
            .await
            .expect_err("refused");

        assert!(matches!(error, EgressError::Blocked { .. }));
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_address_literal_that_is_private_is_refused_without_resolving() {
        let stub = Stub::default();
        let client = client(stub.clone(), PLAIN_HTTP);

        for url in [
            "http://127.0.0.1:9/",
            "http://10.0.0.5/mcp",
            "http://[::1]:9/",
            "http://169.254.169.254/",
            "http://[::ffff:127.0.0.1]/",
        ] {
            let error = get_text(&client, url).await.expect_err(url);
            assert!(matches!(error, EgressError::Blocked { .. }), "{url}");
        }
        assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_connection_goes_to_the_address_that_was_checked_after_one_resolution() {
        let (port, hits) = serve(Router::new().route("/", get(|| async { "pinned" }))).await;
        let stub = Stub::with("pin.example", &["127.0.0.1"]);
        let client = client(stub.clone(), OPEN);

        let body = get_text(&client, &format!("http://pin.example:{port}/"))
            .await
            .expect("served");

        assert_eq!(body, "pinned");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_redirect_is_returned_not_followed() {
        let (target, target_hits) =
            serve(Router::new().route("/", get(|| async { "inside" }))).await;
        let location = format!("http://127.0.0.1:{target}/");
        let (port, _) = serve(Router::new().route(
            "/",
            get(move || {
                let location = location.clone();
                async move { (StatusCode::FOUND, [("location", location)]).into_response() }
            }),
        ))
        .await;
        let client = client(Stub::with("hop.example", &["127.0.0.1"]), OPEN);

        let response = client
            .request(Method::GET, &format!("http://hop.example:{port}/"))
            .unwrap()
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(target_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_body_over_the_limit_is_cut_off() {
        let (port, _) =
            serve(Router::new().route("/", get(|| async { "a".repeat(2 * 1024 * 1024) }))).await;
        let client = client(Stub::with("big.example", &["127.0.0.1"]), OPEN);
        let response = client
            .request(Method::GET, &format!("http://big.example:{port}/"))
            .unwrap()
            .send()
            .await
            .unwrap();

        let error = read_capped(response, 1024 * 1024).await.expect_err("cut");

        assert!(matches!(error, EgressError::TooLarge { limit } if limit == 1024 * 1024));
    }

    #[tokio::test]
    async fn a_slow_server_ends_in_a_timeout() {
        let (port, _) = serve(Router::new().route(
            "/",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                "late"
            }),
        ))
        .await;
        let client = client(Stub::with("slow.example", &["127.0.0.1"]), OPEN);

        let error = get_text(&client, &format!("http://slow.example:{port}/"))
            .await
            .expect_err("timed out");

        assert!(matches!(error, EgressError::Timeout), "{error}");
    }

    #[tokio::test]
    async fn plain_http_credentials_and_unnamed_hosts_are_not_urls_for_the_public_client() {
        let client = client(Stub::default(), Policy::PUBLIC);

        for url in [
            "http://example.com/",
            "https://user:pw@example.com/",
            "ftp://example.com/",
            "not a url",
        ] {
            assert!(
                matches!(client.request(Method::GET, url), Err(EgressError::Url)),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn a_host_that_does_not_resolve_is_unreachable_not_a_panic() {
        let client = client(Stub::default(), PLAIN_HTTP);

        let error = get_text(&client, "http://nowhere.example/")
            .await
            .expect_err("unresolved");

        assert!(matches!(error, EgressError::Unresolved { .. }), "{error}");
    }
}
