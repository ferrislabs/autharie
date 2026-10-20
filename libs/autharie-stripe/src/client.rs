use std::time::Duration;

use autharie_domain::billing::BillingError;
use reqwest::{
    Body, Response, Url,
    header::{HeaderName, HeaderValue},
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tracing::debug;

use crate::{STRIPE_API_VERSION, form::Form};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const STRIPE_VERSION: HeaderName = HeaderName::from_static("stripe-version");
const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");
const DIGEST_BYTES: usize = 16;
const TOKEN_MAX_LEN: usize = 64;

pub(crate) fn failure(template: &str, cause: &str) -> BillingError {
    BillingError::Provider {
        reason: format!("POST {template}: {cause}"),
    }
}

pub(crate) fn malformed(template: &str) -> BillingError {
    failure(template, "the answer lacks what was asked for")
}

pub(crate) enum Key {
    Fixed(String),
    BodyDigest(String),
}

impl Key {
    fn resolve(self, body: &[u8]) -> String {
        match self {
            Self::Fixed(key) => key,
            Self::BodyDigest(prefix) => {
                let digest = Sha256::digest(body);
                format!("{prefix}:{}", hex::encode(&digest[..DIGEST_BYTES]))
            }
        }
    }
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: Option<ErrorDetail>,
}

#[derive(Deserialize)]
struct ErrorDetail {
    #[serde(rename = "type")]
    kind: Option<String>,
    code: Option<String>,
}

fn token(value: &str) -> Option<&str> {
    let shaped = !value.is_empty()
        && value.len() <= TOKEN_MAX_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');

    shaped.then_some(value)
}

pub(crate) struct Client {
    http: reqwest::Client,
    base: Url,
    secret_key: SecretString,
}

impl Client {
    pub(crate) fn new(secret_key: SecretString, api_base: &str) -> Result<Self, BillingError> {
        let unusable = || BillingError::Provider {
            reason: "the Stripe api_base is not a usable URL".to_string(),
        };

        let base = Url::parse(api_base.trim_end_matches('/')).map_err(|_| unusable())?;

        if base.cannot_be_a_base() {
            return Err(unusable());
        }

        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|_| BillingError::Provider {
                reason: "the Stripe HTTP client could not be built".to_string(),
            })?;

        Ok(Self {
            http,
            base,
            secret_key,
        })
    }

    pub(crate) fn base(&self) -> &str {
        self.base.as_str()
    }

    fn url(&self, template: &str, args: &[&str]) -> Option<Url> {
        let mut args = args.iter().copied();
        let mut url = self.base.clone();

        {
            let mut segments = url.path_segments_mut().ok()?;
            segments.pop_if_empty();

            for segment in template.split('/').filter(|segment| !segment.is_empty()) {
                let segment = if segment.starts_with('{') {
                    args.next()?
                } else {
                    segment
                };

                if matches!(segment, "" | "." | "..") {
                    return None;
                }

                segments.push(segment);
            }
        }

        Some(url)
    }

    pub(crate) async fn post<T: DeserializeOwned>(
        &self,
        template: &'static str,
        args: &[&str],
        form: &Form<'_>,
        key: Key,
    ) -> Result<T, BillingError> {
        debug!(method = "POST", path = template, "stripe request");

        let url = self
            .url(template, args)
            .ok_or_else(|| failure(template, "the path is not valid"))?;

        let mut request = self
            .http
            .post(url)
            .bearer_auth(self.secret_key.expose_secret())
            .header(STRIPE_VERSION, STRIPE_API_VERSION)
            .form(form.pairs())
            .build()
            .map_err(|_| failure(template, "the request could not be built"))?;

        let body = request.body().and_then(Body::as_bytes).unwrap_or_default();
        let idempotency = HeaderValue::from_str(&key.resolve(body))
            .map_err(|_| failure(template, "the idempotency key is not a valid header"))?;
        request.headers_mut().insert(IDEMPOTENCY_KEY, idempotency);

        let response = self
            .http
            .execute(request)
            .await
            .map_err(|error| transport(template, &error))?;

        if !response.status().is_success() {
            return Err(refusal(template, response).await);
        }

        response
            .json()
            .await
            .map_err(|_| failure(template, "the answer could not be read"))
    }
}

