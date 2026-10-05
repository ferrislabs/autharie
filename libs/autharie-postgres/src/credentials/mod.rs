mod cloud_credential_repository;
mod sealed_secret_repository;

pub use cloud_credential_repository::PostgresCloudCredentialRepository;
pub use sealed_secret_repository::PostgresSealedSecretRepository;

use autharie_domain::{CoreError, dataplane::cloud_provider::Provider};

pub(crate) fn parse_provider(raw: &str) -> Result<Provider, CoreError> {
    match raw {
        "scaleway" => Ok(Provider::Scaleway),
        other => Err(CoreError::InternalError(format!(
            "unknown cloud provider '{other}'"
        ))),
    }
}
