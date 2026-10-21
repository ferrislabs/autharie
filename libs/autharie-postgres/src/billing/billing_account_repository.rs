use autharie_domain::{
    CoreError,
    billing::{
        BillingAccount, BillingAddress, BillingAddressInput, BillingDetails, BillingEmail,
        BillingName, PaymentMethodStatus, ProviderCustomerId, Siren, VatNumber,
        ports::BillingAccountRepository,
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

        Ok(BillingAccount {
            organisation_id: OrganisationId(row.organisation_id),
            details,
            customer,
            payment_method: parsed::<PaymentMethodStatus>(
                "billing_accounts.payment_method",
                &row.payment_method,
            )?,
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
                       city, country, vat_number, siren, provider_customer_id, payment_method
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
                       city, country, vat_number, siren, provider_customer_id, payment_method
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

        let mut tx = self.tx.lock().await;
        sqlx::query!(
            r#"
            INSERT INTO billing_accounts
                (organisation_id, name, email, address_line1, address_line2, postal_code, city,
                 country, vat_number, siren, provider_customer_id, payment_method)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
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
            payment_method
        )
        .execute(&mut ***tx)
        .await
        .map_err(failed("save the billing account"))?;

        Ok(())
    }
}
