use std::collections::HashMap;

use autharie_domain::{
    CoreError,
    offers::{
        Offer,
        ports::{OfferChange, OfferOverrideRepository},
    },
    organisation::OrganisationId,
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = OfferOverride, backend = Postgres)]
pub struct PostgresOfferOverrideRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresOfferOverrideRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

impl OfferOverrideRepository for PostgresOfferOverrideRepository<'_> {
    async fn overrides(
        &self,
        organisation_id: OrganisationId,
    ) -> Result<HashMap<Offer, bool>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                "SELECT offer, open FROM organisation_offer_overrides WHERE organisation_id = $1",
                organisation_id.0
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to read the offer overrides: {e}"),
        })?;

        Ok(rows
            .into_iter()
            .filter_map(|row| Some((row.offer.parse::<Offer>().ok()?, row.open)))
            .collect())
    }

    async fn set_override(
        &self,
        organisation_id: OrganisationId,
        offer: Offer,
        open: Option<bool>,
        change: &OfferChange,
    ) -> Result<(), CoreError> {
        let mut tx = self.tx.lock().await;
        match open {
            Some(open) => {
                sqlx::query!(
                    r#"
                    INSERT INTO organisation_offer_overrides
                        (organisation_id, offer, open, updated_at, updated_by)
                    VALUES ($1, $2, $3, $4, $5)
                    ON CONFLICT (organisation_id, offer) DO UPDATE
                        SET open = EXCLUDED.open,
                            updated_at = EXCLUDED.updated_at,
                            updated_by = EXCLUDED.updated_by
                    "#,
                    organisation_id.0,
                    offer.to_string(),
                    open,
                    change.at,
                    change.by
                )
                .execute(&mut ***tx)
                .await
            }
            None => {
                sqlx::query!(
                    "DELETE FROM organisation_offer_overrides \
                     WHERE organisation_id = $1 AND offer = $2",
                    organisation_id.0,
                    offer.to_string()
                )
                .execute(&mut ***tx)
                .await
            }
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to set the offer override: {e}"),
        })?;

        Ok(())
    }
}
