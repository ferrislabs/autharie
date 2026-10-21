use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use autharie_core::deployments::ports::DeploymentService;
use autharie_core::deployments::reachability_history::{PROBE_INTERVAL_SECONDS, ReachabilityCheck};
use autharie_core::signals::{Signal, SignalId, SignalKind, SignalSubject};
use chrono::Utc;
use reqwest::{Certificate, Client};
use tokio::time::interval;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::state::AppState;

const EVERY: Duration = Duration::from_secs(PROBE_INTERVAL_SECONDS as u64);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

fn health_path_for_kind(kind: &autharie_core::deployments::DeploymentKind) -> &'static str {
    match kind {
        // The webapp's nginx answers 200 to any path on the host, so "/health"
        // reports up while the API is down. The API serves its own health
        // under its root path, which the HTTPRoute sends to the API.
        autharie_core::deployments::DeploymentKind::Ferriskey => "/api/health/ready",
        autharie_core::deployments::DeploymentKind::Keycloak => "/health/ready",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeVia {
    host: String,
    port: u16,
}

impl FromStr for ProbeVia {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || format!("`{value}` is not host:port, e.g. host.docker.internal:8444");

        let (host, port) = if let Some(rest) = value.strip_prefix('[') {
            let (host, rest) = rest.split_once(']').ok_or_else(invalid)?;
            (host, rest.strip_prefix(':').ok_or_else(invalid)?)
        } else {
            value.rsplit_once(':').ok_or_else(invalid)?
        };

        let port: u16 = port.parse().map_err(|_| invalid())?;

        if host.is_empty() || port == 0 || (!value.starts_with('[') && host.contains(':')) {
            return Err(invalid());
        }

        Ok(Self {
            host: host.to_string(),
            port,
        })
    }
}

