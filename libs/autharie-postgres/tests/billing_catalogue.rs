use std::collections::BTreeSet;

use autharie_domain::{
    CoreError,
    billing::{Hosting, Money, Tier, TierCode, ports::TierRepository},
    deployments::DeploymentKind,
    offers::Offer,
    organisation::features::IamFeature,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::billing::PostgresTierRepository;

mod support;
use support::pool;

macro_rules! pool_or_skip {
    () => {
        match pool().await {
            Some(pool) => pool,
            None => {
                eprintln!("skipped: DATABASE_URL is not set");
                return;
            }
        }
    };
}

fn map_err(e: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: e.to_string(),
    }
}

async fn scratch<T>(work: impl AsyncFnOnce(SharedTx<'_>) -> Result<T, CoreError>) -> Option<T> {
    let pool = pool().await?;

    Some(
        in_scratch_tx(&pool, map_err, work)
            .await
            .expect("the scratch transaction ran"),
    )
}

fn legacy(tiers: &[Tier]) -> Vec<&Tier> {
    tiers
        .iter()
        .filter(|tier| tier.code.as_str().starts_with("legacy-"))
        .collect()
}

#[tokio::test]
async fn the_seed_lists_the_five_legacy_tiers_with_every_feature() {
    let _pool = pool_or_skip!();

    let tiers = scratch(async |tx| PostgresTierRepository::new(&tx).list().await)
        .await
        .expect("the database is set");

    let legacy = legacy(&tiers);
    let codes: Vec<&str> = legacy.iter().map(|tier| tier.code.as_str()).collect();
    assert_eq!(
        codes,
        [
            "legacy-free",
            "legacy-private",
            "legacy-sandbox",
            "legacy-scale",
            "legacy-standard"
        ]
    );

    let every_feature: BTreeSet<IamFeature> = IamFeature::ALL.into_iter().collect();
    for tier in legacy {
        assert_eq!(tier.features, every_feature, "{}", tier.code);
        assert_eq!(tier.monthly_price, Money::ZERO, "{}", tier.code);
        assert_eq!(
            tier.engines,
            [DeploymentKind::Ferriskey, DeploymentKind::Keycloak],
            "{}",
            tier.code
        );
        assert_eq!(tier.hostings, Hosting::ALL, "{}", tier.code);
        assert!(!tier.selectable, "{}", tier.code);
        assert!(!tier.is_billable(), "{}", tier.code);
    }
}

#[tokio::test]
async fn every_legacy_tier_carries_the_offer_it_is_named_after() {
    let _pool = pool_or_skip!();

    let found = scratch(async |tx| {
        let tiers = PostgresTierRepository::new(&tx);
        let mut found = Vec::new();

        for (offer, name) in [
            (Offer::Free, "Free (legacy)"),
            (Offer::Sandbox, "Sandbox (legacy)"),
            (Offer::Standard, "Standard (legacy)"),
            (Offer::Scale, "Scale (legacy)"),
            (Offer::Private, "Private (legacy)"),
        ] {
            found.push((offer, name, tiers.find(&TierCode::legacy(offer)).await?));
        }

        Ok(found)
    })
    .await
    .expect("the database is set");

    for (offer, name, tier) in found {
        let tier = tier.unwrap_or_else(|| panic!("legacy-{offer} is seeded"));
        assert_eq!(tier.offer, offer);
        assert_eq!(tier.code, TierCode::legacy(offer));
        assert_eq!(tier.name, name);
    }
}

#[tokio::test]
async fn there_is_no_internal_tier() {
    let _pool = pool_or_skip!();

    let (internal, tiers) = scratch(async |tx| {
        let tiers = PostgresTierRepository::new(&tx);
        Ok((
            tiers.find(&TierCode::parse("internal").unwrap()).await?,
            tiers.list().await?,
        ))
    })
    .await
    .expect("the database is set");

    assert_eq!(internal, None);
    assert!(tiers.iter().all(|tier| tier.code.as_str() != "internal"));
}

#[tokio::test]
async fn tiers_are_listed_cheapest_first_then_by_code() {
    let _pool = pool_or_skip!();

    let tiers = scratch(async |tx| {
        {
            let mut guard = tx.lock().await;
            for (code, price) in [("zeta", 900), ("alpha", 900), ("cheap", 100)] {
                sqlx::query(
                    "INSERT INTO billing_tiers \
                     (code, name, monthly_price_minor, offer, features, engines, hostings) \
                     VALUES ($1, $1, $2, 'standard', '{mfa}', '{ferriskey}', '{shared}')",
                )
                .bind(code)
                .bind(price)
                .execute(&mut ***guard)
                .await
                .map_err(map_err)?;
            }
        }

        PostgresTierRepository::new(&tx).list().await
    })
    .await
    .expect("the database is set");

    let order: Vec<(i64, &str)> = tiers
        .iter()
        .map(|tier| (tier.monthly_price.minor(), tier.code.as_str()))
        .collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);

    let added: Vec<&str> = order
        .iter()
        .filter(|(price, _)| *price > 0)
        .map(|(_, code)| *code)
        .collect();
    assert_eq!(added, ["cheap", "alpha", "zeta"]);
}

