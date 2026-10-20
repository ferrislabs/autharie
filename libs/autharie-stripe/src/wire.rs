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
pub(crate) struct Finalized {
    pub(crate) hosted_invoice_url: Option<String>,
}
