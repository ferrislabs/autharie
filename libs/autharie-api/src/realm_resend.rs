use std::time::Duration;

use tokio::time::interval;
use tracing::{error, info, warn};

use crate::state::AppState;

const EVERY: Duration = Duration::from_secs(5 * 60);

pub async fn resend_stalled_realms(state: AppState) {
    info!("resending the requests of realms that stalled in a cell");

    let mut ticker = interval(EVERY);

    loop {
        ticker.tick().await;

        let resent = match state.service.resend_stalled_realms().await {
            Ok(resent) => resent,
            Err(err) => {
                error!(%err, "failed to resend the requests of stalled realms");
                continue;
            }
        };

        for action in &resent.recorded {
            info!(
                deployment_id = ?action.deployment_id,
                action_type = %action.action_type.0,
                "resent the request of a realm that stalled"
            );
        }
        for (deployment_id, err) in &resent.refused {
            warn!(%err, deployment_id = %deployment_id.0, "could not resend the request of a stalled realm");
        }
    }
}
