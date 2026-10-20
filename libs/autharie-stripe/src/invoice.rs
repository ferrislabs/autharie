use autharie_domain::billing::{
    BillingError, ProviderCustomerId, ProviderInvoice, ProviderInvoiceId, Statement, StatementLine,
};

use crate::{
    CURRENCY,
    client::{Client, Key, malformed},
    form::Form,
    wire::{Created, Finalized},
};

const INVOICES: &str = "/v1/invoices";
const INVOICE_ITEMS: &str = "/v1/invoiceitems";
const FINALIZE: &str = "/v1/invoices/{invoice}/finalize";

async fn add_item(
    client: &Client,
    customer: &ProviderCustomerId,
    invoice: &ProviderInvoiceId,
    line: &StatementLine,
    key: String,
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
        .post::<Created>(INVOICE_ITEMS, &[], &form, Key::Fixed(key))
        .await
        .map(drop)
}

pub(crate) async fn issue(
    client: &Client,
    customer: &ProviderCustomerId,
    statement: &Statement,
) -> Result<ProviderInvoice, BillingError> {
    let statement_id = statement.id().to_string();
    let period = statement.period().to_string();
    let description = format!("Autharie {period}");

    let form = Form::new()
        .field("customer", customer.as_str())
        .field("collection_method", "charge_automatically")
        .field("currency", CURRENCY)
        .field("auto_advance", "true")
        .field("pending_invoice_items_behavior", "exclude")
        .field("metadata[statement_id]", &statement_id)
        .field("metadata[period]", &period)
        .field("description", &description);

    let created: Created = client
        .post(
            INVOICES,
            &[],
            &form,
            Key::Fixed(format!("statement:{statement_id}")),
        )
        .await?;
    let invoice = ProviderInvoiceId::parse(&created.id).map_err(|_| malformed(INVOICES))?;

    for (index, line) in statement.lines().iter().enumerate() {
        let key = format!("statement:{statement_id}:line:{index}");
        add_item(client, customer, &invoice, line, key).await?;
    }

    let form = Form::new().field("auto_advance", "true");
    let finalized: Finalized = client
        .post(
            FINALIZE,
            &[invoice.as_str()],
            &form,
            Key::Fixed(format!("statement:{statement_id}:finalize")),
        )
        .await?;
    let hosted_url = finalized
        .hosted_invoice_url
        .ok_or_else(|| malformed(FINALIZE))?;

    Ok(ProviderInvoice {
        id: invoice,
        hosted_url,
    })
}
