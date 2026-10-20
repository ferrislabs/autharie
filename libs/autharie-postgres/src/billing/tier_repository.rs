use std::collections::BTreeSet;

use autharie_domain::{
    CoreError,
    billing::{Hosting, Money, Tier, TierCode, ports::TierRepository},
    deployments::DeploymentKind,
    offers::Offer,
    organisation::features::IamFeature,
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;

use super::columns::{failed, invalid, parsed};

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = Tier, backend = Postgres)]
pub struct PostgresTierRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresTierRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

struct TierRow {
    code: String,
    name: String,
    monthly_price_minor: i64,
    offer: String,
    features: Vec<String>,
    engines: Vec<String>,
    hostings: Vec<String>,
    selectable: bool,
}

impl TryFrom<TierRow> for Tier {
    type Error = CoreError;

    fn try_from(row: TierRow) -> Result<Self, Self::Error> {
        let features = row
            .features
            .iter()
            .map(|feature| parsed::<IamFeature>("billing_tiers.features", feature))
            .collect::<Result<BTreeSet<_>, _>>()?;

        let engines = row
            .engines
            .iter()
            .map(|engine| {
                DeploymentKind::try_from(engine.as_str())
                    .map_err(|reason| invalid("billing_tiers.engines", engine, reason))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let hostings = row
            .hostings
            .iter()
            .map(|hosting| parsed::<Hosting>("billing_tiers.hostings", hosting))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Tier {
            code: parsed::<TierCode>("billing_tiers.code", &row.code)?,
            name: row.name,
            monthly_price: Money::from_minor(row.monthly_price_minor),
            offer: parsed::<Offer>("billing_tiers.offer", &row.offer)?,
            features,
            engines,
            hostings,
            selectable: row.selectable,
        })
    }
}

impl TierRepository for PostgresTierRepository<'_> {
    async fn list(&self) -> Result<Vec<Tier>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                TierRow,
                r#"
                SELECT code, name, monthly_price_minor, offer, features, engines, hostings,
                       selectable
                FROM billing_tiers
                ORDER BY monthly_price_minor, code
                "#
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(failed("list the tiers"))?;

        rows.into_iter().map(Tier::try_from).collect()
    }

    async fn find(&self, code: &TierCode) -> Result<Option<Tier>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                TierRow,
                r#"
                SELECT code, name, monthly_price_minor, offer, features, engines, hostings,
                       selectable
                FROM billing_tiers
                WHERE code = $1
                "#,
                code.as_str()
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(failed("find the tier"))?;

        row.map(Tier::try_from).transpose()
    }
}
