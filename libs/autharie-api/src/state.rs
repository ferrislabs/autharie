use std::sync::Arc;

use autharie_core::{
    AutharieConfig, AutharieService, CloudProviders, PooledCredentialStore, create_service,
};
use autharie_transit::TransitKeyProvider;
use tracing::{error, warn};

use crate::{
    args::Args,
    certificate::KubeCertificateSource,
    errors::ApiError,
    quickwit::{QuickwitLogSearchIndex, QuickwitTraceSearchIndex},
};

#[derive(Clone)]
pub struct AppState {
    #[allow(unused)]
    pub args: Arc<Args>,

    #[allow(unused)]
    pub service: AutharieService,

    /// Where the current certificate can be read from, when this
    /// installation was given one. `None` the same way a missing
    /// `DnsProvider` is: nothing that reads this fails, it simply has
    /// nothing to distribute.
    pub certificate_source: Option<Arc<KubeCertificateSource>>,

    /// Where log search answers from, when this installation runs Quickwit.
    /// `None` for an installation that has not configured `--quickwit-url`:
    /// the search endpoint refuses plainly rather than the request failing to
    /// connect somewhere.
    pub quickwit_search: Option<Arc<QuickwitLogSearchIndex>>,

    /// Where trace search answers from -- the same Quickwit deployment as
    /// `quickwit_search`, under `traces-{organisation_id}` rather than
    /// `logs-{organisation_id}`, so it shares that field's `--quickwit-url`
    /// and its `None`-means-unconfigured shape.
    pub quickwit_traces: Option<Arc<QuickwitTraceSearchIndex>>,
}

pub fn customer_cloud_providers(
    args: &Args,
    service: &AutharieService,
    keys: Option<&TransitKeyProvider>,
) -> CloudProviders {
    if !args.customer_cloud.enabled {
        return CloudProviders::Unconfigured;
    }

    if let Some(blocker) = args.customer_cloud_blocker() {
        error!(
            blocker,
            "customer cloud is enabled but cannot run: it stays off"
        );
        return CloudProviders::Unconfigured;
    }

    let Some(keys) = keys else {
        error!(
            "customer cloud is enabled but the key manager client could not be built: it stays off"
        );
        return CloudProviders::Unconfigured;
    };

    let credentials = PooledCredentialStore::new(service.pool().clone(), Arc::new(keys.clone()));
    match CloudProviders::scaleway(&args.customer_cloud.scaleway_config(), credentials) {
        Ok(providers) => providers,
        Err(error) => {
            error!(%error, "the Scaleway client could not be built: customer cloud stays off");
            CloudProviders::Unconfigured
        }
    }
}

pub async fn state(args: Arc<Args>) -> Result<AppState, ApiError> {
    let config: AutharieConfig = AutharieConfig::from(args.as_ref().clone());
    let keys = args
        .key_manager
        .config()
        .and_then(|(config, _)| TransitKeyProvider::new(config).ok());

    let service = create_service(config)
        .await
        .map_err(|e| ApiError::InternalServerError {
            reason: e.to_string(),
        })?
        // The one administrative capability the control plane holds on the
        // realm. Given here rather than read from the environment deeper in,
        // so an installation that has none is visible in the wiring.
        .administering(args.realm.admin())
        .with_domain(args.ovh.domain())
        .with_credential_keys(keys.clone());
    let cloud = customer_cloud_providers(&args, &service, keys.as_ref());
    let service = service.with_cloud_providers(cloud);

    // Best-effort, like every other optional integration here: a cluster
    // this pod cannot reach, or a Secret that never turns up, means no
    // certificate is distributed -- not that the control plane fails to
    // start.
    let certificate_source = match args.certificate.configured() {
        Some((name, namespace)) => match KubeCertificateSource::from_env(name, namespace).await {
            Ok(source) => Some(Arc::new(source)),
            Err(error) => {
                warn!(%error, "the certificate source could not be built: no certificate will be distributed");
                None
            }
        },
        None => None,
    };

    let quickwit_search = args
        .quickwit
        .configured()
        .map(|url| Arc::new(QuickwitLogSearchIndex::new(url)));
    let quickwit_traces = args
        .quickwit
        .configured()
        .map(|url| Arc::new(QuickwitTraceSearchIndex::new(url)));

    Ok(AppState {
        args,
        service,
        certificate_source,
        quickwit_search,
        quickwit_traces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{Args, CustomerCloudArgs, DatabaseArgs, KeyManagerArgs, RealmArgs};
    use autharie_core::dataplane::{
        cloud_provider::{CatalogError, CredentialVerifier, Provider, ProviderCatalog},
        credential::{CloudCredentialId, SecretString},
        value_objects::Region,
    };
    use sqlx::postgres::PgPoolOptions;
    use std::sync::Arc;
    use tokio::time::{Duration, timeout};

    fn lazy_service() -> AutharieService {
        AutharieService::new(
            PgPoolOptions::new()
                .connect_lazy("postgres://user:pass@127.0.0.1:1/db")
                .expect("valid database url"),
        )
    }

    fn keys() -> TransitKeyProvider {
        let (config, _) = KeyManagerArgs::default().config().expect("wrapping is on");
        TransitKeyProvider::new(config).expect("a key manager client")
    }

    fn enabled_args() -> Args {
        Args {
            customer_cloud: CustomerCloudArgs {
                enabled: true,
                control_plane_url: "https://autharie.example".to_string(),
                ..CustomerCloudArgs::default()
            },
            realm: RealmArgs {
                admin_url: "http://realm.test".to_string(),
                admin_username: "admin".to_string(),
                ..RealmArgs::default()
            },
            ..Args::default()
        }
    }

    #[tokio::test]
    async fn with_customer_cloud_off_nothing_is_composed_and_creation_is_refused_readably() {
        let args = Args::default();
        let providers = customer_cloud_providers(&args, &lazy_service(), Some(&keys()));

        assert_eq!(providers, CloudProviders::Unconfigured);
        assert!(!args.customer_cloud_active());

        let refused = providers
            .offers(
                Provider::Scaleway,
                &CloudCredentialId(uuid::Uuid::new_v4()),
                &Region::new("fr-par"),
            )
            .await
            .expect_err("refused");
        assert!(matches!(&refused, CatalogError::Unavailable(reason)
            if reason == "no cloud provider is configured on this installation"));

        let verified = providers
            .verify(Provider::Scaleway, &SecretString::new("key"))
            .await;
        assert!(verified.is_err());
    }

    #[tokio::test]
    async fn enabled_without_its_prerequisites_stays_off() {
        let mut args = enabled_args();
        args.customer_cloud.control_plane_url = String::new();

        let providers = customer_cloud_providers(&args, &lazy_service(), Some(&keys()));

        assert_eq!(providers, CloudProviders::Unconfigured);
        assert!(!args.customer_cloud_active());
    }

    #[tokio::test]
    async fn enabled_with_its_prerequisites_composes_scaleway() {
        let args = enabled_args();

        let providers = customer_cloud_providers(&args, &lazy_service(), Some(&keys()));

        assert!(matches!(providers, CloudProviders::Scaleway(_)));
        assert!(args.customer_cloud_active());
    }

    #[tokio::test]
    async fn state_returns_error_on_invalid_db() {
        let args = Args {
            db: DatabaseArgs {
                host: "127.0.0.1".to_string(),
                port: 1,
                ..DatabaseArgs::default()
            },
            ..Args::default()
        };

        let result = timeout(Duration::from_millis(200), state(Arc::new(args))).await;
        assert!(matches!(result, Ok(Err(ApiError::InternalServerError { .. }))) || result.is_err());
    }
}
