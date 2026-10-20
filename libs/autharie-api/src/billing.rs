use autharie_core::billing::BillingPeriod;
use autharie_stripe::{StripeConfig, StripePaymentProvider};

use crate::{args::BillingArgs, errors::ApiError};

#[derive(Debug)]
pub struct BillingRuntime {
    pub provider: StripePaymentProvider,
    pub first_period: BillingPeriod,
    pub return_url: String,
}

fn misconfigured(reason: String) -> ApiError {
    ApiError::InternalServerError { reason }
}

fn required<'a>(value: Option<&'a str>, variable: &str) -> Result<&'a str, ApiError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| misconfigured(format!("billing is enabled but {variable} is not set")))
}

impl BillingRuntime {
    pub fn from_args(args: &BillingArgs) -> Result<Option<Self>, ApiError> {
        if !args.enabled {
            return Ok(None);
        }

        let first_period = required(args.first_period.as_deref(), "BILLING_FIRST_PERIOD")?
            .parse::<BillingPeriod>()
            .map_err(|_| {
                misconfigured(
                    "BILLING_FIRST_PERIOD is not a billing period, expected YYYY-MM".into(),
                )
            })?;
        let return_url = required(args.return_url.as_deref(), "BILLING_RETURN_URL")?;
        let secret_key = required(args.stripe_secret_key.as_deref(), "STRIPE_SECRET_KEY")?;
        let webhook_secret = required(
            args.stripe_webhook_secret.as_deref(),
            "STRIPE_WEBHOOK_SECRET",
        )?;

        let provider = StripePaymentProvider::new(StripeConfig {
            secret_key: secret_key.into(),
            webhook_secret: webhook_secret.into(),
            api_base: args.stripe_api_base.clone(),
        })
        .map_err(|error| {
            misconfigured(format!(
                "the Stripe client could not be built from STRIPE_API_BASE: {error}"
            ))
        })?;

        Ok(Some(Self {
            provider,
            first_period,
            return_url: return_url.to_string(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET_KEY: &str = "sk_test_do_not_print";
    const WEBHOOK_SECRET: &str = "whsec_do_not_print";

    fn enabled() -> BillingArgs {
        BillingArgs {
            enabled: true,
            first_period: Some("2026-12".to_string()),
            return_url: Some("https://console.test/billing".to_string()),
            stripe_secret_key: Some(SECRET_KEY.to_string()),
            stripe_webhook_secret: Some(WEBHOOK_SECRET.to_string()),
            ..BillingArgs::default()
        }
    }

    fn reason(error: ApiError) -> String {
        match error {
            ApiError::InternalServerError { reason } => reason,
            other => panic!("expected an internal server error, got {other:?}"),
        }
    }

    #[test]
    fn disabled_billing_builds_nothing_whatever_else_is_missing() {
        let args = BillingArgs {
            enabled: false,
            ..BillingArgs::default()
        };

        assert!(
            BillingRuntime::from_args(&args)
                .expect("no error")
                .is_none()
        );
        assert!(
            BillingRuntime::from_args(&BillingArgs::default())
                .expect("no error")
                .is_none()
        );
    }

    #[test]
    fn enabled_billing_with_everything_builds_the_runtime() {
        let runtime = BillingRuntime::from_args(&enabled())
            .expect("a runtime")
            .expect("billing is enabled");

        assert_eq!(
            runtime.first_period,
            BillingPeriod::new(2026, 12).expect("a period")
        );
        assert_eq!(runtime.return_url, "https://console.test/billing");
    }

    #[test]
    fn surrounding_whitespace_is_not_part_of_a_value() {
        let args = BillingArgs {
            first_period: Some(" 2026-12\n".to_string()),
            return_url: Some(" https://console.test/billing ".to_string()),
            stripe_secret_key: Some(format!("{SECRET_KEY}\n")),
            ..enabled()
        };

        let runtime = BillingRuntime::from_args(&args)
            .expect("a runtime")
            .expect("billing is enabled");

        assert_eq!(
            runtime.first_period,
            BillingPeriod::new(2026, 12).expect("a period")
        );
        assert_eq!(runtime.return_url, "https://console.test/billing");
    }

    #[test]
    fn every_missing_or_blank_piece_is_named_by_its_variable() {
        let broken: [(&str, BillingArgs); 8] = [
            (
                "BILLING_FIRST_PERIOD",
                BillingArgs {
                    first_period: None,
                    ..enabled()
                },
            ),
            (
                "BILLING_FIRST_PERIOD",
                BillingArgs {
                    first_period: Some("  ".to_string()),
                    ..enabled()
                },
            ),
            (
                "BILLING_RETURN_URL",
                BillingArgs {
                    return_url: None,
                    ..enabled()
                },
            ),
            (
                "BILLING_RETURN_URL",
                BillingArgs {
                    return_url: Some(" ".to_string()),
                    ..enabled()
                },
            ),
            (
                "STRIPE_SECRET_KEY",
                BillingArgs {
                    stripe_secret_key: None,
                    ..enabled()
                },
            ),
            (
                "STRIPE_SECRET_KEY",
                BillingArgs {
                    stripe_secret_key: Some(String::new()),
                    ..enabled()
                },
            ),
            (
                "STRIPE_WEBHOOK_SECRET",
                BillingArgs {
                    stripe_webhook_secret: None,
                    ..enabled()
                },
            ),
            (
                "STRIPE_WEBHOOK_SECRET",
                BillingArgs {
                    stripe_webhook_secret: Some("\t".to_string()),
                    ..enabled()
                },
            ),
        ];

        for (variable, args) in broken {
            let error = BillingRuntime::from_args(&args).expect_err(variable);

            assert!(reason(error).contains(variable), "{variable}");
        }
    }

    #[test]
    fn a_first_period_that_is_not_a_month_is_refused_without_echoing_it() {
        let args = BillingArgs {
            first_period: Some("december".to_string()),
            ..enabled()
        };

        let reason = reason(BillingRuntime::from_args(&args).expect_err("not a month"));

        assert!(reason.contains("BILLING_FIRST_PERIOD"), "{reason}");
        assert!(!reason.contains("december"), "{reason}");
    }

    #[test]
    fn no_failure_ever_carries_a_secret() {
        let unusable = BillingArgs {
            stripe_api_base: "not a url".to_string(),
            ..enabled()
        };
        let incomplete = BillingArgs {
            stripe_webhook_secret: None,
            ..enabled()
        };

        for args in [unusable, incomplete] {
            let reason = reason(BillingRuntime::from_args(&args).expect_err("refused"));

            assert!(!reason.contains(SECRET_KEY), "{reason}");
            assert!(!reason.contains(WEBHOOK_SECRET), "{reason}");
        }
    }

    #[test]
    fn an_unusable_stripe_base_is_named_by_its_variable() {
        let args = BillingArgs {
            stripe_api_base: "not a url".to_string(),
            ..enabled()
        };

        let reason = reason(BillingRuntime::from_args(&args).expect_err("unusable"));

        assert!(reason.contains("STRIPE_API_BASE"), "{reason}");
    }
}
