use std::time::Duration;

use autharie_core::AutharieService;
use tokio::time::interval;
use tracing::{error, info};

const EVERY: Duration = Duration::from_secs(30);

pub async fn purge_expired_iam_requests(service: AutharieService) {
    let mut ticker = interval(EVERY);

    loop {
        ticker.tick().await;

        match service.purge_expired_iam_requests().await {
            Ok(0) => {}
            Ok(purged) => info!(purged, "removed iam requests past their lifetime"),
            Err(err) => error!(%err, "failed to purge iam requests"),
        }
    }
}
