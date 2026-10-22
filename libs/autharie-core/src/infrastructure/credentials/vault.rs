use autharie_domain::{
    CoreError,
    backups::{
        keys::{KeyError, KeyName},
        ports::{KeyProvider, KeyProviderAdmin},
    },
    dataplane::{credential::SecretString, credential_repository::SealedSecret},
    webhooks::ports::SecretVault,
};

use super::envelope::{CREDENTIALS_KEY, open, seal};

pub struct EnvelopeVault<'a, K> {
    keys: Option<&'a K>,
    key_name: KeyName,
}

impl<'a, K> EnvelopeVault<'a, K> {
    pub fn new(keys: Option<&'a K>) -> Result<Self, CoreError> {
        Ok(Self {
            keys,
            key_name: KeyName::new(CREDENTIALS_KEY)?,
        })
    }

    fn keys(&self) -> Result<&'a K, CoreError> {
        self.keys.ok_or_else(|| {
            KeyError::ProviderUnavailable {
                reason: "no key manager is configured on this installation".to_string(),
            }
            .into()
        })
    }
}

impl<K> SecretVault for EnvelopeVault<'_, K>
where
    K: KeyProvider + KeyProviderAdmin,
{
    async fn seal(
        &self,
        context: &str,
        plaintext: &SecretString,
    ) -> Result<SealedSecret, CoreError> {
        let keys = self.keys()?;
        keys.ensure_key(&self.key_name).await?;

        let (dek, wrapped_dek, key) = keys.generate_data_key(&self.key_name).await?.into_parts();
        let (ciphertext, nonce) = seal(&dek, context.as_bytes(), plaintext)?;

        Ok(SealedSecret {
            ciphertext,
            nonce,
            wrapped_dek,
            key,
        })
    }

    async fn open(&self, context: &str, sealed: &SealedSecret) -> Result<SecretString, CoreError> {
        let dek = self
            .keys()?
            .unwrap_data_key(&sealed.key, &sealed.wrapped_dek)
            .await?;

        Ok(open(&dek, context.as_bytes(), sealed)?)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use aes_gcm::{Aes256Gcm, KeyInit, aead::OsRng};
    use autharie_domain::backups::keys::{
        DataKey, Dek, KeyRef, KeyVersion, ProviderName, WrappedDek,
    };
    use uuid::Uuid;

    use super::*;

    const PLAINTEXT: &str = "whsec_super-secret";

    #[derive(Default)]
    struct FakeKeys {
        wrapped: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl KeyProvider for FakeKeys {
        async fn generate_data_key(&self, name: &KeyName) -> Result<DataKey, KeyError> {
            let key = Aes256Gcm::generate_key(OsRng).to_vec();
            let wrapped = format!("wrapped:{}", Uuid::new_v4());
            self.wrapped
                .lock()
                .unwrap()
                .insert(wrapped.clone(), key.clone());

            Ok(DataKey::new(
                Dek::new(key),
                WrappedDek::new(wrapped),
                KeyRef::new(ProviderName::platform(), name.clone(), KeyVersion::new(1)),
            ))
        }

        async fn unwrap_data_key(&self, _: &KeyRef, wrapped: &WrappedDek) -> Result<Dek, KeyError> {
            self.wrapped
                .lock()
                .unwrap()
                .get(wrapped.as_str())
                .map(|bytes| Dek::new(bytes.clone()))
                .ok_or(KeyError::Refused {
                    operation: "unwrap".to_string(),
                    reason: "unknown".to_string(),
                })
        }
    }

    impl KeyProviderAdmin for FakeKeys {
        async fn ensure_key(&self, _: &KeyName) -> Result<(), KeyError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_sealed_secret_opens_under_its_own_context_only() {
        let keys = FakeKeys::default();
        let vault = EnvelopeVault::new(Some(&keys)).unwrap();

        let sealed = vault
            .seal("webhook-secret:a:1", &SecretString::new(PLAINTEXT))
            .await
            .unwrap();

        let needle = PLAINTEXT.as_bytes();
        assert!(!sealed.ciphertext.windows(needle.len()).any(|w| w == needle));
        assert_eq!(
            vault
                .open("webhook-secret:a:1", &sealed)
                .await
                .unwrap()
                .expose(),
            PLAINTEXT
        );
        assert!(vault.open("webhook-secret:b:1", &sealed).await.is_err());
    }

    #[tokio::test]
    async fn two_seals_of_one_secret_differ() {
        let keys = FakeKeys::default();
        let vault = EnvelopeVault::new(Some(&keys)).unwrap();
        let secret = SecretString::new(PLAINTEXT);

        let first = vault.seal("ctx", &secret).await.unwrap();
        let second = vault.seal("ctx", &secret).await.unwrap();

        assert_ne!(first.ciphertext, second.ciphertext);
        assert_ne!(first.nonce, second.nonce);
        assert_ne!(first.wrapped_dek, second.wrapped_dek);
    }

    #[tokio::test]
    async fn without_a_key_manager_nothing_is_sealed_or_opened() {
        let vault: EnvelopeVault<'_, FakeKeys> = EnvelopeVault::new(None).unwrap();

        let sealed = vault.seal("ctx", &SecretString::new(PLAINTEXT)).await;

        assert!(matches!(
            sealed,
            Err(CoreError::Key(KeyError::ProviderUnavailable { .. }))
        ));
    }
}
