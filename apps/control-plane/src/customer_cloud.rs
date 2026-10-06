use std::{sync::Arc, time::Duration};

use autharie_api::{args::Args, state::AppState};
use autharie_core::{
    FerrisKeyHeraldIdentities, HelmBootstrapper, PooledCredentialStore, PooledDataPlanes,
    PooledInventory, PostgresClusterQueue, ScalewayProvisioner,
    customer_clusters::{CustomerClusterWorker, ProvisionReport, TeardownReport},
};
use autharie_transit::TransitKeyProvider;
use tokio::time::{MissedTickBehavior, interval};
use tracing::{error, info};

pub async fn run_customer_cluster_worker(args: Arc<Args>, state: AppState) {
    if !args.customer_cloud_active() {
        return;
    }

    let Some(worker) = build_worker(&args, &state) else {
        return;
    };

    let every = args.customer_cloud.worker_interval();
    info!(
        seconds = every.as_secs(),
        "building and releasing customer clusters"
    );

    tokio::join!(
        provision_loop(&worker, every),
        teardown_loop(&worker, every)
    );
}

fn ticker(every: Duration) -> tokio::time::Interval {
    let mut ticker = interval(every);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker
}

async fn provision_loop(worker: &Worker, every: Duration) {
    let mut ticker = ticker(every);

    loop {
        ticker.tick().await;

        match worker.provision_pending().await {
            Ok(report) if report == ProvisionReport::default() => {}
            Ok(report) => info!(?report, "provisioning customer clusters finished a pass"),
            Err(err) => error!(%err, "failed to provision customer clusters"),
        }
    }
}

async fn teardown_loop(worker: &Worker, every: Duration) {
    let mut ticker = ticker(every);

    loop {
        ticker.tick().await;

        match worker.teardown_released().await {
            Ok(report) if report == TeardownReport::default() => {}
            Ok(report) => info!(?report, "releasing customer clusters finished a pass"),
            Err(err) => error!(%err, "failed to release customer clusters"),
        }
    }
}

type Worker = CustomerClusterWorker<
    PostgresClusterQueue,
    ScalewayProvisioner<
        PooledCredentialStore,
        PooledInventory,
        PooledDataPlanes,
        HelmBootstrapper<FerrisKeyHeraldIdentities, PooledDataPlanes>,
    >,
    FerrisKeyHeraldIdentities,
>;

fn build_worker(args: &Args, state: &AppState) -> Option<Worker> {
    let pool = state.service.pool().clone();

    let keys = args
        .key_manager
        .config()
        .and_then(|(config, _)| TransitKeyProvider::new(config).ok())?;
    let realm = args.realm.admin()?;

    let bootstrapper = HelmBootstrapper::new(
        FerrisKeyHeraldIdentities::new(realm.clone()),
        PooledDataPlanes::new(pool.clone()),
        args.customer_cloud.helm_config(&args.auth),
    );
    let provisioner = match ScalewayProvisioner::new(
        args.customer_cloud.scaleway_config(),
        PooledCredentialStore::new(pool.clone(), Arc::new(keys)),
        PooledInventory::new(pool.clone()),
        PooledDataPlanes::new(pool.clone()),
        bootstrapper,
    ) {
        Ok(provisioner) => provisioner,
        Err(err) => {
            error!(%err, "the Scaleway client could not be built: customer clusters are not built");
            return None;
        }
    };

    Some(CustomerClusterWorker::new(
        PostgresClusterQueue::new(pool),
        provisioner,
        FerrisKeyHeraldIdentities::new(realm),
        args.customer_cloud.worker_settings(),
    ))
}

#[cfg(test)]
mod tests {
    use autharie_api::args::CustomerCloudArgs;
    use autharie_core::AutharieService;
    use sqlx::postgres::PgPoolOptions;
    use tokio::time::timeout;

    use super::*;

    fn state(args: &Arc<Args>) -> AppState {
        AppState {
            args: args.clone(),
            service: AutharieService::new(
                PgPoolOptions::new()
                    .connect_lazy("postgres://user:pass@127.0.0.1:1/db")
                    .expect("valid database url"),
            ),
            certificate_source: None,
            quickwit_search: None,
            quickwit_traces: None,
        }
    }

    #[tokio::test]
    async fn with_customer_cloud_off_the_worker_returns_at_once_and_runs_nothing() {
        let args = Arc::new(Args::default());

        let finished = timeout(
            Duration::from_millis(500),
            run_customer_cluster_worker(args.clone(), state(&args)),
        )
        .await;

        assert!(finished.is_ok());
        assert!(!args.customer_cloud_active());
    }

    #[tokio::test]
    async fn enabled_without_its_prerequisites_the_worker_returns_at_once() {
        let args = Arc::new(Args {
            customer_cloud: CustomerCloudArgs {
                enabled: true,
                ..CustomerCloudArgs::default()
            },
            ..Args::default()
        });

        let finished = timeout(
            Duration::from_millis(500),
            run_customer_cluster_worker(args.clone(), state(&args)),
        )
        .await;

        assert!(finished.is_ok());
        assert!(!args.customer_cloud_active());
    }

    #[tokio::test]
    async fn enabled_with_its_prerequisites_the_worker_keeps_running() {
        let args = Arc::new(Args {
            customer_cloud: CustomerCloudArgs {
                enabled: true,
                control_plane_url: "https://autharie.example".to_string(),
                ..CustomerCloudArgs::default()
            },
            realm: autharie_api::args::RealmArgs {
                admin_url: "http://realm.test".to_string(),
                admin_username: "admin".to_string(),
                ..Default::default()
            },
            ..Args::default()
        });

        let finished = timeout(
            Duration::from_millis(300),
            run_customer_cluster_worker(args.clone(), state(&args)),
        )
        .await;

        assert!(finished.is_err());
    }
}
