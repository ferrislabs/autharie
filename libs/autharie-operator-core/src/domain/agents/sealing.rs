use std::fmt;

use autharie_crds::v1alpha::identity_instance::SealingKeySpec;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::rngs::OsRng;
use rsa::{BigUint, Oaep, RsaPublicKey, traits::PublicKeyParts};
use sha2::Sha256;

pub const MIN_SEALING_BITS: usize = 2048;
pub const MAX_SEALING_BITS: usize = 8192;
const MIN_EXPONENT: u64 = 65_537;
const ALGORITHM: &str = "RSA-OAEP-256";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealingError {
    NotRsa,
    WrongAlgorithm,
    Malformed,
    TooSmall,
    TooLarge,
    WeakExponent,
    Failed,
}

impl fmt::Display for SealingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotRsa => "the sealing key is not an RSA key",
            Self::WrongAlgorithm => "the sealing key is not meant for RSA-OAEP-256",
            Self::Malformed => "the sealing key is not well formed",
            Self::TooSmall => "the sealing key is under 2048 bits",
            Self::TooLarge => "the sealing key is over 8192 bits",
            Self::WeakExponent => "the sealing key's exponent is too small or even",
            Self::Failed => "the secret could not be sealed",
        })
    }
}

impl std::error::Error for SealingError {}

#[derive(Clone)]
pub struct SealingKey(RsaPublicKey);

impl fmt::Debug for SealingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealingKey")
            .field("bits", &(self.0.size() * 8))
            .finish()
    }
}

fn bits(bytes: &[u8]) -> usize {
    match bytes.iter().position(|byte| *byte != 0) {
        None => 0,
        Some(first) => (bytes.len() - first) * 8 - bytes[first].leading_zeros() as usize,
    }
}

impl TryFrom<&SealingKeySpec> for SealingKey {
    type Error = SealingError;

    fn try_from(spec: &SealingKeySpec) -> Result<Self, Self::Error> {
        if spec.kty != "RSA" {
            return Err(SealingError::NotRsa);
        }
        if spec.alg != ALGORITHM {
            return Err(SealingError::WrongAlgorithm);
        }

        let modulus = URL_SAFE_NO_PAD
            .decode(&spec.n)
            .map_err(|_| SealingError::Malformed)?;
        let size = bits(&modulus);
        if size < MIN_SEALING_BITS {
            return Err(SealingError::TooSmall);
        }
        if size > MAX_SEALING_BITS {
            return Err(SealingError::TooLarge);
        }

        let exponent = URL_SAFE_NO_PAD
            .decode(&spec.e)
            .map_err(|_| SealingError::Malformed)?;
        if exponent.is_empty() || exponent.len() > 8 {
            return Err(SealingError::WeakExponent);
        }
        let value = exponent
            .iter()
            .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
        if value < MIN_EXPONENT || value % 2 == 0 {
            return Err(SealingError::WeakExponent);
        }

        RsaPublicKey::new(
            BigUint::from_bytes_be(&modulus),
            BigUint::from_bytes_be(&exponent),
        )
        .map(Self)
        .map_err(|_| SealingError::Malformed)
    }
}

impl SealingKey {
    pub fn seal(&self, secret: &str) -> Result<String, SealingError> {
        self.seal_bytes(secret.as_bytes())
            .map(|sealed| URL_SAFE_NO_PAD.encode(sealed))
    }

