use std::fmt::Display;

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use autharie_domain::{
    backups::{
        keys::{Dek, KeyName},
        ports::{KeyProvider, KeyProviderAdmin},
    },
    dataplane::{
        cloud_provider::Provider,
        credential::{CloudCredentialId, CloudCredentialStore, CredentialError, SecretString},
        credential_repository::{SealedCredential, SealedSecret, SealedSecretRepository},
    },
    generate_uuid_v7,
    organisation::OrganisationId,
};
use zeroize::Zeroizing;

pub const CREDENTIALS_KEY: &str = "cloud-credentials";

const NONCE_LEN: usize = 12;

pub struct EnvelopeCredentialStore<'a, R, K> {
    repository: R,
    keys: &'a K,
    key_name: KeyName,
}

impl<'a, R, K> EnvelopeCredentialStore<'a, R, K> {
    pub fn new(repository: R, keys: &'a K) -> Result<Self, CredentialError> {
        Ok(Self {
            repository,
            keys,
            key_name: KeyName::new(CREDENTIALS_KEY).map_err(store_failure)?,
        })
    }
}

fn store_failure(error: impl Display) -> CredentialError {
    CredentialError::Store(error.to_string())
}

fn cipher(dek: &Dek) -> Result<Aes256Gcm, CredentialError> {
    Aes256Gcm::new_from_slice(dek.expose())
        .map_err(|_| store_failure("the data key is not a 256 bit key"))
}

fn seal(
    dek: &Dek,
    id: &CloudCredentialId,
    secret: &SecretString,
) -> Result<(Vec<u8>, Vec<u8>), CredentialError> {
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher(dek)?
        .encrypt(
            &nonce,
            Payload {
                msg: secret.expose().as_bytes(),
                aad: id.0.as_bytes(),
            },
        )
        .map_err(|_| store_failure("the credential could not be encrypted"))?;

    Ok((ciphertext, nonce.to_vec()))
}

fn open(
    dek: &Dek,
    id: &CloudCredentialId,
    sealed: &SealedSecret,
) -> Result<SecretString, CredentialError> {
    if sealed.nonce.len() != NONCE_LEN {
        return Err(store_failure("the stored nonce has the wrong length"));
    }

    let plaintext = Zeroizing::new(
        cipher(dek)?
            .decrypt(
                Nonce::from_slice(&sealed.nonce),
                Payload {
                    msg: &sealed.ciphertext,
                    aad: id.0.as_bytes(),
                },
            )
            .map_err(|_| store_failure("the stored credential cannot be decrypted"))?,
    );

    let text = std::str::from_utf8(&plaintext)
        .map_err(|_| store_failure("the stored credential is not text"))?;

    Ok(SecretString::new(text))
}

