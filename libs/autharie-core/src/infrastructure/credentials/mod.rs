mod envelope;
mod providers;

pub use envelope::{CREDENTIALS_KEY, EnvelopeCredentialStore};
pub use providers::{CloudProviders, FixedCloudProvider, FixedVerdict};
