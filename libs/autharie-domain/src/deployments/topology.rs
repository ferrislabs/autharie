use serde::Serialize;
use utoipa::ToSchema;

use crate::CoreError;

const RANGE: std::ops::RangeInclusive<u8> = 1..=5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, ToSchema)]
pub struct Topology {
    #[schema(minimum = 1, maximum = 5)]
    replicas: u8,
    #[schema(minimum = 1, maximum = 5)]
    database_instances: u8,
}

impl Topology {
    pub const SINGLE: Self = Self {
        replicas: 1,
        database_instances: 1,
    };

    pub const HIGH_AVAILABILITY: Self = Self {
        replicas: 2,
        database_instances: 2,
    };

    pub fn new(replicas: u8, database_instances: u8) -> Result<Self, CoreError> {
        if !RANGE.contains(&replicas) {
            return Err(CoreError::InvalidDeploymentResources {
                reason: format!("replicas must be from 1 to 5, not {replicas}"),
            });
        }

        if !RANGE.contains(&database_instances) {
            return Err(CoreError::InvalidDeploymentResources {
                reason: format!("database instances must be from 1 to 5, not {database_instances}"),
            });
        }

        Ok(Self {
            replicas,
            database_instances,
        })
    }

    pub fn replicas(&self) -> u8 {
        self.replicas
    }

    pub fn database_instances(&self) -> u8 {
        self.database_instances
    }

    pub fn is_highly_available(&self) -> bool {
        self.replicas >= 2 && self.database_instances >= 2
    }
}

impl Default for Topology {
    fn default() -> Self {
        Self::SINGLE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_count_from_one_to_five_is_accepted() {
        for count in 1..=5 {
            assert!(Topology::new(count, 1).is_ok());
            assert!(Topology::new(1, count).is_ok());
        }
    }

    #[test]
    fn zero_and_six_are_refused_for_each_field() {
        for count in [0, 6] {
            assert!(matches!(
                Topology::new(count, 1),
                Err(CoreError::InvalidDeploymentResources { .. })
            ));
            assert!(matches!(
                Topology::new(1, count),
                Err(CoreError::InvalidDeploymentResources { .. })
            ));
        }
    }

    #[test]
    fn the_constants_say_what_they_name() {
        assert_eq!(Topology::SINGLE, Topology::new(1, 1).unwrap());
        assert_eq!(Topology::HIGH_AVAILABILITY, Topology::new(2, 2).unwrap());
        assert_eq!(Topology::default(), Topology::SINGLE);
    }

    #[test]
    fn high_availability_needs_both_halves() {
        assert!(!Topology::SINGLE.is_highly_available());
        assert!(Topology::HIGH_AVAILABILITY.is_highly_available());
        assert!(!Topology::new(2, 1).unwrap().is_highly_available());
        assert!(!Topology::new(1, 2).unwrap().is_highly_available());
        assert!(Topology::new(5, 3).unwrap().is_highly_available());
    }

    #[test]
    fn it_serialises_both_counts() {
        let value = serde_json::to_value(Topology::HIGH_AVAILABILITY).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"replicas": 2, "database_instances": 2})
        );
    }
}