fn transport(template: &str, error: &reqwest::Error) -> BillingError {
    let cause = if error.is_timeout() {
        "the request timed out"
    } else if error.is_connect() {
        "the connection failed"
    } else {
        "the request failed"
    };

    failure(template, cause)
}

async fn refusal(template: &str, response: Response) -> BillingError {
    let status = response.status().as_u16();
    let detail = response
        .bytes()
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ErrorEnvelope>(&bytes).ok())
        .and_then(|envelope| envelope.error);

    let mut reason = format!("POST {template} answered {status}");

    if let Some(kind) = detail
        .as_ref()
        .and_then(|detail| detail.kind.as_deref())
        .and_then(token)
    {
        reason.push_str(&format!(", type {kind}"));
    }

    if let Some(code) = detail
        .as_ref()
        .and_then(|detail| detail.code.as_deref())
        .and_then(token)
    {
        reason.push_str(&format!(", code {code}"));
    }

    BillingError::Provider { reason }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(base: &str) -> Client {
        Client::new(SecretString::from("sk_test_x"), base).unwrap()
    }

    #[test]
    fn a_template_is_filled_with_encoded_segments() {
        let url = client("https://api.stripe.test")
            .url("/v1/invoices/{invoice}/finalize", &["in_1"])
            .unwrap();

        assert_eq!(
            url.as_str(),
            "https://api.stripe.test/v1/invoices/in_1/finalize"
        );
    }

    #[test]
    fn a_trailing_slash_and_a_path_prefix_on_the_base_are_kept_tidy() {
        let url = client("https://proxy.test/stripe/")
            .url("/v1/customers", &[])
            .unwrap();

        assert_eq!(url.as_str(), "https://proxy.test/stripe/v1/customers");
    }

    #[test]
    fn a_segment_cannot_add_a_path_a_query_or_a_fragment() {
        let url = client("https://api.stripe.test")
            .url("/v1/customers/{customer}", &["a/b?c=d#e%2F"])
            .unwrap();

        assert_eq!(
            url.as_str(),
            "https://api.stripe.test/v1/customers/a%2Fb%3Fc=d%23e%252F"
        );
        assert_eq!(url.query(), None);
        assert_eq!(url.fragment(), None);
    }

    #[test]
    fn dot_segments_are_refused() {
        let client = client("https://api.stripe.test");

        for segment in ["", ".", ".."] {
            assert!(
                client.url("/v1/customers/{customer}", &[segment]).is_none(),
                "{segment:?}"
            );
        }
    }

    #[test]
    fn a_template_asking_for_more_arguments_than_given_is_refused() {
        assert!(
            client("https://api.stripe.test")
                .url("/v1/customers/{customer}", &[])
                .is_none()
        );
    }

    #[test]
    fn a_base_that_is_not_a_url_is_refused_with_a_fixed_reason() {
        for base in ["", "not a url", "mailto:someone@example.test"] {
            let error = Client::new(SecretString::from("sk_test_x"), base)
                .err()
                .unwrap();

            assert_eq!(
                error,
                BillingError::Provider {
                    reason: "the Stripe api_base is not a usable URL".to_string()
                }
            );
        }
    }

    #[test]
    fn only_lowercase_tokens_are_echoed_from_an_error() {
        assert_eq!(token("card_declined"), Some("card_declined"));
        assert_eq!(
            token("invalid_request_error"),
            Some("invalid_request_error")
        );
        assert_eq!(token(""), None);
        assert_eq!(token("Has Space"), None);
        assert_eq!(token("sk_test_ABC"), None);
        assert_eq!(token(&"a".repeat(65)), None);
    }

    #[test]
    fn a_body_digest_key_is_the_prefix_and_32_hex_characters() {
        let key = Key::BodyDigest("customer:cus_1:update".to_string()).resolve(b"name=Acme");

        let digest = key.strip_prefix("customer:cus_1:update:").unwrap();
        assert_eq!(digest.len(), 32);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            key,
            Key::BodyDigest("customer:cus_1:update".to_string()).resolve(b"name=Acme")
        );
        assert_ne!(
            key,
            Key::BodyDigest("customer:cus_1:update".to_string()).resolve(b"name=Other")
        );
    }
}