    pub fn seal_bytes(&self, data: &[u8]) -> Result<Vec<u8>, SealingError> {
        self.0
            .encrypt(&mut OsRng, Oaep::new::<Sha256>(), data)
            .map_err(|_| SealingError::Failed)
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub const PRIVATE_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDHF23PRKIxgKZC
8b8NihRuCh/PTx8bdX+x5Mp6WR44eVFNirC/j9mhmtK5vezS3CokPgpl0g1CVfBp
iZR1OEZs+Y0cFFJBZTWxuDiUz3jAQIqQlN7WH6dNsOu14FJS799Tv4yAC/wtx7ig
yLpncaQYJE5CkSOAGd+P7YBT2ONtOs0dR0+bdTbDkpu1MlIEsVMojzJFVuGGKtJp
cgLrYVZKACNop5y84tQUJx7vLW2JfdEZleFJ6k4g9DNnl/Y6njLTsKVtCKakORf3
wiqbk80IQxN7labaVQlXd1GooBC+7mBxwyXbFW35eM31GgQBPVaqBOHpBLu60knW
t7hC0x/xAgMBAAECggEAHjaCfg1K1dtRn+Ai37GgJxDXQfUeYeLjZYI0bfu/N8/F
VFCjQPbaDom5x+E4IsmxhX16w3fsdjAng0STKHTJTzlRvjyhPPZYfydXQtH3X6mL
vaQx6umz0Hj0VE3+AEMRr5pmfnoTI3lnHdNIYnFe9yDvVW/EJOkIQcXHjzHfVZBt
ofFGHL8NjJ008VEVwDtscaCq+ibfoEghvI9GMffd/HqZAYd9qhrz+wiT8ZQAFbp5
kTlP6YBUJ+mo2K7OkNdGPivgaxQhijwqc9d53eFMrmnETxliAHN1Alniud16o1j8
TpaIwF0Y+y6trmHrKXWaQkVRbPfYT2QTSmpTeLe1jwKBgQDzEevZNdjBJWvjvqaX
5n5F3ZPQD67XKghgokkNa+uKrIvHrzG4HDXrR7R24SHBxTmHGgw2k3WRfaFBnoHN
n7BoJNK+M8ddP3b0ea2kFpPAkWuWmOxv0VQykt721vfkHohBu5ra5eoXXd4Efnj5
PqX50JCVPT+k5Xl4R9dpbniziwKBgQDRrp4QZoiX3GXEmddqIn5ZwMrY/ia9Z8M0
da3I/+PCUFw23HEP0T6LskS8g64dG63hhrCy0BZN+WrJQu/m82cAJaRsQCbzilIt
K6/3NtXlu4SmXotGxEpn26X03j0YO1osKLFgd2FiT/0KiIQYj1/Ipyst3YghCIjR
zYm1KKx58wKBgBAV4oa4UoTNpisnJb0tqrOS60I8l3RzuqQyeSUjPC4sJv/q7x5g
94x/bUjksygwlhMDvUUrUv9y0eYWyD5EUBdEQJIHuSzJk2SwXLZcLCD1Pqpzqkno
D2tdXtX0+eilwJyg/ql3x5sOQjAH8peD9tXmYHsP15NhAD3eeznl7qTrAoGBAIXj
8pqWXnJaEcHQWnUzQWseaGjXIPWg5E0DN805WL4jgj6l1Kw8+KtLUgjuLKf5nLZ9
wybrKNLxiPaq/3WBxyuY3b0h2b15fa/KTbqWEU94xeNWS6kMflaDMx2BK5HllFbO
RTVMBas5WGL5eSAVrRv7Yt8OrnYpdPRDQsOjDT9xAoGBAMq7pYVEJBWoyFYWDnSY
LoQgUrpiRssRjaCMHOpEBxjtOTv3TzeyzHWD7+r2+y/qToJXcdA8jEyhaSeUa7mr
9e2VtIC/6Ouhmfb0+mwgwO/zQHR0sd/ruyNc7v4FBgYfZ/XqvYtzzTZzhNmvX9gQ
HUim3t4M1KMtX1QmMKKCg4i4
-----END PRIVATE KEY-----
"#;

    pub fn private_key() -> rsa::RsaPrivateKey {
        use rsa::pkcs8::DecodePrivateKey;
        rsa::RsaPrivateKey::from_pkcs8_pem(PRIVATE_KEY_PEM).expect("a test key")
    }

    pub fn public_spec() -> SealingKeySpec {
        let public = private_key().to_public_key();
        SealingKeySpec {
            kty: "RSA".to_string(),
            n: URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
            e: URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
            alg: ALGORITHM.to_string(),
        }
    }

    pub fn open(sealed: &str) -> String {
        let raw = URL_SAFE_NO_PAD.decode(sealed).expect("base64url");
        String::from_utf8(
            private_key()
                .decrypt(Oaep::new::<Sha256>(), &raw)
                .expect("decrypts"),
        )
        .expect("utf-8")
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn small(bytes: usize) -> SealingKeySpec {
        let mut modulus = vec![0xffu8; bytes];
        modulus[bytes - 1] = 0xfd;
        SealingKeySpec {
            kty: "RSA".to_string(),
            n: URL_SAFE_NO_PAD.encode(modulus),
            e: "AQAB".to_string(),
            alg: ALGORITHM.to_string(),
        }
    }

    #[test]
    fn a_secret_sealed_to_the_key_opens_only_with_its_private_half() {
        let key = SealingKey::try_from(&public_spec()).expect("accepted");

        let sealed = key.seal("s3cret-value").expect("sealed");

        assert_eq!(open(&sealed), "s3cret-value");
        assert!(!sealed.contains("s3cret-value"));
    }

    #[test]
    fn sealing_twice_gives_two_different_ciphertexts() {
        let key = SealingKey::try_from(&public_spec()).expect("accepted");

        assert_ne!(key.seal("same").unwrap(), key.seal("same").unwrap());
    }

    #[test]
    fn spec_ar_6_a_key_under_2048_bits_is_refused() {
        assert_eq!(
            SealingKey::try_from(&small(128)).unwrap_err(),
            SealingError::TooSmall
        );
        assert_eq!(
            SealingKey::try_from(&small(255)).unwrap_err(),
            SealingError::TooSmall
        );
    }

    #[test]
    fn a_key_over_8192_bits_is_refused() {
        assert_eq!(
            SealingKey::try_from(&small(1025)).unwrap_err(),
            SealingError::TooLarge
        );
    }

    #[test]
    fn a_key_that_is_not_rsa_or_not_for_oaep_256_is_refused() {
        let mut ec = public_spec();
        ec.kty = "EC".to_string();
        let mut other = public_spec();
        other.alg = "RSA-OAEP".to_string();

        assert_eq!(SealingKey::try_from(&ec).unwrap_err(), SealingError::NotRsa);
        assert_eq!(
            SealingKey::try_from(&other).unwrap_err(),
            SealingError::WrongAlgorithm
        );
    }

    #[test]
    fn a_weak_even_or_garbled_key_is_refused() {
        let mut weak = public_spec();
        weak.e = "Aw".to_string();
        let mut even = public_spec();
        even.e = "AQAC".to_string();
        let mut garbled = public_spec();
        garbled.n = "***".to_string();

        assert_eq!(
            SealingKey::try_from(&weak).unwrap_err(),
            SealingError::WeakExponent
        );
        assert_eq!(
            SealingKey::try_from(&even).unwrap_err(),
            SealingError::WeakExponent
        );
        assert_eq!(
            SealingKey::try_from(&garbled).unwrap_err(),
            SealingError::Malformed
        );
    }

    #[test]
    fn a_key_never_prints_its_material() {
        let key = SealingKey::try_from(&public_spec()).expect("accepted");

        assert!(!format!("{key:?}").contains(&public_spec().n));
    }
}
