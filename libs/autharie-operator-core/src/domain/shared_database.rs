use thiserror::Error;
use uuid::Uuid;

const INSTANCE_PREFIX: &str = "deployment-";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SharedDatabaseName(String);

impl SharedDatabaseName {
    pub fn for_deployment(deployment_id: Uuid) -> Self {
        Self(format!("free_{}", deployment_id.simple()))
    }

    pub fn for_instance_name(instance_name: &str) -> Result<Self, SharedDatabaseError> {
        instance_name
            .strip_prefix(INSTANCE_PREFIX)
            .and_then(|id| Uuid::parse_str(id).ok())
            .map(Self::for_deployment)
            .ok_or_else(|| {
                SharedDatabaseError::Refused(format!(
                    "`{instance_name}` is not named after a deployment id, so no shared database name can be derived"
                ))
            })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SharedDatabaseError {
    #[error("the shared database cluster could not be reached")]
    Unreachable,

    #[error("the shared database cluster refused: {0}")]
    Refused(String),
}

impl SharedDatabaseError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Unreachable => "SharedDatabaseUnreachable",
            Self::Refused(_) => "SharedDatabaseRefused",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "b6a1c2d3-e4f5-4a6b-8c9d-0e1f2a3b4c5d";

    #[test]
    fn the_name_is_free_then_the_id_without_dashes() {
        let name = SharedDatabaseName::for_deployment(Uuid::parse_str(ID).unwrap());

        assert_eq!(name.as_str(), "free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d");
        assert_eq!(name.as_str().len(), 5 + 32);
    }

    #[test]
    fn the_name_is_deterministic() {
        let id = Uuid::parse_str(ID).unwrap();

        assert_eq!(
            SharedDatabaseName::for_deployment(id),
            SharedDatabaseName::for_deployment(id)
        );
    }

    #[test]
    fn two_deployments_never_share_a_name() {
        assert_ne!(
            SharedDatabaseName::for_deployment(Uuid::from_u128(1)),
            SharedDatabaseName::for_deployment(Uuid::from_u128(2))
        );
    }

    #[test]
    fn the_name_is_only_lowercase_hex_after_the_prefix() {
        let name = SharedDatabaseName::for_deployment(Uuid::new_v4());

        assert!(
            name.as_str()["free_".len()..]
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
    }

    #[test]
    fn an_instance_named_after_a_deployment_gives_the_same_name() {
        let from_instance = SharedDatabaseName::for_instance_name(&format!("deployment-{ID}"));

        assert_eq!(
            from_instance,
            Ok(SharedDatabaseName::for_deployment(
                Uuid::parse_str(ID).unwrap()
            ))
        );
    }

    #[test]
    fn an_instance_not_named_after_a_deployment_is_refused() {
        for name in ["acme", "deployment-not-a-uuid", ID] {
            assert!(matches!(
                SharedDatabaseName::for_instance_name(name),
                Err(SharedDatabaseError::Refused(_))
            ));
        }
    }

    #[test]
    fn each_error_has_its_own_reason() {
        assert_eq!(
            SharedDatabaseError::Unreachable.reason(),
            "SharedDatabaseUnreachable"
        );
        assert_eq!(
            SharedDatabaseError::Refused(String::new()).reason(),
            "SharedDatabaseRefused"
        );
    }
}
