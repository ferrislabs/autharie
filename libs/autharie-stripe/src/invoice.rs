use autharie_domain::billing::{
    BillingError, ProviderCustomerId, ProviderInvoice, ProviderInvoiceId, Statement, StatementLine,
};
use reqwest::Method;

use crate::{
    CURRENCY,
    client::{Client, failure, https_url, malformed},
    form::Form,
    wire::{Created, Finalized, Found, InvoiceStatus, Search},
};

const INVOICES: &str = "/v1/invoices";
const INVOICE_SEARCH: &str = "/v1/invoices/search";
const INVOICE_ITEMS: &str = "/v1/invoiceitems";
const FINALIZE: &str = "/v1/invoices/{invoice}/finalize";

async fn add_item(
    client: &Client,
    customer: &ProviderCustomerId,
    invoice: &ProviderInvoiceId,
    line: &StatementLine,
    key: &str,
) -> Result<(), BillingError> {
    let amount = line.amount.minor().to_string();
    let description = format!(
        "{} for deployment {}, {} to {}",
        line.tier,
        line.deployment_id,
        line.starts_at.to_rfc3339(),
        line.ends_at.to_rfc3339(),
    );
    let starts_at = line.starts_at.timestamp().to_string();
    let ends_at = line.ends_at.timestamp().to_string();
    let deployment = line.deployment_id.to_string();
    let subscription = line.subscription_id.to_string();

    let form = Form::new()
        .field("customer", customer.as_str())
        .field("invoice", invoice.as_str())
        .field("currency", CURRENCY)
        .field("amount", &amount)
        .field("description", &description)
        .field("period[start]", &starts_at)
        .field("period[end]", &ends_at)
        .field("metadata[deployment_id]", &deployment)
        .field("metadata[subscription_id]", &subscription);

    client
        .post::<Created>(INVOICE_ITEMS, &[], &form, key)
        .await
        .map(drop)
}

async fn find(client: &Client, statement_id: &str) -> Result<Option<Found>, BillingError> {
    let query = format!("metadata['statement_id']:'{statement_id}'");
    let found: Search = client
        .get(INVOICE_SEARCH, &[], &[("query", query.as_str())])
        .await?;

    Ok(found.data.into_iter().next())
}

async fn create(
    client: &Client,
    customer: &ProviderCustomerId,
    statement: &Statement,
    statement_id: &str,
) -> Result<ProviderInvoiceId, BillingError> {
    let period = statement.period().to_string();
    let description = format!("Autharie {period}");

    let form = Form::new()
        .field("customer", customer.as_str())
        .field("collection_method", "charge_automatically")
        .field("currency", CURRENCY)
        .field("auto_advance", "true")
        .field("pending_invoice_items_behavior", "exclude")
        .field("metadata[statement_id]", statement_id)
        .field("metadata[period]", &period)
        .field("description", &description);

    let created: Created = client
        .post(INVOICES, &[], &form, &format!("statement:{statement_id}"))
        .await?;

    ProviderInvoiceId::parse(&created.id).map_err(|_| malformed(&Method::POST, INVOICES))
}

async fn complete(
    client: &Client,
    customer: &ProviderCustomerId,
    statement: &Statement,
    statement_id: &str,
    invoice: ProviderInvoiceId,
) -> Result<ProviderInvoice, BillingError> {
    for (index, line) in statement.lines().iter().enumerate() {
        let key = format!("statement:{statement_id}:line:{index}");
        add_item(client, customer, &invoice, line, &key).await?;
    }

    let form = Form::new().field("auto_advance", "true");
    let finalized: Finalized = client
        .post(
            FINALIZE,
            &[invoice.as_str()],
            &form,
            &format!("statement:{statement_id}:finalize"),
        )
        .await?;
    let hosted_url = https_url(&Method::POST, FINALIZE, finalized.hosted_invoice_url)?;

    Ok(ProviderInvoice {
        id: invoice,
        hosted_url,
    })
}

pub(crate) async fn issue(
    client: &Client,
    customer: &ProviderCustomerId,
    statement: &Statement,
) -> Result<ProviderInvoice, BillingError> {
    let statement_id = statement.id().to_string();

    let Some(found) = find(client, &statement_id).await? else {
        let invoice = create(client, customer, statement, &statement_id).await?;

        return complete(client, customer, statement, &statement_id, invoice).await;
    };

    let invoice =
        ProviderInvoiceId::parse(&found.id).map_err(|_| malformed(&Method::GET, INVOICE_SEARCH))?;

    match found.status {
        Some(InvoiceStatus::Draft) => {
            complete(client, customer, statement, &statement_id, invoice).await
        }
        Some(InvoiceStatus::Open | InvoiceStatus::Paid | InvoiceStatus::Uncollectible) => {
            let hosted_url = https_url(&Method::GET, INVOICE_SEARCH, found.hosted_invoice_url)?;

            Ok(ProviderInvoice {
                id: invoice,
                hosted_url,
            })
        }
        Some(InvoiceStatus::Other) | None => Err(failure(
            &Method::GET,
            INVOICE_SEARCH,
            "the invoice of the statement is in a state that cannot be sent",
        )),
    }
}
