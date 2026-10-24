use std::fmt;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::IamError;

pub const MAX_ENVELOPE_BYTES: usize = 700 * 1024;
pub const ENVELOPE_VERSION: u8 = 1;
pub const ENVELOPE_ALG: &str = "RSA-OAEP-256+A256GCM";

const IV_BYTES: usize = 12;
const GCM_TAG_BYTES: usize = 16;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    v: u8,
    alg: String,
    ek: String,
    iv: String,
    ct: String,
}

impl fmt::Debug for Envelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Envelope")
            .field("alg", &self.alg)
            .field("sealed", &"<sealed>")
            .finish()
    }
}

fn refuse(reason: &str) -> IamError {
    IamError::invalid(format!("the sealed result is not valid: {reason}"))
}

fn decoded_len(field: &str, name: &str) -> Result<usize, IamError> {
    URL_SAFE_NO_PAD
        .decode(field)
        .map(|bytes| bytes.len())
        .map_err(|_| refuse(&format!("{name} is not base64url")))
}

impl Envelope {
    pub fn parse(value: Value) -> Result<Self, IamError> {
        if !value.is_object() {
            return Err(refuse("an envelope is an object"));
        }
        let size = serde_json::to_vec(&value)
            .map_err(|_| refuse("it cannot be read"))?
            .len();
        if size > MAX_ENVELOPE_BYTES {
            return Err(refuse("it is larger than 700 KiB"));
        }

        let envelope: Self = serde_json::from_value(value)
            .map_err(|_| refuse("it needs exactly v, alg, ek, iv and ct"))?;

        if envelope.v != ENVELOPE_VERSION {
            return Err(refuse("only version 1 exists"));
        }
        if envelope.alg != ENVELOPE_ALG {
            return Err(refuse("the algorithm is not RSA-OAEP-256+A256GCM"));
        }
        if decoded_len(&envelope.ek, "ek")? == 0 {
            return Err(refuse("ek is empty"));
        }
        if decoded_len(&envelope.iv, "iv")? != IV_BYTES {
            return Err(refuse("iv is not 12 bytes"));
        }
        if decoded_len(&envelope.ct, "ct")? < GCM_TAG_BYTES {
            return Err(refuse("ct is shorter than its tag"));
        }

        Ok(envelope)
    }

    pub fn into_value(self) -> Result<Value, IamError> {
        serde_json::to_value(self).map_err(|_| refuse("it cannot be written"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn b64(bytes: usize) -> String {
        URL_SAFE_NO_PAD.encode(vec![7u8; bytes])
    }

    fn envelope() -> Value {
        json!({
            "v": 1,
            "alg": "RSA-OAEP-256+A256GCM",
            "ek": b64(256),
            "iv": b64(12),
            "ct": b64(64),
        })
    }

    #[test]
    fn a_well_formed_envelope_is_kept_exactly_as_received() {
        let parsed = Envelope::parse(envelope()).expect("accepted");

        assert_eq!(parsed.into_value().unwrap(), envelope());
    }

    #[test]
    fn what_is_not_an_object_is_refused() {
        for value in [json!(null), json!("x"), json!([1]), json!(1)] {
            assert!(Envelope::parse(value).is_err());
        }
    }

    #[test]
    fn a_missing_wrong_or_extra_field_is_refused() {
        for field in ["v", "alg", "ek", "iv", "ct"] {
            let mut missing = envelope();
            missing.as_object_mut().unwrap().remove(field);
            assert!(Envelope::parse(missing).is_err(), "{field}");
        }

        let mut extra = envelope();
        extra["plain"] = json!("clear text");
        assert!(Envelope::parse(extra).is_err());

        let mut version = envelope();
        version["v"] = json!(2);
        assert!(Envelope::parse(version).is_err());

        let mut alg = envelope();
        alg["alg"] = json!("none");
        assert!(Envelope::parse(alg).is_err());

        let mut number = envelope();
        number["ct"] = json!(12);
        assert!(Envelope::parse(number).is_err());
    }

    #[test]
    fn fields_are_unpadded_base64url_of_the_right_shape() {
        for (field, value) in [
            ("ek", json!("not base64 !")),
            ("ek", json!("")),
            ("iv", json!(b64(11))),
            ("iv", json!(format!("{}=", b64(12)))),
            ("ct", json!(b64(15))),
            ("ct", json!("a+b/")),
        ] {
            let mut bad = envelope();
            bad[field] = value;
            assert!(Envelope::parse(bad).is_err(), "{field}");
        }
    }

    #[test]
    fn an_envelope_over_700_kib_is_refused_and_one_just_under_is_kept() {
        let mut large = envelope();
        large["ct"] = json!(URL_SAFE_NO_PAD.encode(vec![0u8; 540 * 1024]));
        assert!(Envelope::parse(large).is_err());

        let mut fits = envelope();
        fits["ct"] = json!(URL_SAFE_NO_PAD.encode(vec![0u8; 500 * 1024]));
        assert!(Envelope::parse(fits).is_ok());
    }

    #[test]
    fn an_envelope_does_not_print_its_content() {
        let printed = format!("{:?}", Envelope::parse(envelope()).unwrap());

        assert!(!printed.contains(&b64(64)));
    }
}
