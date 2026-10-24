use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::RequestId;
use crate::domain::agents::sealing::{SealingError, SealingKey};

pub const ENVELOPE_VERSION: u8 = 1;
pub const ENVELOPE_ALGORITHM: &str = "RSA-OAEP-256+A256GCM";
const AES_KEY_BYTES: usize = 32;
const IV_BYTES: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u8,
    pub alg: String,
    pub ek: String,
    pub iv: String,
    pub ct: String,
}

pub fn seal(
    key: &SealingKey,
    request: &RequestId,
    plaintext: &[u8],
) -> Result<Envelope, SealingError> {
    let mut aes_key = Zeroizing::new([0u8; AES_KEY_BYTES]);
    let mut iv = [0u8; IV_BYTES];
    OsRng.fill_bytes(aes_key.as_mut_slice());
    OsRng.fill_bytes(&mut iv);

    let aad = request.to_string();
    let ciphertext = Aes256Gcm::new_from_slice(aes_key.as_slice())
        .map_err(|_| SealingError::Failed)?
        .encrypt(
            Nonce::from_slice(&iv),
            Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| SealingError::Failed)?;
    let wrapped_key = key.seal_bytes(aes_key.as_slice())?;

    Ok(Envelope {
        v: ENVELOPE_VERSION,
        alg: ENVELOPE_ALGORITHM.to_string(),
        ek: URL_SAFE_NO_PAD.encode(wrapped_key),
        iv: URL_SAFE_NO_PAD.encode(iv),
        ct: URL_SAFE_NO_PAD.encode(ciphertext),
    })
}

#[cfg(test)]
pub(crate) mod fixtures {
    use rsa::traits::{PrivateKeyParts, PublicKeyParts};
    use rsa::{Oaep, RsaPrivateKey};
    use serde_json::{Value, json};
    use sha2::Sha256;

    use super::*;

