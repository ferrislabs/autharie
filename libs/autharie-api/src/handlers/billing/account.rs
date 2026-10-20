use autharie_auth::Identity;
use autharie_core::{
    CoreError,
    billing::{
        BillingAccount, BillingAddress, BillingAddressInput, BillingDetails, BillingEmail,
        BillingError, BillingName, PaymentMethodStatus, Siren, VatNumber,
    },
    organisation::OrganisationId,
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/billing/account")]
pub struct BillingAccountRoute {
    pub organisation_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct BillingAddressBody {
    pub line1: String,
    #[serde(default)]
    pub line2: Option<String>,
    pub postal_code: String,
    pub city: String,
    #[schema(example = "FR")]
    pub country: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BillingAccountRequest {
    pub name: String,
    pub email: String,
    pub address: BillingAddressBody,
    #[serde(default)]
    #[schema(example = "FR12345678901")]
    pub vat_number: Option<String>,
    #[serde(default)]
    #[schema(example = "123456789")]
    pub siren: Option<String>,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct BillingAccountResponse {
    pub name: String,
    pub email: String,
    pub address: BillingAddressBody,
    pub vat_number: Option<String>,
    pub siren: Option<String>,
    pub payment_method: PaymentMethodStatus,
    pub complete: bool,
}

impl From<&BillingAccount> for BillingAccountResponse {
    fn from(account: &BillingAccount) -> Self {
        let details = &account.details;

        Self {
            name: details.name.as_str().to_string(),
            email: details.email.as_str().to_string(),
            address: BillingAddressBody {
                line1: details.address.line1().to_string(),
                line2: details.address.line2().map(str::to_string),
                postal_code: details.address.postal_code().to_string(),
                city: details.address.city().to_string(),
                country: details.address.country().as_str().to_string(),
            },
            vat_number: details
                .vat_number
                .as_ref()
                .map(|vat_number| vat_number.as_str().to_string()),
            siren: details
                .siren
                .as_ref()
                .map(|siren| siren.as_str().to_string()),
            payment_method: account.payment_method,
            complete: account.is_complete(),
        }
    }
}

fn optional<T>(
    raw: Option<&str>,
    parse: impl FnOnce(&str) -> Result<T, BillingError>,
) -> Result<Option<T>, BillingError> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => parse(value).map(Some),
    }
}

impl TryFrom<&BillingAccountRequest> for BillingDetails {
    type Error = BillingError;

    fn try_from(request: &BillingAccountRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            name: BillingName::parse(&request.name)?,
            email: BillingEmail::parse(&request.email)?,
            address: BillingAddress::parse(BillingAddressInput {
                line1: &request.address.line1,
                line2: request.address.line2.as_deref(),
                postal_code: &request.address.postal_code,
                city: &request.address.city,
                country: &request.address.country,
            })?,
            vat_number: optional(request.vat_number.as_deref(), VatNumber::parse)?,
            siren: optional(request.siren.as_deref(), Siren::parse)?,
        })
    }
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/billing/account",
    summary = "read the billing account of an organisation",
    tag = "billing",
    description = "Who is invoiced, where, and whether a payment method is on file. The payment \
                   provider's own identifiers are never part of the answer.",
    params(BillingAccountRoute),
    responses(
        (status = 200, description = "The billing account", body = BillingAccountResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this organisation's billing", body = ApiError),
        (status = 404, description = "No such organisation, or it has no billing account yet", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_billing_account_handler(
    BillingAccountRoute { organisation_id }: BillingAccountRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<BillingAccountResponse>, ApiError> {
    let account = state
        .service
        .billing_account(identity, OrganisationId(organisation_id))
        .await?;

    Ok(Response::OK(BillingAccountResponse::from(&account)))
}

#[utoipa::path(
    put,
    path = "/{organisation_id}/billing/account",
    summary = "create or replace the billing account of an organisation",
    tag = "billing",
    description = "Replaces who is invoiced and where. The payment method is not touched: it is \
                   added through a payment method session. `vat_number` and `siren` are optional \
                   and a blank one is the same as none.",
    params(BillingAccountRoute),
    request_body = BillingAccountRequest,
    responses(
        (status = 200, description = "The billing account as stored", body = BillingAccountResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not manage this organisation's billing", body = ApiError),
        (status = 404, description = "No such organisation", body = ApiError),
        (status = 422, description = "A field is not valid", body = ApiError),
        (status = 502, description = "The payment provider failed", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn put_billing_account_handler(
    BillingAccountRoute { organisation_id }: BillingAccountRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<BillingAccountRequest>,
) -> Result<Response<BillingAccountResponse>, ApiError> {
    let details = BillingDetails::try_from(&request).map_err(CoreError::from)?;
    let provider = state.billing.as_deref().map(|runtime| &runtime.provider);

    let account = state
        .service
        .save_billing_account(identity, OrganisationId(organisation_id), details, provider)
        .await?;

    Ok(Response::OK(BillingAccountResponse::from(&account)))
}

#[cfg(test)]
mod tests {
    use autharie_core::billing::ProviderCustomerId;
    use serde_json::json;

    use super::*;

    fn request() -> BillingAccountRequest {
        serde_json::from_value(json!({
            "name": "  Acme SAS ",
            "email": "billing@acme.test",
            "address": {
                "line1": "1 rue de la Paix",
                "postal_code": "75002",
                "city": "Paris",
                "country": "fr"
            }
        }))
        .expect("a request")
    }

    #[test]
    fn a_request_without_vat_number_or_siren_has_neither() {
        let details = BillingDetails::try_from(&request()).expect("valid details");

        assert_eq!(details.name.as_str(), "Acme SAS");
        assert_eq!(details.address.country().as_str(), "FR");
        assert_eq!(details.address.line2(), None);
        assert_eq!(details.vat_number, None);
        assert_eq!(details.siren, None);
    }

    #[test]
    fn a_blank_vat_number_or_siren_is_none() {
        let details = BillingDetails::try_from(&BillingAccountRequest {
            vat_number: Some("  ".to_string()),
            siren: Some(String::new()),
            ..request()
        })
        .expect("valid details");

        assert_eq!(details.vat_number, None);
        assert_eq!(details.siren, None);
    }

    #[test]
    fn a_vat_number_and_a_siren_are_normalised() {
        let details = BillingDetails::try_from(&BillingAccountRequest {
            vat_number: Some("fr 12 345 678 901".to_string()),
            siren: Some("123 456 789".to_string()),
            ..request()
        })
        .expect("valid details");

        assert_eq!(
            details.vat_number.as_ref().map(VatNumber::as_str),
            Some("FR12345678901")
        );
        assert_eq!(details.siren.as_ref().map(Siren::as_str), Some("123456789"));
    }

    #[test]
    fn an_invalid_field_is_named() {
        let cases = [
            (
                BillingAccountRequest {
                    email: "nope".to_string(),
                    ..request()
                },
                "email",
            ),
            (
                BillingAccountRequest {
                    vat_number: Some("12".to_string()),
                    ..request()
                },
                "vat_number",
            ),
            (
                BillingAccountRequest {
                    siren: Some("12".to_string()),
                    ..request()
                },
                "siren",
            ),
        ];

        for (invalid, expected) in cases {
            assert!(
                matches!(
                    BillingDetails::try_from(&invalid),
                    Err(BillingError::InvalidBillingDetails { field, .. }) if field == expected
                ),
                "{expected}"
            );
        }
    }

    #[test]
    fn the_answer_carries_no_provider_identifier() {
        let details = BillingDetails::try_from(&request()).expect("valid details");
        let account = BillingAccount {
            customer: Some(ProviderCustomerId::parse("cus_secret").expect("a customer")),
            payment_method: PaymentMethodStatus::Valid,
            ..BillingAccount::new(OrganisationId::new(), details)
        };

        let answer = serde_json::to_string(&BillingAccountResponse::from(&account))
            .expect("a serializable answer");

        assert!(!answer.contains("cus_secret"), "{answer}");
        assert!(answer.contains("\"complete\":true"), "{answer}");
        assert!(answer.contains("\"payment_method\":\"valid\""), "{answer}");
    }
}
