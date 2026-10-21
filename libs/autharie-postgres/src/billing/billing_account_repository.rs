use autharie_domain::{
    CoreError,
    billing::{
        BillingAccount, BillingAddress, BillingAddressInput, BillingDetails, BillingEmail,
        BillingName, CardSummary, CardSummaryInput, PaymentMethodStatus, ProviderCustomerId, Siren,
        VatNumber, ports::BillingAccountRepository,
    },
    organisation::OrganisationId,
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use uuid::Uuid;

use super::columns::{failed, invalid, parsed};

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = BillingAccount, backend = Postgres)]
pub struct PostgresBillingAccountRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresBillingAccountRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

struct AccountRow {
    organisation_id: Uuid,
    name: String,
    email: String,
    address_line1: String,
    address_line2: Option<String>,
    postal_code: String,
    city: String,
    country: String,
    vat_number: Option<String>,
    siren: Option<String>,
    provider_customer_id: Option<String>,
    payment_method: String,
    card_brand: Option<String>,
    card_last4: Option<String>,
    card_exp_month: Option<i16>,
    card_exp_year: Option<i16>,
}

impl TryFrom<AccountRow> for BillingAccount {
    type Error = CoreError;

    fn try_from(row: AccountRow) -> Result<Self, Self::Error> {
        let address = BillingAddress::parse(BillingAddressInput {
            line1: &row.address_line1,
            line2: row.address_line2.as_deref(),
            postal_code: &row.postal_code,
            city: &row.city,
            country: &row.country,
        })
        .map_err(|reason| invalid("billing_accounts.address", &row.address_line1, reason))?;

        let details = BillingDetails {
            name: BillingName::parse(&row.name)
                .map_err(|reason| invalid("billing_accounts.name", &row.name, reason))?,
            email: BillingEmail::parse(&row.email)
                .map_err(|reason| invalid("billing_accounts.email", &row.email, reason))?,
            address,
            vat_number: row
                .vat_number
                .as_deref()
                .map(|value| {
                    VatNumber::parse(value)
                        .map_err(|reason| invalid("billing_accounts.vat_number", value, reason))
                })
                .transpose()?,
            siren: row
                .siren
                .as_deref()
                .map(|value| {
                    Siren::parse(value)
                        .map_err(|reason| invalid("billing_accounts.siren", value, reason))
                })
                .transpose()?,
        };

        let customer = row
            .provider_customer_id
            .as_deref()
            .map(|value| {
                ProviderCustomerId::parse(value).map_err(|reason| {
                    invalid("billing_accounts.provider_customer_id", value, reason)
                })
            })
            .transpose()?;

        let card = match (
            row.card_brand.as_deref(),
            row.card_last4.as_deref(),
            row.card_exp_month,
            row.card_exp_year,
        ) {
            (Some(brand), Some(last4), Some(exp_month), Some(exp_year)) => Some(
                CardSummary::parse(CardSummaryInput {
                    brand,
                    last4,
                    exp_month: u32::try_from(exp_month).unwrap_or(u32::MAX),
                    exp_year: u32::try_from(exp_year).unwrap_or(u32::MAX),
                })
                .map_err(|reason| invalid("billing_accounts.card_last4", last4, reason))?,
            ),
            (None, None, None, None) => None,
            _ => {
                return Err(invalid(
                    "billing_accounts.card_brand",
                    row.card_brand.as_deref().unwrap_or_default(),
                    "the card is only partly stored",
                ));
            }
        };

        Ok(BillingAccount {
            organisation_id: OrganisationId(row.organisation_id),
            details,
            customer,
            payment_method: parsed::<PaymentMethodStatus>(
                "billing_accounts.payment_method",
                &row.payment_method,
            )?,
            card,
        })
    }
}

impl BillingAccountRepository for PostgresBillingAccountRepository<'_> {
    async fn find(
        &self,
        organisation_id: &OrganisationId,
    ) -> Result<Option<BillingAccount>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                AccountRow,
                r#"
                SELECT organisation_id, name, email, address_line1, address_line2, postal_code,
                       city, country, vat_number, siren, provider_customer_id, payment_method,
                       card_brand, card_last4, card_exp_month, card_exp_year
                FROM billing_accounts
                WHERE organisation_id = $1
                FOR UPDATE
                "#,
                organisation_id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(failed("find the billing account"))?;

        row.map(BillingAccount::try_from).transpose()
    }

    async fn find_by_customer(
        &self,
        customer: &ProviderCustomerId,
    ) -> Result<Option<BillingAccount>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                AccountRow,
                r#"
                SELECT organisation_id, name, email, address_line1, address_line2, postal_code,
                       city, country, vat_number, siren, provider_customer_id, payment_method,
                       card_brand, card_last4, card_exp_month, card_exp_year
                FROM billing_accounts
                WHERE provider_customer_id = $1
                FOR UPDATE
                "#,
                customer.as_str()
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(failed("find the billing account by customer"))?;

        row.map(BillingAccount::try_from).transpose()
    }

    async fn save(&self, account: &BillingAccount) -> Result<(), CoreError> {
        let details = &account.details;
        let payment_method = account.payment_method.to_string();
        let card = account.card.as_ref();

        let mut tx = self.tx.lock().await;
        sqlx::query!(
            r#"
            INSERT INTO billing_accounts
                (organisation_id, name, email, address_line1, address_line2, postal_code, city,
                 country, vat_number, siren, provider_customer_id, payment_method,
                 card_brand, card_last4, card_exp_month, card_exp_year)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ON CONFLICT (organisation_id) DO UPDATE
                SET name = EXCLUDED.name,
                    email = EXCLUDED.email,
                    address_line1 = EXCLUDED.address_line1,
                    address_line2 = EXCLUDED.address_line2,
                    postal_code = EXCLUDED.postal_code,
                    city = EXCLUDED.city,
                    country = EXCLUDED.country,
                    vat_number = EXCLUDED.vat_number,
                    siren = EXCLUDED.siren,
                    provider_customer_id = EXCLUDED.provider_customer_id,
                    payment_method = EXCLUDED.payment_method,
                    card_brand = EXCLUDED.card_brand,
                    card_last4 = EXCLUDED.card_last4,
                    card_exp_month = EXCLUDED.card_exp_month,
                    card_exp_year = EXCLUDED.card_exp_year,
                    updated_at = now()
            "#,
            account.organisation_id.0,
            details.name.as_str(),
            details.email.as_str(),
            details.address.line1(),
            details.address.line2(),
            details.address.postal_code(),
            details.address.city(),
            details.address.country().as_str(),
            details.vat_number.as_ref().map(VatNumber::as_str),
            details.siren.as_ref().map(Siren::as_str),
            account.customer.as_ref().map(ProviderCustomerId::as_str),
            payment_method,
            card.map(CardSummary::brand),
            card.map(CardSummary::last4),
            card.map(|card| i16::from(card.exp_month())),
            card.map(|card| i16::try_from(card.exp_year()).unwrap_or(i16::MAX))
        )
        .execute(&mut ***tx)
        .await
        .map_err(failed("save the billing account"))?;

        Ok(())
    }
}
