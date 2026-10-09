use std::fmt;

use serde::Serialize;
use utoipa::ToSchema;

use crate::{CoreError, deployments::DeploymentName};

use super::RealmError;

const MAX_LENGTH: usize = 63;
const ADMINISTRATOR_REALMS: [&str; 1] = ["master"];

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
pub struct RealmName(String);

impl RealmName {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for RealmName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<&str> for RealmName {
    type Error = CoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let invalid = |reason: &str| {
            CoreError::Realm(RealmError::InvalidName {
                reason: format!("'{value}' is not a valid realm name: {reason}"),
            })
        };

        if value.is_empty() || value.len() > MAX_LENGTH {
            return Err(invalid("it must be 1 to 63 characters"));
        }
        if !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(invalid("only a-z, 0-9 and '-' are allowed"));
        }
        if value.starts_with('-') || value.ends_with('-') {
            return Err(invalid("it must not start or end with '-'"));
        }

        if value.as_bytes().get(2..4) == Some(b"--") {
            return Err(invalid(
                "'--' is reserved at the third and fourth characters",
            ));
        }
        if ADMINISTRATOR_REALMS.contains(&value) {
            return Err(invalid("it is the administrator realm of a cell"));
        }

        let name = DeploymentName(value.to_owned())
            .publishable()
            .map_err(|_| invalid("it is reserved"))?;

        Ok(Self(name.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lowercase_dns_label_is_a_realm_name() {
        for value in ["acme", "acme-prod", "a", "a1-b2"] {
            let name = RealmName::try_from(value).expect("valid name");
            assert_eq!(name.as_str(), value);
        }
    }

    #[test]
    fn a_label_of_63_characters_is_accepted_and_64_is_refused() {
        assert!(RealmName::try_from("a".repeat(63).as_str()).is_ok());
        assert!(RealmName::try_from("a".repeat(64).as_str()).is_err());
    }

    #[test]
    fn a_malformed_label_is_refused() {
        for value in [
            "",
            "Acme",
            "-acme",
            "acme-",
            "ac me",
            "acme.io",
            "acmé",
            "acme_prod",
        ] {
            assert!(RealmName::try_from(value).is_err(), "{value}");
        }
    }

    #[test]
    fn a_badly_formed_name_is_the_callers_mistake_and_not_a_refusal_by_the_cell() {
        let error = RealmName::try_from("-acme").expect_err("a leading dash");

        assert!(
            matches!(error, CoreError::Realm(RealmError::InvalidName { .. })),
            "{error}"
        );
    }

    #[test]
    fn the_administrator_realm_of_a_cell_is_never_a_tenant_name() {
        let error = RealmName::try_from("master").expect_err("the administrator realm");

        assert!(
            matches!(error, CoreError::Realm(RealmError::InvalidName { .. })),
            "{error}"
        );
    }

    #[test]
    fn a_label_that_reads_as_punycode_is_refused() {
        assert!(RealmName::try_from("xn--abc").is_err());
        assert!(RealmName::try_from("a-b--c").is_ok());
    }

    #[test]
    fn a_reserved_slug_is_an_invalid_name_like_the_others() {
        let error = RealmName::try_from("www").expect_err("a reserved slug");

        assert!(
            matches!(error, CoreError::Realm(RealmError::InvalidName { .. })),
            "{error}"
        );
    }
}
