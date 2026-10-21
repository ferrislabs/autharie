use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Created {
    pub(crate) id: String,
}

#[derive(Deserialize)]
pub(crate) struct Session {
    pub(crate) url: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct PaymentMethod {
    pub(crate) card: Option<Card>,
}

#[derive(Deserialize)]
pub(crate) struct Card {
    pub(crate) brand: String,
    pub(crate) last4: String,
    pub(crate) exp_month: u32,
    pub(crate) exp_year: u32,
}

#[derive(Deserialize)]
pub(crate) struct Finalized {
    pub(crate) hosted_invoice_url: Option<String>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InvoiceStatus {
    Draft,
    Open,
    Paid,
    Uncollectible,
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
pub(crate) struct Found {
    pub(crate) id: String,
    pub(crate) status: Option<InvoiceStatus>,
    pub(crate) hosted_invoice_url: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct Search {
    pub(crate) data: Vec<Found>,
}
