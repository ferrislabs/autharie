use autharie_domain::{
    CoreError,
    backups::{
        keys::KeyName,
        ports::{KeyProvider, KeyProviderAdmin},
    },
    cells::{
        ADMINISTRATOR_CREDENTIAL_LIFETIME, AdministratorVault, InitialAdministrator,
        SealedAdministrator, TenantAdministratorRepository,
    },
    dataplane::{credential::CredentialError, credential_repository::SealedSecret},
    deployments::DeploymentId,
};
use chrono::{DateTime, Utc};

use super::envelope::{open, seal, store_failure};

pub const ADMINISTRATORS_KEY: &str = "tenant-administrators";

pub struct EnvelopeAdministratorVault<'a, R, K> {
    repository: R,
    keys: &'a K,
    key_name: KeyName,
}

impl<'a, R, K> EnvelopeAdministratorVault<'a, R, K> {
    pub fn new(repository: R, keys: &'a K) -> Result<Self, CredentialError> {
        Ok(Self {
            repository,
            keys,
            key_name: KeyName::new(ADMINISTRATORS_KEY).map_err(store_failure)?,
        })
    }
}

impl<R, K> AdministratorVault for EnvelopeAdministratorVault<'_, R, K>
where
    R: TenantAdministratorRepository,
    K: KeyProvider + KeyProviderAdmin,
{
    async fn keep(
        &self,
        deployment: DeploymentId,
        administrator: InitialAdministrator,
        now: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        self.keys
            .ensure_key(&self.key_name)
            .await
            .map_err(store_failure)?;
        let (dek, wrapped_dek, key) = self
            .keys
            .generate_data_key(&self.key_name)
            .await
            .map_err(store_failure)?
            .into_parts();

        let (ciphertext, nonce) = seal(&dek, deployment.0.as_bytes(), administrator.password())?;

        self.repository
            .store_issued(
                deployment,
                &SealedAdministrator {
                    username: administrator.username().to_string(),
                    sealed: SealedSecret {
                        ciphertext,
                        nonce,
                        wrapped_dek,
                        key,
                    },
                    issued_at: now,
                    expires_at: now + ADMINISTRATOR_CREDENTIAL_LIFETIME,
                },
            )
            .await
    }

    async fn reveal(
        &self,
        deployment: DeploymentId,
        now: DateTime<Utc>,
    ) -> Result<InitialAdministrator, CoreError> {
        let taken = self.repository.take(deployment, now).await?.into_sealed()?;

        let dek = self
            .keys
            .unwrap_data_key(&taken.sealed.key, &taken.sealed.wrapped_dek)
            .await
            .map_err(store_failure)?;
        let password = open(&dek, deployment.0.as_bytes(), &taken.sealed)?;

        Ok(InitialAdministrator::new(taken.username, password))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use autharie_domain::{
        backups::keys::{DataKey, Dek, KeyError, KeyRef, KeyVersion, ProviderName, WrappedDek},
        cells::{AdministratorCredentialError, IssueRecord, Taken, TenantAdministrator},
        dataplane::credential::SecretString,
    };
    use chrono::TimeZone;
    use uuid::Uuid;

    use super::*;

    const PASSWORD: &str = "Tmp-Admin-Password-42!x";
    const DEPLOYMENT: DeploymentId = DeploymentId(Uuid::from_u128(7));

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 13, 12, 0, 0).unwrap()
    }

    #[derive(Default)]
    struct Rows {
        requested: Mutex<bool>,
        row: Mutex<Option<(SealedAdministrator, bool)>>,
    }

    impl Rows {
        fn requested() -> Self {
            Self {
                requested: Mutex::new(true),
                ..Self::default()
            }
        }
    }

    impl TenantAdministratorRepository for &Rows {
        async fn issue_record(&self, _: DeploymentId) -> Result<Option<IssueRecord>, CoreError> {
            Ok(None)
        }

        async fn request_issue(&self, _: DeploymentId, _: DateTime<Utc>) -> Result<(), CoreError> {
            *self.requested.lock().unwrap() = true;
            *self.row.lock().unwrap() = None;
            Ok(())
        }

        async fn store_issued(
            &self,
            _: DeploymentId,
            administrator: &SealedAdministrator,
        ) -> Result<bool, CoreError> {
            let mut requested = self.requested.lock().unwrap();
            if !*requested {
                return Ok(false);
            }
            *requested = false;
            *self.row.lock().unwrap() = Some((administrator.clone(), false));
            Ok(true)
        }

        async fn take(&self, _: DeploymentId, now: DateTime<Utc>) -> Result<Taken, CoreError> {
            let mut row = self.row.lock().unwrap();
            Ok(match row.as_mut() {
                None => Taken::Absent,
                Some((_, true)) => Taken::Revealed,
                Some((sealed, _)) if sealed.expires_at <= now => Taken::Expired,
                Some((sealed, revealed)) => {
                    *revealed = true;
                    Taken::Sealed(sealed.clone())
                }
            })
        }

        async fn describe(
            &self,
            _: DeploymentId,
            _: DateTime<Utc>,
        ) -> Result<Option<TenantAdministrator>, CoreError> {
            Ok(None)
        }

        async fn purge_expired(&self, _: DateTime<Utc>) -> Result<u64, CoreError> {
            Ok(0)
        }
    }

    struct FixedKeys;

    fn key_ref() -> KeyRef {
        KeyRef::new(
            ProviderName::new("test"),
            KeyName::new(ADMINISTRATORS_KEY).unwrap(),
            KeyVersion::new(1),
        )
    }

    impl KeyProvider for FixedKeys {
        async fn generate_data_key(&self, _: &KeyName) -> Result<DataKey, KeyError> {
            Ok(DataKey::new(
                Dek::new(vec![9u8; 32]),
                WrappedDek::new("wrapped"),
                key_ref(),
            ))
        }

        async fn unwrap_data_key(&self, _: &KeyRef, _: &WrappedDek) -> Result<Dek, KeyError> {
            Ok(Dek::new(vec![9u8; 32]))
        }
    }

    impl KeyProviderAdmin for FixedKeys {
        async fn ensure_key(&self, _: &KeyName) -> Result<(), KeyError> {
            Ok(())
        }
    }

    fn admin() -> InitialAdministrator {
        InitialAdministrator::new("admin", SecretString::new(PASSWORD))
    }

    #[tokio::test]
    async fn a_kept_administrator_is_stored_sealed_and_revealed_once() {
        let rows = Rows::requested();
        let vault = EnvelopeAdministratorVault::new(&rows, &FixedKeys).unwrap();

        assert!(vault.keep(DEPLOYMENT, admin(), now()).await.unwrap());
        let (stored, _) = rows.row.lock().unwrap().clone().unwrap();
        assert!(
            !stored
                .sealed
                .ciphertext
                .windows(PASSWORD.len())
                .any(|w| w == PASSWORD.as_bytes())
        );
        assert_eq!(stored.expires_at, now() + ADMINISTRATOR_CREDENTIAL_LIFETIME);

        let revealed = vault.reveal(DEPLOYMENT, now()).await.unwrap();
        assert_eq!(revealed.username(), "admin");
        assert_eq!(revealed.password().expose(), PASSWORD);

        let again = vault.reveal(DEPLOYMENT, now()).await;
        assert!(matches!(
            again,
            Err(CoreError::TenantAdministrator(
                AdministratorCredentialError::AlreadyRevealed
            ))
        ));
    }

    #[tokio::test]
    async fn a_credential_nobody_asked_for_is_not_kept() {
        let rows = Rows::default();
        let vault = EnvelopeAdministratorVault::new(&rows, &FixedKeys).unwrap();

        assert!(!vault.keep(DEPLOYMENT, admin(), now()).await.unwrap());
        assert!(rows.row.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn a_new_issue_replaces_the_credential_before_it() {
        let rows = Rows::requested();
        let vault = EnvelopeAdministratorVault::new(&rows, &FixedKeys).unwrap();
        vault.keep(DEPLOYMENT, admin(), now()).await.unwrap();

        (&rows).request_issue(DEPLOYMENT, now()).await.unwrap();
        let newer = InitialAdministrator::new("admin", SecretString::new("Newer-Pass-7!"));
        assert!(vault.keep(DEPLOYMENT, newer, now()).await.unwrap());

        let revealed = vault.reveal(DEPLOYMENT, now()).await.unwrap();
        assert_eq!(revealed.password().expose(), "Newer-Pass-7!");
    }

    #[tokio::test]
    async fn a_credential_cannot_be_opened_under_another_deployment() {
        let rows = Rows::requested();
        let vault = EnvelopeAdministratorVault::new(&rows, &FixedKeys).unwrap();
        vault.keep(DEPLOYMENT, admin(), now()).await.unwrap();

        let result = vault.reveal(DeploymentId(Uuid::from_u128(8)), now()).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn an_expired_or_missing_credential_is_never_opened() {
        let rows = Rows::requested();
        let vault = EnvelopeAdministratorVault::new(&rows, &FixedKeys).unwrap();
        assert!(matches!(
            vault.reveal(DEPLOYMENT, now()).await,
            Err(CoreError::TenantAdministrator(
                AdministratorCredentialError::NotIssued
            ))
        ));

        vault.keep(DEPLOYMENT, admin(), now()).await.unwrap();
        let later = now() + ADMINISTRATOR_CREDENTIAL_LIFETIME;
        assert!(matches!(
            vault.reveal(DEPLOYMENT, later).await,
            Err(CoreError::TenantAdministrator(
                AdministratorCredentialError::Expired
            ))
        ));
    }
}
