use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    billing::statement::StatementId, deployments::DeploymentId, generate_uuid_v7,
    organisation::OrganisationId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BillingSuspensionId(pub Uuid);

impl BillingSuspensionId {
    pub fn new() -> Self {
        Self(generate_uuid_v7())
    }
}

impl Default for BillingSuspensionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillingSuspension {
    pub id: BillingSuspensionId,
    pub organisation_id: OrganisationId,
    pub statement_id: StatementId,
    pub stopped: Vec<DeploymentId>,
    pub suspended_organisation: bool,
    pub suspended_at: DateTime<Utc>,
    pub lifted_at: Option<DateTime<Utc>>,
}

impl BillingSuspension {
    pub fn is_active(&self) -> bool {
        self.lifted_at.is_none()
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn suspension(lifted_at: Option<DateTime<Utc>>) -> BillingSuspension {
        BillingSuspension {
            id: BillingSuspensionId::new(),
            organisation_id: OrganisationId::new(),
            statement_id: StatementId::new(),
            stopped: vec![DeploymentId(Uuid::new_v4())],
            suspended_organisation: true,
            suspended_at: Utc.with_ymd_and_hms(2026, 12, 20, 0, 0, 0).unwrap(),
            lifted_at,
        }
    }

    #[test]
    fn a_suspension_is_active_until_it_is_lifted() {
        let lifted = Utc.with_ymd_and_hms(2026, 12, 22, 0, 0, 0).unwrap();

        assert!(suspension(None).is_active());
        assert!(!suspension(Some(lifted)).is_active());
    }

    #[test]
    fn suspension_ids_are_distinct() {
        assert_ne!(BillingSuspensionId::new(), BillingSuspensionId::default());
    }
}