impl<R, K> CloudCredentialStore for EnvelopeCredentialStore<'_, R, K>
where
    R: SealedSecretRepository,
    K: KeyProvider + KeyProviderAdmin,
{
    async fn put(
        &self,
        organisation_id: OrganisationId,
        provider: Provider,
        secret: SecretString,
    ) -> Result<CloudCredentialId, CredentialError> {
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

        let id = CloudCredentialId(generate_uuid_v7());
        let (ciphertext, nonce) = seal(&dek, &id, &secret)?;

        self.repository
            .insert(&SealedCredential {
                id,
                organisation_id,
                provider,
                sealed: SealedSecret {
                    ciphertext,
                    nonce,
                    wrapped_dek,
                    key,
                },
            })
            .await
            .map_err(store_failure)?;

        Ok(id)
    }

    async fn get_for_provisioning(
        &self,
        id: &CloudCredentialId,
    ) -> Result<SecretString, CredentialError> {
        let stored = self
            .repository
            .get(id)
            .await
            .map_err(store_failure)?
            .ok_or(CredentialError::NotFound { id: *id })?;

        let dek = self
            .keys
            .unwrap_data_key(&stored.sealed.key, &stored.sealed.wrapped_dek)
            .await
            .map_err(store_failure)?;

        open(&dek, id, &stored.sealed)
    }

    async fn delete(&self, id: &CloudCredentialId) -> Result<(), CredentialError> {
        let removed = self.repository.delete(id).await.map_err(store_failure)?;

        if removed {
            Ok(())
        } else {
            Err(CredentialError::NotFound { id: *id })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use autharie_domain::{
        CoreError,
        backups::keys::{DataKey, KeyError, KeyRef, KeyVersion, ProviderName, WrappedDek},
    };
    use uuid::Uuid;

    use super::*;

    const PLAINTEXT: &str = "SCWXXXXXXXXXXXXXXXXX-super-secret-key";

    #[derive(Default)]
    struct InMemoryRepository {
        rows: Mutex<HashMap<CloudCredentialId, SealedCredential>>,
    }

    impl SealedSecretRepository for InMemoryRepository {
        async fn insert(&self, credential: &SealedCredential) -> Result<(), CoreError> {
            self.rows
                .lock()
                .expect("lock")
                .insert(credential.id, credential.clone());
            Ok(())
        }

        async fn get(&self, id: &CloudCredentialId) -> Result<Option<SealedCredential>, CoreError> {
            Ok(self.rows.lock().expect("lock").get(id).cloned())
        }

        async fn delete(&self, id: &CloudCredentialId) -> Result<bool, CoreError> {
            Ok(self.rows.lock().expect("lock").remove(id).is_some())
        }
    }

    #[derive(Default)]
    struct FakeKeys {
        wrapped: Mutex<HashMap<String, Vec<u8>>>,
        ensured: AtomicUsize,
        destroyed: bool,
    }

    fn key_ref() -> KeyRef {
        KeyRef::new(
            ProviderName::platform(),
            KeyName::new(CREDENTIALS_KEY).expect("a key name"),
            KeyVersion::new(1),
        )
    }

    impl KeyProvider for FakeKeys {
        async fn generate_data_key(&self, name: &KeyName) -> Result<DataKey, KeyError> {
            assert_eq!(name.as_str(), CREDENTIALS_KEY);
            let key = Aes256Gcm::generate_key(OsRng).to_vec();
            let wrapped = format!("wrapped:{}", Uuid::new_v4());
            self.wrapped
                .lock()
                .expect("lock")
                .insert(wrapped.clone(), key.clone());

            Ok(DataKey::new(
                Dek::new(key),
                WrappedDek::new(wrapped),
                key_ref(),
            ))
        }

        async fn unwrap_data_key(
            &self,
            key: &KeyRef,
            wrapped: &WrappedDek,
        ) -> Result<Dek, KeyError> {
            if self.destroyed {
                return Err(KeyError::KeyUnavailable {
                    key: key.to_string(),
                    reason: "destroyed".to_string(),
                });
            }

            self.wrapped
                .lock()
                .expect("lock")
                .get(wrapped.as_str())
                .map(|bytes| Dek::new(bytes.clone()))
                .ok_or(KeyError::Refused {
                    operation: "unwrap".to_string(),
                    reason: "unknown".to_string(),
                })
        }
    }

    impl KeyProviderAdmin for FakeKeys {
        async fn ensure_key(&self, name: &KeyName) -> Result<(), KeyError> {
            assert_eq!(name.as_str(), CREDENTIALS_KEY);
            self.ensured.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn organisation() -> OrganisationId {
        OrganisationId(Uuid::new_v4())
    }

    async fn stored(
        store: &EnvelopeCredentialStore<'_, InMemoryRepository, FakeKeys>,
    ) -> CloudCredentialId {
        store
            .put(
                organisation(),
                Provider::Scaleway,
                SecretString::new(PLAINTEXT),
            )
            .await
            .expect("stored")
    }

    fn store<'a>(
        repository: InMemoryRepository,
        keys: &'a FakeKeys,
    ) -> EnvelopeCredentialStore<'a, InMemoryRepository, FakeKeys> {
        EnvelopeCredentialStore::new(repository, keys).expect("a store")
    }

    #[tokio::test]
    async fn a_stored_secret_comes_back_for_provisioning() {
        let keys = FakeKeys::default();
        let store = store(InMemoryRepository::default(), &keys);

        let id = stored(&store).await;
        let secret = store.get_for_provisioning(&id).await.expect("read back");

        assert_eq!(secret.expose(), PLAINTEXT);
    }

    #[tokio::test]
    async fn the_stored_row_never_holds_the_secret_in_clear_spec_ccp_3() {
        let keys = FakeKeys::default();
        let store = store(InMemoryRepository::default(), &keys);

        let id = stored(&store).await;
        let row = store
            .repository
            .rows
            .lock()
            .expect("lock")
            .get(&id)
            .cloned()
            .expect("a row");

        let needle = PLAINTEXT.as_bytes();
        assert!(
            !row.sealed
                .ciphertext
                .windows(needle.len())
                .any(|w| w == needle)
        );
        assert!(!row.sealed.wrapped_dek.as_str().contains(PLAINTEXT));
        assert_eq!(row.sealed.nonce.len(), NONCE_LEN);
        assert_eq!(row.sealed.key, key_ref());
    }

    #[tokio::test]
    async fn each_credential_gets_its_own_nonce_and_data_key() {
        let keys = FakeKeys::default();
        let store = store(InMemoryRepository::default(), &keys);

        let first = stored(&store).await;
        let second = stored(&store).await;
        let rows = store.repository.rows.lock().expect("lock");

        assert_ne!(rows[&first].sealed.nonce, rows[&second].sealed.nonce);
        assert_ne!(
            rows[&first].sealed.wrapped_dek,
            rows[&second].sealed.wrapped_dek
        );
        assert_ne!(
            rows[&first].sealed.ciphertext,
            rows[&second].sealed.ciphertext
        );
    }

    #[tokio::test]
    async fn the_key_is_ensured_before_it_is_first_used() {
        let keys = FakeKeys::default();
        let store = store(InMemoryRepository::default(), &keys);

        stored(&store).await;

        assert_eq!(keys.ensured.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_row_moved_under_another_id_cannot_be_opened() {
        let keys = FakeKeys::default();
        let store = store(InMemoryRepository::default(), &keys);

        let id = stored(&store).await;
        let other = CloudCredentialId(Uuid::new_v4());
        {
            let mut rows = store.repository.rows.lock().expect("lock");
            let mut row = rows[&id].clone();
            row.id = other;
            rows.insert(other, row);
        }

        let result = store.get_for_provisioning(&other).await;

        assert!(matches!(result, Err(CredentialError::Store(_))));
    }

    #[tokio::test]
    async fn a_tampered_ciphertext_cannot_be_opened() {
        let keys = FakeKeys::default();
        let store = store(InMemoryRepository::default(), &keys);

        let id = stored(&store).await;
        store
            .repository
            .rows
            .lock()
            .expect("lock")
            .get_mut(&id)
            .expect("a row")
            .sealed
            .ciphertext[0] ^= 1;

        assert!(matches!(
            store.get_for_provisioning(&id).await,
            Err(CredentialError::Store(_))
        ));
    }

    #[tokio::test]
    async fn a_destroyed_key_makes_the_credential_unreadable_without_leaking_it() {
        let keys = FakeKeys::default();
        let writer = store(InMemoryRepository::default(), &keys);
        let id = stored(&writer).await;
        let repository = InMemoryRepository {
            rows: Mutex::new(writer.repository.rows.lock().expect("lock").clone()),
        };
        let gone = FakeKeys {
            destroyed: true,
            ..FakeKeys::default()
        };
        let reader = EnvelopeCredentialStore::new(repository, &gone).expect("a store");

        let error = reader
            .get_for_provisioning(&id)
            .await
            .err()
            .expect("refused");

        assert!(!error.to_string().contains(PLAINTEXT));
    }

    #[tokio::test]
    async fn an_unknown_credential_is_not_found() {
        let keys = FakeKeys::default();
        let store = store(InMemoryRepository::default(), &keys);
        let id = CloudCredentialId(Uuid::new_v4());

        assert!(matches!(
            store.get_for_provisioning(&id).await,
            Err(CredentialError::NotFound { .. })
        ));
        assert!(matches!(
            store.delete(&id).await,
            Err(CredentialError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn a_deleted_credential_is_gone() {
        let keys = FakeKeys::default();
        let store = store(InMemoryRepository::default(), &keys);
        let id = stored(&store).await;

        store.delete(&id).await.expect("deleted");

        assert!(matches!(
            store.get_for_provisioning(&id).await,
            Err(CredentialError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn a_data_key_of_the_wrong_size_is_refused() {
        let dek = Dek::new(vec![0; 16]);

        assert!(matches!(cipher(&dek), Err(CredentialError::Store(_))));
    }
}
