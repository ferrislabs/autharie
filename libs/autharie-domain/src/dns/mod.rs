//! Publishing where a deployment can be reached.
//!
//! `<deployment>.autharie.fr` cannot be one wildcard record
//! the way the control plane's own hostnames are: a deployment lands on one
//! of potentially many data planes, each its own cluster with its own
//! address (see [`crate::dataplane::entities::DataPlane::gateway_address`]).
//! Routing it needs a record naming that cluster specifically.
//!
//! [`DnsProvider`] is this platform's own shape for "somewhere that turns a
//! hostname into a record" -- OVH today, and whatever the adapter under this
//! port is on the day it is not. Configured like every other optional
//! integration here: absent means the feature does nothing, not that it
//! fails.

use thiserror::Error;

/// What went wrong asking the provider to change a record.
///
/// Two of these are worth telling apart: a provider that did not answer at
/// all is worth retrying, one that answered and refused is not -- an installer
/// waiting on a hostname needs to know which.
#[derive(Debug, Clone, Error)]
pub enum DnsError {
    #[error("the DNS provider is unreachable: {reason}")]
    ProviderUnavailable { reason: String },

    #[error("{operation} was refused: {reason}")]
    Refused { operation: String, reason: String },

    #[error("the DNS provider returned something this platform cannot read: {reason}")]
    Malformed { reason: String },
}

/// Somewhere that turns a hostname into a record pointing at an address.
///
/// One provider, one zone: an adapter is constructed for the zone it
/// publishes into, which is why `zone` is a method here rather than a
/// parameter on every call -- there is nowhere a caller could get a second
/// one from.
pub trait DnsProvider: Send + Sync {
    /// The zone this provider publishes records in, e.g. `autharie.fr`.
    fn zone(&self) -> &str;

    /// Creates or updates the record so `hostname` resolves to `target`.
    ///
    /// Idempotent: called again with the address it already carries is a
    /// no-op, which is what lets a reconciliation sweep call this on every
    /// deployment it knows about rather than tracking which ones changed.
    fn upsert_record(
        &self,
        hostname: &str,
        target: &str,
    ) -> impl Future<Output = Result<(), DnsError>> + Send;

    /// Removes the record for `hostname`.
    ///
    /// Idempotent: no matching record is the outcome asked for, not an error
    /// -- a deployment deleted before its data plane ever reported an address
    /// never had one to remove.
    fn delete_record(&self, hostname: &str) -> impl Future<Output = Result<(), DnsError>> + Send;
}

/// Labels the platform owns, or will, in the zone deployments are published
/// under. A deployment's hostname is flat (`<name>.<zone>`), so a customer
/// naming one `www` or `api` would otherwise make the platform publish over,
/// or delete, a record that is not theirs.
pub const RESERVED_HOSTNAME_LABELS: &[&str] = &[
    "account",
    "accounts",
    "admin",
    "api",
    "app",
    "assets",
    "auth",
    "autharie",
    "autoconfig",
    "autodiscover",
    "billing",
    "blog",
    "cdn",
    "cell",
    "cells",
    "console",
    "dashboard",
    "docs",
    "ferriskey",
    "ferrislabs",
    "ftp",
    "git",
    "grafana",
    "help",
    "id",
    "identity",
    "imap",
    "keycloak",
    "landing",
    "login",
    "mail",
    "metrics",
    "mx",
    "mx1",
    "ns",
    "ns1",
    "ns2",
    "oauth",
    "pop",
    "pop3",
    "portal",
    "registry",
    "secure",
    "security",
    "signin",
    "smtp",
    "sso",
    "static",
    "status",
    "support",
    "vpn",
    "webmail",
    "www",
];

pub const RESERVED_HOSTNAME_PREFIXES: &[&str] =
    &["admin", "api", "auth", "autharie", "ferriskey", "www"];

pub fn is_reserved_label(slug: &str) -> bool {
    RESERVED_HOSTNAME_LABELS.contains(&slug)
}

pub fn is_refused_name(slug: &str) -> bool {
    is_reserved_label(slug)
        || RESERVED_HOSTNAME_PREFIXES
            .iter()
            .any(|prefix| slug.starts_with(prefix))
}

pub fn label_under_zone(url: &str, zone: &str) -> Option<String> {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = without_scheme.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
        .split(':')
        .next()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let label = host.strip_suffix(&format!(".{}", zone.to_ascii_lowercase()))?;

    (!label.is_empty() && !label.contains('.')).then(|| label.to_string())
}

/// A deployment's own hostname, under whichever zone this installation
/// publishes DNS records in.
///
/// Flat: `<name>.<zone>`. Uniqueness of the name across the whole platform is
/// enforced by the database, and the labels the platform keeps for itself are
/// refused at creation (see [`is_reserved_label`]).
pub fn hostname_for(deployment_name: &str, zone: &str) -> String {
    format!(
        "{}.{zone}",
        crate::deployments::environment::slug(deployment_name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hostname_is_the_slugged_name_under_the_zone() {
        assert_eq!(
            hostname_for("My Deployment", "autharie.fr"),
            "my-deployment.autharie.fr"
        );
    }

    #[test]
    fn a_name_that_is_already_a_valid_label_is_left_alone() {
        assert_eq!(
            hostname_for("acme-prod", "autharie.fr"),
            "acme-prod.autharie.fr"
        );
        assert_eq!(hostname_for("demo", "autharie.fr"), "demo.autharie.fr");
    }

    #[test]
    fn every_reserved_label_is_a_lowercase_label_and_reported_reserved() {
        for label in RESERVED_HOSTNAME_LABELS {
            assert!(is_reserved_label(label));
            assert_eq!(crate::deployments::environment::slug(label), *label);
        }
    }

    #[test]
    fn the_reserved_lists_stay_sorted_and_without_duplicates() {
        for list in [RESERVED_HOSTNAME_LABELS, RESERVED_HOSTNAME_PREFIXES] {
            assert!(list.windows(2).all(|pair| pair[0] < pair[1]), "{list:?}");
        }
    }

    #[test]
    fn a_name_starting_like_a_platform_label_is_refused() {
        for name in [
            "api-gateway",
            "admin-panel",
            "auth0",
            "authentication",
            "autharie-support",
            "ferriskey-demo",
            "www2",
        ] {
            assert!(is_refused_name(name), "{name}");
        }
        for name in ["acme", "demo", "my-api", "billing-acme", "tenant-login"] {
            assert!(!is_refused_name(name), "{name}");
        }
    }

    #[test]
    fn a_platform_url_under_the_zone_names_its_label() {
        assert_eq!(
            label_under_zone("https://console.autharie.fr/realms/x", "autharie.fr").as_deref(),
            Some("console")
        );
        assert_eq!(
            label_under_zone("https://user@Quickwit.Autharie.fr:7280", "autharie.fr").as_deref(),
            Some("quickwit")
        );
        assert_eq!(label_under_zone("https://autharie.fr", "autharie.fr"), None);
        assert_eq!(
            label_under_zone("https://a.b.autharie.fr", "autharie.fr"),
            None
        );
        assert_eq!(label_under_zone("http://vault:8200", "autharie.fr"), None);
        assert_eq!(label_under_zone("", "autharie.fr"), None);
    }

    #[test]
    fn demo_and_ordinary_names_are_not_reserved() {
        assert!(!is_reserved_label("demo"));
        assert!(!is_reserved_label("acme-prod"));
        assert!(!is_reserved_label("www2"));
    }
}
