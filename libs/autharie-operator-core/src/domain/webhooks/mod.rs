pub mod deliveries;
pub mod plan;
pub mod ports;
pub mod service;

use std::collections::BTreeMap;

use autharie_crds::v1alpha::identity_instance::WebhookSpec;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const MANAGED_PREFIX: &str = "autharie:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookError {
    /// Worth trying again as is: FerrisKey was down, slow or busy.
    Retry { message: String },
    /// FerrisKey (or the cluster) said no; the same call will say it again.
    Rejected { message: String },
}

impl WebhookError {
    pub fn message(&self) -> &str {
        match self {
            Self::Retry { message } | Self::Rejected { message } => message,
        }
    }

    pub fn is_retry(&self) -> bool {
        matches!(self, Self::Retry { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteWebhook {
    pub ferriskey_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub endpoint: String,
    pub events: Vec<String>,
}

/// A FerrisKey webhook this operator owns, found in `realm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedRemote {
    pub realm: String,
    pub autharie_id: String,
    pub webhook: RemoteWebhook,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookWrite {
    pub name: String,
    pub description: String,
    pub endpoint: String,
    pub events: Vec<String>,
    /// Absent leaves the headers FerrisKey already holds; FerrisKey never
    /// returns them, so this is the only way to change them.
    pub headers: Option<BTreeMap<String, String>>,
}

pub fn managed_name(autharie_id: &str) -> String {
    format!("{MANAGED_PREFIX}{autharie_id}")
}

/// The Autharie id in a managed webhook's name. The id has to be a UUID, so a
/// webhook someone happened to name `autharie:something` is still theirs.
pub fn managed_id(name: &str) -> Option<&str> {
    let id = name.strip_prefix(MANAGED_PREFIX)?;
    Uuid::parse_str(id).ok()?;
    Some(id)
}

/// FerrisKey's name is taken by the marker, so the human name travels in the
/// description: `<name> — <description>`, or just the name without one.
pub fn remote_description(spec: &WebhookSpec) -> String {
    match spec
        .description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty())
    {
        Some(description) => format!("{} — {description}", spec.name),
        None => spec.name.clone(),
    }
}

pub fn header_key(webhook_id: &str, header_name: &str) -> String {
    format!("{webhook_id}.{header_name}")
}

/// The header values a webhook needs, or the names of the ones not in the
/// Secret yet.
pub fn resolve_headers(
    spec: &WebhookSpec,
    stored: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, Vec<String>> {
    let mut found = BTreeMap::new();
    let mut missing = Vec::new();
    for name in &spec.header_names {
        match stored.get(&header_key(&spec.id, name)) {
            Some(value) => {
                found.insert(name.clone(), value.clone());
            }
            None => missing.push(name.clone()),
        }
    }
    if missing.is_empty() {
        Ok(found)
    } else {
        Err(missing)
    }
}

/// Stands in for the header values, which FerrisKey never hands back: the
/// operator remembers what it last sent and sends again when this changes.
pub fn headers_fingerprint(headers: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    for (name, value) in headers {
        hasher.update((name.len() as u64).to_be_bytes());
        hasher.update(name.as_bytes());
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";

    fn spec(description: Option<&str>, header_names: &[&str]) -> WebhookSpec {
        WebhookSpec {
            id: ID.to_string(),
            realm: "master".to_string(),
            name: "Audit".to_string(),
            description: description.map(ToString::to_string),
            endpoint: "https://example.com/hook".to_string(),
            events: vec!["user.created".to_string()],
            header_names: header_names.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn a_managed_name_carries_the_autharie_id() {
        assert_eq!(managed_name(ID), format!("autharie:{ID}"));
        assert_eq!(managed_id(&managed_name(ID)), Some(ID));
    }

    #[test]
    fn names_that_are_not_ours_are_not_managed() {
        for name in [
            "Audit",
            "autharie:",
            "autharie:something",
            "Autharie:0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
            "x-autharie:0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
            "autharie:0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11-copy",
        ] {
            assert_eq!(managed_id(name), None, "{name}");
        }
    }

    #[test]
    fn the_description_keeps_the_human_name() {
        assert_eq!(
            remote_description(&spec(Some("to the SIEM"), &[])),
            "Audit — to the SIEM"
        );
        assert_eq!(remote_description(&spec(None, &[])), "Audit");
        assert_eq!(remote_description(&spec(Some("  "), &[])), "Audit");
    }

    #[test]
    fn headers_are_read_from_the_webhook_scoped_keys() {
        let stored = BTreeMap::from([
            (format!("{ID}.Authorization"), "Bearer x".to_string()),
            (format!("{ID}.X-Env"), "prod".to_string()),
            ("other.Authorization".to_string(), "no".to_string()),
        ]);

        let found = resolve_headers(&spec(None, &["Authorization", "X-Env"]), &stored).unwrap();

        assert_eq!(found.len(), 2);
        assert_eq!(found["Authorization"], "Bearer x");
    }

    #[test]
    fn a_missing_header_value_is_named() {
        let stored = BTreeMap::from([(format!("{ID}.Authorization"), "x".to_string())]);

        let missing = resolve_headers(&spec(None, &["Authorization", "X-Env"]), &stored);

        assert_eq!(missing, Err(vec!["X-Env".to_string()]));
    }

    #[test]
    fn a_webhook_without_headers_resolves_to_nothing() {
        assert_eq!(
            resolve_headers(&spec(None, &[]), &BTreeMap::new()),
            Ok(BTreeMap::new())
        );
    }

    #[test]
    fn the_fingerprint_follows_names_and_values_and_hides_them() {
        let one = BTreeMap::from([("A".to_string(), "secret-value".to_string())]);
        let same = one.clone();
        let other_value = BTreeMap::from([("A".to_string(), "secret-valuf".to_string())]);
        let other_name = BTreeMap::from([("B".to_string(), "secret-value".to_string())]);
        let shifted_one = BTreeMap::from([("AB".to_string(), "c".to_string())]);
        let shifted_two = BTreeMap::from([("A".to_string(), "Bc".to_string())]);

        assert_eq!(headers_fingerprint(&one), headers_fingerprint(&same));
        assert_ne!(headers_fingerprint(&one), headers_fingerprint(&other_value));
        assert_ne!(headers_fingerprint(&one), headers_fingerprint(&other_name));
        assert_ne!(
            headers_fingerprint(&shifted_one),
            headers_fingerprint(&shifted_two)
        );
        assert!(!headers_fingerprint(&one).contains("secret"));
        assert_eq!(headers_fingerprint(&one).len(), 64);
    }
}
