use std::time::Duration;

use chrono::Utc;
use tokio::time::interval;
use tracing::{error, info};

use crate::state::AppState;

const EVERY: Duration = Duration::from_secs(60 * 60);

pub async fn close_billing_periods(state: AppState) {
    let Some(runtime) = state.billing.as_deref() else {
        info!("billing is not enabled: no billing period will be closed");
        return;
    };

    info!(first_period = %runtime.first_period, "closing billing periods");

    let mut ticker = interval(EVERY);

    loop {
        ticker.tick().await;

        match state
            .service
            .run_billing(&runtime.provider, runtime.first_period, Utc::now())
            .await
        {
            Ok(run) if run.closed + run.sent + run.failed == 0 => {}
            Ok(run) => info!(
                closed = run.closed,
                sent = run.sent,
                failed = run.failed,
                "billing run finished"
            ),
            Err(error) => error!(%error, "billing run failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::time::timeout;

    use super::*;
    use crate::{args::BillingArgs, billing::BillingRuntime, test_helpers::app_state};

    #[tokio::test]
    async fn without_billing_the_job_returns_at_once() {
        let state = app_state();
        assert!(state.billing.is_none());

        let finished = timeout(Duration::from_millis(500), close_billing_periods(state)).await;

        assert!(finished.is_ok());
    }

    #[tokio::test]
    async fn with_billing_the_job_keeps_running_even_when_a_run_fails() {
        let runtime = BillingRuntime::from_args(&BillingArgs {
            enabled: true,
            first_period: Some("2026-12".to_string()),
            return_url: Some("https://console.test/billing".to_string()),
            stripe_secret_key: Some("sk_test_x".to_string()),
            stripe_webhook_secret: Some("whsec_x".to_string()),
            ..BillingArgs::default()
        })
        .expect("a runtime")
        .expect("billing is enabled");
        let state = AppState {
            billing: Some(Arc::new(runtime)),
            ..app_state()
        };

        let finished = timeout(Duration::from_millis(300), close_billing_periods(state)).await;

        assert!(finished.is_err());
    }
}
