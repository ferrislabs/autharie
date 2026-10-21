use std::{net::IpAddr, time::Duration};

use autharie_domain::billing::BillingError;
use reqwest::{
    Method, Request, Response, Url,
    header::{HeaderName, HeaderValue},
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, de::DeserializeOwned};
use tracing::debug;

use crate::{STRIPE_API_VERSION, form::Form};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const STRIPE_VERSION: HeaderName = HeaderName::from_static("stripe-version");
const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");
const TOKEN_MAX_LEN: usize = 64;

pub(crate) fn failure(method: &Method, template: &str, cause: &str) -> BillingError {
    BillingError::Provider {
        reason: format!("{method} {template}: {cause}"),
    }
}

pub(crate) fn malformed(method: &Method, template: &str) -> BillingError {
    failure(method, template, "the answer lacks what was asked for")
}

pub(crate) fn https_url(
    method: &Method,
    template: &str,
    url: Option<String>,
) -> Result<String, BillingError> {
    let url = url.ok_or_else(|| malformed(method, template))?;

    match Url::parse(&url) {
        Ok(parsed) if parsed.scheme() == "https" && parsed.has_host() => Ok(url),
        _ => Err(failure(
            method,
            template,
            "the answer carries a URL that is not an https one",
        )),
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

fn is_loopback(base: &Url) -> bool {
    let Some(host) = base.host_str() else {
        return false;
    };

    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn is_trustworthy(base: &Url) -> bool {
    match base.scheme() {
        "https" => true,
        "http" => is_loopback(base),
        _ => false,
    }
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

        if !is_trustworthy(&base) {
            return Err(BillingError::NotConfigured);
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
        key: &str,
    ) -> Result<T, BillingError> {
        debug!(method = "POST", path = template, "stripe request");

        let url = self
            .url(template, args)
            .ok_or_else(|| failure(&Method::POST, template, "the path is not valid"))?;
        let idempotency = HeaderValue::from_str(key).map_err(|_| {
            failure(
                &Method::POST,
                template,
                "the idempotency key is not a valid header",
            )
        })?;

        let request = self
            .http
            .post(url)
            .bearer_auth(self.secret_key.expose_secret())
            .header(STRIPE_VERSION, STRIPE_API_VERSION)
            .header(IDEMPOTENCY_KEY, idempotency)
            .form(form.pairs())
            .build()
            .map_err(|_| failure(&Method::POST, template, "the request could not be built"))?;

        self.run(&Method::POST, template, request).await
    }

    pub(crate) async fn get<T: DeserializeOwned>(
        &self,
        template: &'static str,
        args: &[&str],
        query: &[(&str, &str)],
    ) -> Result<T, BillingError> {
        debug!(method = "GET", path = template, "stripe request");

        let url = self
            .url(template, args)
            .ok_or_else(|| failure(&Method::GET, template, "the path is not valid"))?;

        let request = self
            .http
            .get(url)
            .bearer_auth(self.secret_key.expose_secret())
            .header(STRIPE_VERSION, STRIPE_API_VERSION)
            .query(query)
            .build()
            .map_err(|_| failure(&Method::GET, template, "the request could not be built"))?;

        self.run(&Method::GET, template, request).await
    }

    async fn run<T: DeserializeOwned>(
        &self,
        method: &Method,
        template: &str,
        request: Request,
    ) -> Result<T, BillingError> {
        let response = self
            .http
            .execute(request)
            .await
            .map_err(|error| transport(method, template, &error))?;

        if !response.status().is_success() {
            return Err(refusal(method, template, response).await);
        }

        response
            .json()
            .await
            .map_err(|_| failure(method, template, "the answer could not be read"))
    }
}

fn transport(method: &Method, template: &str, error: &reqwest::Error) -> BillingError {
    let cause = if error.is_timeout() {
        "the request timed out"
    } else if error.is_connect() {
        "the connection failed"
    } else {
        "the request failed"
    };

    failure(method, template, cause)
}

async fn refusal(method: &Method, template: &str, response: Response) -> BillingError {
    let status = response.status().as_u16();
    let detail = response
        .bytes()
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ErrorEnvelope>(&bytes).ok())
        .and_then(|envelope| envelope.error);

    let mut reason = format!("{method} {template} answered {status}");

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
    fn a_base_that_is_not_https_is_refused_unless_it_is_a_loopback_address() {
        for base in [
            "http://api.stripe.com",
            "http://127.0.0.1.evil.test",
            "http://localhost.evil.test",
            "http://0.0.0.0",
            "http://[::ffff:7f00:1]",
            "ftp://127.0.0.1",
        ] {
            let error = Client::new(SecretString::from("sk_test_x"), base)
                .err()
                .unwrap();

            assert_eq!(error, BillingError::NotConfigured, "{base}");
        }

        for base in [
            "https://api.stripe.com",
            "http://127.0.0.1:8080",
            "http://127.1",
            "http://localhost",
            "http://LocalHost:9",
            "http://[::1]:8080",
        ] {
            assert!(
                Client::new(SecretString::from("sk_test_x"), base).is_ok(),
                "{base}"
            );
        }
    }

    #[test]
    fn only_an_https_url_with_a_host_is_let_through() {
        let template = "/v1/checkout/sessions";

        assert_eq!(
            https_url(
                &Method::POST,
                template,
                Some("https://checkout.stripe.test/c/1".to_string())
            ),
            Ok("https://checkout.stripe.test/c/1".to_string())
        );

        for hostile in [
            "http://checkout.stripe.test/c/1",
            "javascript:alert(1)",
            "data:text/html,hi",
            "https://",
            "//checkout.stripe.test",
            "not a url",
            "",
        ] {
            let Err(BillingError::Provider { reason }) =
                https_url(&Method::POST, template, Some(hostile.to_string()))
            else {
                panic!("{hostile:?} was let through");
            };

            assert!(reason.contains(template), "{reason}");
            assert!(hostile.is_empty() || !reason.contains(hostile), "{reason}");
        }

        assert!(matches!(
            https_url(&Method::GET, template, None),
            Err(BillingError::Provider { reason }) if reason.starts_with("GET ")
        ));
    }
}