#[tokio::test]
async fn a_selectable_paid_tier_is_read_back_whole() {
    let _pool = pool_or_skip!();

    let found = scratch(async |tx| {
        {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO billing_tiers \
                 (code, name, monthly_price_minor, offer, features, engines, hostings, \
                  selectable) \
                 VALUES ('standard-2026', 'Standard', 4900, 'standard', '{mfa,branding}', \
                         '{ferriskey}', '{shared,self_hosted}', true)",
            )
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
        }

        PostgresTierRepository::new(&tx)
            .find(&TierCode::parse("standard-2026").unwrap())
            .await
    })
    .await
    .expect("the database is set");

    assert_eq!(
        found,
        Some(Tier {
            code: TierCode::parse("standard-2026").unwrap(),
            name: "Standard".to_string(),
            monthly_price: Money::from_minor(4900),
            offer: Offer::Standard,
            features: BTreeSet::from([IamFeature::Mfa, IamFeature::Branding]),
            engines: vec![DeploymentKind::Ferriskey],
            hostings: vec![Hosting::Shared, Hosting::SelfHosted],
            selectable: true,
        })
    );
}

#[tokio::test]
async fn a_stored_value_the_domain_does_not_know_is_a_database_error_naming_the_column() {
    let _pool = pool_or_skip!();

    for (column, features, engines, hostings) in [
        (
            "billing_tiers.features",
            "{bogus}",
            "{ferriskey}",
            "{shared}",
        ),
        ("billing_tiers.engines", "{mfa}", "{bogus}", "{shared}"),
        ("billing_tiers.hostings", "{mfa}", "{ferriskey}", "{bogus}"),
    ] {
        let outcome = scratch(async |tx| {
            {
                let mut guard = tx.lock().await;
                sqlx::query(
                    "INSERT INTO billing_tiers \
                     (code, name, monthly_price_minor, offer, features, engines, hostings) \
                     VALUES ('broken', 'Broken', 0, 'standard', $1::text[], $2::text[], \
                             $3::text[])",
                )
                .bind(features)
                .bind(engines)
                .bind(hostings)
                .execute(&mut ***guard)
                .await
                .map_err(map_err)?;
            }

            Ok(PostgresTierRepository::new(&tx)
                .find(&TierCode::parse("broken").unwrap())
                .await)
        })
        .await
        .expect("the database is set");

        match outcome {
            Err(CoreError::DatabaseError { message }) => {
                assert!(message.contains(column), "{message}");
            }
            other => panic!("expected a database error for {column}, got {other:?}"),
        }
    }
}
