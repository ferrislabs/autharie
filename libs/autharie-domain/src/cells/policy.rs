use super::CellError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPolicy {
    capacity: u16,
    spare_slots: u16,
}

impl CellPolicy {
    pub fn new(capacity: u16, spare_slots: u16) -> Result<Self, CellError> {
        if capacity == 0 || spare_slots >= capacity {
            return Err(CellError::CapacityInvalid);
        }

        Ok(Self {
            capacity,
            spare_slots,
        })
    }

    pub fn capacity(&self) -> u16 {
        self.capacity
    }

    pub fn spare_slots(&self) -> u16 {
        self.spare_slots
    }
}

impl Default for CellPolicy {
    fn default() -> Self {
        Self {
            capacity: 200,
            spare_slots: 50,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_defaults_to_200_realms_and_50_spare_slots() {
        let policy = CellPolicy::default();

        assert_eq!(policy.capacity(), 200);
        assert_eq!(policy.spare_slots(), 50);
    }

    #[test]
    fn a_policy_without_capacity_is_refused() {
        assert_eq!(CellPolicy::new(0, 0), Err(CellError::CapacityInvalid));
    }

    #[test]
    fn a_policy_whose_spare_slots_reach_the_capacity_is_refused() {
        assert_eq!(CellPolicy::new(10, 10), Err(CellError::CapacityInvalid));
        assert_eq!(CellPolicy::new(10, 11), Err(CellError::CapacityInvalid));
    }

    #[test]
    fn a_policy_with_spare_slots_below_the_capacity_is_built() {
        let policy = CellPolicy::new(10, 9).expect("valid policy");

        assert_eq!(policy.capacity(), 10);
        assert_eq!(policy.spare_slots(), 9);
    }
}