impl ProbeVia {
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

pub struct ProbeSettings {
    pub via: Option<ProbeVia>,
    pub ca: Option<Certificate>,
}

pub fn load_probe_ca(path: &str) -> Result<Certificate, String> {
    let pem = std::fs::read(path).map_err(|err| format!("cannot read {path}: {err}"))?;

    Certificate::from_pem(&pem).map_err(|err| format!("{path} is not a PEM certificate: {err}"))
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProbeTarget {
    pub url: String,
}

impl ProbeTarget {
    pub fn new(hostname: &str, path: &str, via: Option<&ProbeVia>) -> Self {
        let url = match via {
            Some(via) => format!("https://{hostname}:{}{path}", via.port()),
            None => format!("https://{hostname}{path}"),
        };

        Self { url }
    }
}

fn build_client(
    ca: Option<&Certificate>,
    overrides: &[(&str, &[SocketAddr])],
) -> Result<Client, reqwest::Error> {
    let mut builder = Client::builder().timeout(HTTP_TIMEOUT);

    if let Some(ca) = ca {
        builder = builder.add_root_certificate(ca.clone());
    }

    for (hostname, addrs) in overrides {
        builder = builder.resolve_to_addrs(hostname, addrs);
    }

    builder.build()
}

async fn check_deployment_reachable(client: &Client, url: &str) -> bool {
    match client.get(url).timeout(HTTP_TIMEOUT).send().await {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

pub async fn run_deployment_reachability_probe(state: AppState, settings: ProbeSettings) {
    info!(via = ?settings.via, "starting deployment reachability probe");

    let mut ticker = interval(EVERY);

    loop {
        ticker.tick().await;

        let deployments = match state.service.list_all_live_deployments().await {
            Ok(deployments) => deployments,
            Err(err) => {
                error!(%err, "failed to list live deployments for reachability probe");
                continue;
            }
        };

        let Some(zone) = state.args.ovh.domain() else {
            continue;
        };

        let hostnames: Vec<String> = deployments
            .iter()
            .map(|deployment| autharie_core::dns::hostname_for(&deployment.name.0, &zone))
            .collect();

        let addrs: Vec<SocketAddr> = match &settings.via {
            Some(via) => match tokio::net::lookup_host((via.host(), via.port())).await {
                Ok(addrs) => addrs.collect(),
                Err(err) => {
                    warn!(via = via.host(), %err, "cannot resolve the reachability probe address");
                    continue;
                }
            },
            None => Vec::new(),
        };

        let overrides: Vec<(&str, &[SocketAddr])> = if settings.via.is_some() {
            hostnames
                .iter()
                .map(|hostname| (hostname.as_str(), addrs.as_slice()))
                .collect()
        } else {
            Vec::new()
        };

        let client = match build_client(settings.ca.as_ref(), &overrides) {
            Ok(client) => client,
            Err(err) => {
                error!(%err, "failed to build the reachability probe client");
                continue;
            }
        };

        for (deployment, hostname) in deployments.into_iter().zip(hostnames) {
            let health_path = health_path_for_kind(&deployment.kind);
            let url = ProbeTarget::new(&hostname, health_path, settings.via.as_ref()).url;

            let now = Utc::now();
            let dedup_key = format!("deployment-unreachable-{}", deployment.id.0);

            let reachable = check_deployment_reachable(&client, &url).await;

            let check = ReachabilityCheck {
                deployment_id: deployment.id,
                checked_at: now,
                reachable,
            };

            if let Err(err) = state.service.record_reachability_check(check).await {
                error!(
                    deployment_id = %deployment.id,
                    %err,
                    "failed to record reachability check"
                );
            }

            if reachable {
                if let Err(err) = state.service.close_signal(&dedup_key, now).await {
                    error!(
                        deployment_id = %deployment.id,
                        %err,
                        "failed to close deployment reachability signal"
                    );
                }
            } else {
                let message = format!(
                    "Deployment {} at {} did not respond to health check",
                    deployment.name.0, hostname
                );

                let signal = Signal::open(
                    SignalId(Uuid::new_v4()),
                    SignalKind::DeploymentUnreachable,
                    SignalSubject::Deployment { id: deployment.id },
                    dedup_key,
                    message,
                    now,
                );

                if let Err(err) = state.service.write_signal(signal).await {
                    error!(
                        deployment_id = %deployment.id,
                        %err,
                        "failed to write deployment reachability signal"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autharie_core::deployments::DeploymentKind;

    #[test]
    fn health_path_is_correct_per_kind() {
        assert_eq!(
            health_path_for_kind(&DeploymentKind::Ferriskey),
            "/api/health/ready"
        );
        assert_eq!(
            health_path_for_kind(&DeploymentKind::Keycloak),
            "/health/ready"
        );
    }

    #[test]
    fn unset_via_probes_the_hostname_on_the_default_port() {
        let target = ProbeTarget::new("a.example.fr", "/api/health/ready", None);

        assert_eq!(target.url, "https://a.example.fr/api/health/ready");
    }

    #[test]
    fn via_keeps_the_hostname_and_moves_the_port_into_the_url() {
        let via: ProbeVia = "host.docker.internal:8444".parse().unwrap();
        let target = ProbeTarget::new("a.autharie.local", "/health/ready", Some(&via));

        assert_eq!(target.url, "https://a.autharie.local:8444/health/ready");
        assert_eq!(via.host(), "host.docker.internal");
        assert_eq!(via.port(), 8444);
    }

    #[test]
    fn via_accepts_an_ipv4_and_a_bracketed_ipv6_literal() {
        let v4: ProbeVia = "192.168.5.2:443".parse().unwrap();
        assert_eq!((v4.host(), v4.port()), ("192.168.5.2", 443));

        let v6: ProbeVia = "[::1]:8444".parse().unwrap();
        assert_eq!((v6.host(), v6.port()), ("::1", 8444));
    }

    #[test]
    fn via_rejects_what_is_not_host_and_port() {
        for bad in [
            "host",
            "host:",
            ":8444",
            "host:0",
            "host:99999",
            "host:abc",
            "::1:8444",
            "[::1]",
            "[::1:8444",
            "",
        ] {
            assert!(bad.parse::<ProbeVia>().is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn unset_settings_are_the_default_behaviour() {
        let args = crate::args::ReachabilityArgs::default();
        let settings = args.probe_settings().unwrap();

        assert!(settings.via.is_none());
        assert!(settings.ca.is_none());
    }

    #[test]
    fn invalid_settings_fail_with_the_variable_named() {
        let bad_via = crate::args::ReachabilityArgs {
            probe_via: "nope".to_string(),
            probe_ca_file: String::new(),
        };
        let err = bad_via.probe_settings().err().unwrap();
        assert!(err.starts_with("REACHABILITY_PROBE_VIA"), "{err}");

        let missing_ca = crate::args::ReachabilityArgs {
            probe_via: String::new(),
            probe_ca_file: "/nonexistent/ca.pem".to_string(),
        };
        let err = missing_ca.probe_settings().err().unwrap();
        assert!(err.starts_with("REACHABILITY_PROBE_CA_FILE"), "{err}");

        let dir = std::env::temp_dir().join(format!("probe-ca-{}", Uuid::new_v4()));
        std::fs::write(&dir, "not a certificate").unwrap();
        let junk_ca = crate::args::ReachabilityArgs {
            probe_via: String::new(),
            probe_ca_file: dir.to_string_lossy().into_owned(),
        };
        let err = junk_ca.probe_settings().err().unwrap();
        std::fs::remove_file(&dir).unwrap();
        assert!(err.starts_with("REACHABILITY_PROBE_CA_FILE"), "{err}");
    }

    #[test]
    fn probe_runs_every_interval_the_history_counts_in() {
        assert_eq!(EVERY.as_secs(), 60);
    }

    #[test]
    fn dedup_key_format_is_stable() {
        let id = Uuid::nil();
        let dedup_key = format!("deployment-unreachable-{}", id);

        assert_eq!(
            dedup_key,
            "deployment-unreachable-00000000-0000-0000-0000-000000000000"
        );
    }
}