    pub fn open(
        private: &RsaPrivateKey,
        request: &RequestId,
        envelope: &Envelope,
    ) -> Result<Vec<u8>, String> {
        let decode = |text: &str| URL_SAFE_NO_PAD.decode(text).map_err(|e| e.to_string());
        let aes_key = Zeroizing::new(
            private
                .decrypt(Oaep::new::<Sha256>(), &decode(&envelope.ek)?)
                .map_err(|e| e.to_string())?,
        );
        let aad = request.to_string();

        Aes256Gcm::new_from_slice(&aes_key)
            .map_err(|e| e.to_string())?
            .decrypt(
                Nonce::from_slice(&decode(&envelope.iv)?),
                Payload {
                    msg: &decode(&envelope.ct)?,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| "the envelope does not open for this request".to_string())
    }

    pub fn private_jwk(private: &RsaPrivateKey) -> Value {
        let encode = |bytes: Vec<u8>| URL_SAFE_NO_PAD.encode(bytes);
        let primes = private.primes();
        json!({
            "kty": "RSA",
            "alg": "RSA-OAEP-256",
            "ext": true,
            "key_ops": ["decrypt"],
            "n": encode(private.n().to_bytes_be()),
            "e": encode(private.e().to_bytes_be()),
            "d": encode(private.d().to_bytes_be()),
            "p": encode(primes[0].to_bytes_be()),
            "q": encode(primes[1].to_bytes_be()),
            "dp": encode(private.dp().expect("precomputed").to_bytes_be()),
            "dq": encode(private.dq().expect("precomputed").to_bytes_be()),
            "qi": encode(private.qinv().expect("precomputed").to_bytes_be().1),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use rsa::traits::PublicKeyParts;
    use serde_json::{Value, json};

    use super::fixtures::*;
    use super::*;
    use crate::domain::agents::sealing::fixtures::{private_key, public_spec};
    use crate::domain::iam::result::{ApplicationPage, IamResult, fixtures::summary};

    const REQUEST: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";
    const OTHER_REQUEST: &str = "7f1c2d3e-0000-4000-8000-00000000000b";

    fn key() -> SealingKey {
        SealingKey::try_from(&public_spec()).expect("accepted")
    }

    fn request(raw: &str) -> RequestId {
        RequestId::parse(raw).expect("a UUID")
    }

    fn is_unpadded_base64url(text: &str) -> bool {
        !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    #[test]
    fn spec_iam_2_an_envelope_opens_with_the_browsers_private_key() {
        let plaintext = br#"{"items":[],"secret":"nothing here"}"#;

        let envelope = seal(&key(), &request(REQUEST), plaintext).expect("sealed");

        assert_eq!(
            open(&private_key(), &request(REQUEST), &envelope).expect("opens"),
            plaintext
        );
    }

    #[test]
    fn spec_iam_2_the_sealed_value_shows_nothing_of_the_content() {
        let envelope = seal(&key(), &request(REQUEST), b"jane.doe@example.com").expect("sealed");

        let shown = serde_json::to_string(&envelope).unwrap();

        assert!(!shown.contains("jane.doe"));
        assert!(!shown.contains("amFuZS5kb2U"));
    }

    #[test]
    fn spec_iam_4_an_envelope_for_one_request_does_not_open_as_another() {
        let envelope = seal(&key(), &request(REQUEST), b"{}").expect("sealed");

        assert!(open(&private_key(), &request(OTHER_REQUEST), &envelope).is_err());
        assert!(open(&private_key(), &request(REQUEST), &envelope).is_ok());
    }

    #[test]
    fn the_request_id_binds_in_its_hyphenated_lowercase_text_whatever_the_input_spelling() {
        let envelope =
            seal(&key(), &request("0B9AD3C44A5F4C3B8D570B2D6D9E1A11"), b"{}").expect("sealed");

        assert!(open(&private_key(), &request(REQUEST), &envelope).is_ok());
    }

    #[test]
    fn a_tampered_ciphertext_does_not_open() {
        let mut envelope = seal(&key(), &request(REQUEST), b"{}").expect("sealed");
        let mut raw = URL_SAFE_NO_PAD.decode(&envelope.ct).unwrap();
        raw[0] ^= 1;
        envelope.ct = URL_SAFE_NO_PAD.encode(raw);

        assert!(open(&private_key(), &request(REQUEST), &envelope).is_err());
    }

    #[test]
    fn every_envelope_has_a_fresh_key_and_iv() {
        let first = seal(&key(), &request(REQUEST), b"same").expect("sealed");
        let second = seal(&key(), &request(REQUEST), b"same").expect("sealed");

        assert_ne!(first.iv, second.iv);
        assert_ne!(first.ek, second.ek);
        assert_ne!(first.ct, second.ct);
    }

    #[test]
    fn the_envelope_is_in_the_frozen_shape() {
        let plaintext = b"{\"a\":1}";

        let envelope = seal(&key(), &request(REQUEST), plaintext).expect("sealed");
        let value = serde_json::to_value(&envelope).unwrap();

        assert_eq!(value["v"], json!(1));
        assert_eq!(value["alg"], json!("RSA-OAEP-256+A256GCM"));
        assert_eq!(value.as_object().unwrap().len(), 5);
        for field in ["ek", "iv", "ct"] {
            assert!(
                is_unpadded_base64url(value[field].as_str().unwrap()),
                "{field}"
            );
        }
        assert_eq!(URL_SAFE_NO_PAD.decode(&envelope.iv).unwrap().len(), 12);
        assert_eq!(
            URL_SAFE_NO_PAD.decode(&envelope.ek).unwrap().len(),
            private_key().size()
        );
        assert_eq!(
            URL_SAFE_NO_PAD.decode(&envelope.ct).unwrap().len(),
            plaintext.len() + 16
        );
    }

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/iam-envelope.json")
    }

    fn fixture_plaintext() -> Value {
        let page = IamResult::Applications(ApplicationPage {
            items: vec![summary("0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a22")],
            next_page: Some(2),
        });
        serde_json::from_slice(&page.to_json().unwrap()).unwrap()
    }

    #[test]
    fn the_interop_fixture_opens_with_its_private_key() {
        let fixture: Value =
            serde_json::from_str(&fs::read_to_string(fixture_path()).expect("the fixture"))
                .expect("JSON");
        let envelope: Envelope = serde_json::from_value(fixture["envelope"].clone()).unwrap();
        let id = request(fixture["request_id"].as_str().unwrap());

        let opened = open(&private_key(), &id, &envelope).expect("opens");

        assert_eq!(
            serde_json::from_slice::<Value>(&opened).unwrap(),
            fixture["plaintext"]
        );
        assert_eq!(fixture["private_jwk"], private_jwk(&private_key()));
        assert!(open(&private_key(), &request(OTHER_REQUEST), &envelope).is_err());
    }

    #[test]
    fn regenerate_the_interop_fixture_when_asked() {
        if std::env::var_os("REGENERATE_IAM_FIXTURE").is_none() {
            return;
        }
        let plaintext = fixture_plaintext();
        let envelope = seal(
            &key(),
            &request(REQUEST),
            &serde_json::to_vec(&plaintext).unwrap(),
        )
        .expect("sealed");
        let fixture = json!({
            "private_jwk": private_jwk(&private_key()),
            "request_id": REQUEST,
            "plaintext": plaintext,
            "envelope": envelope,
        });

        fs::write(
            fixture_path(),
            format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap()),
        )
        .expect("written");
    }
}
