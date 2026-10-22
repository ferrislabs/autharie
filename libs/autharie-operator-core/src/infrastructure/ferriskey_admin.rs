use std::time::Duration;

use k8s_openapi::api::core::v1::Secret;
use kube::{Api, Client};
use serde_json::Value;

use autharie_crds::v1alpha::identity_instance::IdentityInstance;

use super::identity_instance::{
    FERRISKEY_API_PORT, FERRISKEY_API_ROOT_PATH, ferriskey_api_admin_secret_name,
    ferriskey_api_name,
};

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The realm every instance is created with, and the only one the operator can
/// name on its own: the admin account lives here.
pub const ADMIN_REALM: &str = "master";

pub struct AdminCredentials<'a> {
    pub username: &'a str,
    pub password: &'a str,
}

pub struct AdminToken {
    pub value: String,
    pub expires_in: Option<u64>,
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// The error is a message fit for a status: it never carries the credentials.
pub async fn request_token(
    http: &reqwest::Client,
    base_url: &str,
    credentials: &AdminCredentials<'_>,
) -> Result<AdminToken, String> {
    let response = http
        .post(format!(
            "{base_url}/realms/{ADMIN_REALM}/protocol/openid-connect/token"
        ))
        .form(&[
            ("grant_type", "password"),
            ("client_id", "admin-cli"),
            ("username", credentials.username),
            ("password", credentials.password),
        ])
        .send()
        .await
        .map_err(|error| unreachable_message(&error))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "FerrisKey refused an admin token (status {status})"
        ));
    }

    let value = serde_json::from_str::<Value>(&text).ok();
    let token = value
        .as_ref()
        .and_then(|value| value["access_token"].as_str())
        .map(ToOwned::to_owned);
    match token {
        Some(token) => Ok(AdminToken {
            value: token,
            expires_in: value
                .as_ref()
                .and_then(|value| value["expires_in"].as_u64()),
        }),
        None => Err("FerrisKey answered the token request without a token".to_string()),
    }
}

pub fn unreachable_message(error: &reqwest::Error) -> String {
    let reason = if error.is_timeout() {
        "timed out"
    } else {
        "unreachable"
    };
    format!("FerrisKey is {reason}")
}

pub fn error_message(status: u16, body: &str) -> String {
    let value = serde_json::from_str::<Value>(body).ok();
    let message = value
        .as_ref()
        .and_then(|value| value["message"].as_str())
        .map(ToOwned::to_owned);
    let issues = value.as_ref().map(issue_lines).unwrap_or_default();

    match (message, issues.is_empty()) {
        (Some(message), true) => format!("FerrisKey answered {status}: {message}"),
        (Some(message), false) => format!("FerrisKey answered {status}: {message} ({issues})"),
        (None, false) => format!("FerrisKey answered {status}: {issues}"),
        (None, true) => format!("FerrisKey answered {status}"),
    }
}

fn issue_lines(body: &Value) -> String {
    ["issues", "errors"]
        .iter()
        .filter_map(|key| body[*key].as_array())
        .flatten()
        .filter_map(|issue| match issue {
            Value::String(text) => Some(text.clone()),
            Value::Object(_) => {
                let text = issue["message"].as_str()?;
                match (issue["line"].as_u64(), issue["column"].as_u64()) {
                    (Some(line), Some(column)) => {
                        Some(format!("line {line}, column {column}: {text}"))
                    }
                    (Some(line), None) => Some(format!("line {line}: {text}")),
                    _ => Some(text.to_string()),
                }
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn api_base_url(name: &str, namespace: &str) -> String {
    format!(
        "http://{}.{namespace}.svc:{FERRISKEY_API_PORT}{FERRISKEY_API_ROOT_PATH}",
        ferriskey_api_name(name)
    )
}

pub fn name_and_namespace(instance: &IdentityInstance) -> Option<(&str, &str)> {
    Some((
        instance.metadata.name.as_deref()?,
        instance.metadata.namespace.as_deref()?,
    ))
}

pub async fn read_credentials(
    client: &Client,
    name: &str,
    namespace: &str,
) -> Result<(String, String), String> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let secret = secrets
        .get_opt(&ferriskey_api_admin_secret_name(name))
        .await
        .map_err(|_| "the admin credentials could not be read".to_string())?;

    let read = |key: &str| {
        secret
            .as_ref()
            .and_then(|secret| secret.data.as_ref())
            .and_then(|data| data.get(key))
            .and_then(|value| String::from_utf8(value.0.clone()).ok())
    };

    match (read("username"), read("password")) {
        (Some(username), Some(password)) => Ok((username, password)),
        _ => Err("the admin credentials are not there yet".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_message_is_quoted_with_the_status() {
        assert_eq!(
            error_message(400, r#"{"message":"bad realm"}"#),
            "FerrisKey answered 400: bad realm"
        );
    }

    #[test]
    fn a_body_without_a_message_names_only_the_status() {
        assert_eq!(error_message(502, "<html>"), "FerrisKey answered 502");
    }

    #[test]
    fn validation_issues_are_listed_with_their_position() {
        let body = r#"{"message":"invalid authorization policy","issues":[
            {"policy":"a","message":"unexpected token `)`","line":2,"column":7},
            {"message":"unknown action"}
        ]}"#;

        assert_eq!(
            error_message(422, body),
            "FerrisKey answered 422: invalid authorization policy (line 2, column 7: unexpected token `)`; unknown action)"
        );
    }

    #[test]
    fn issues_alone_are_enough() {
        assert_eq!(
            error_message(422, r#"{"errors":["no such entity type"]}"#),
            "FerrisKey answered 422: no such entity type"
        );
    }
}
