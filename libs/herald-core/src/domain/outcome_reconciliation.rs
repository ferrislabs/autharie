use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use super::entities::deployment::{Deployment, DeploymentId, Transition};
use super::entities::outcome::DeploymentOutcomeReport;

pub const QUIET_BEFORE_RECONCILING: Duration = Duration::minutes(5);
pub const RECONCILED_PER_CYCLE: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceState {
    Serving { version: String },
    Failed,
    Unsettled,
}

pub type ObservedInstances = HashMap<DeploymentId, InstanceState>;

pub fn awaiting_reconciliation<'a>(
    deployments: &'a [Deployment],
    busy: &HashSet<DeploymentId>,
    now: DateTime<Utc>,
) -> Vec<&'a Deployment> {
    let quiet_since = now - QUIET_BEFORE_RECONCILING;

    deployments
        .iter()
        .filter(|deployment| !deployment.pooled && deployment.transition.is_some())
        .filter(|deployment| deployment.changed_at.is_some_and(|at| at <= quiet_since))
        .filter(|deployment| !busy.contains(&deployment.id))
        .collect()
}

pub fn overdue_outcomes(
    candidates: &[&Deployment],
    observed: &ObservedInstances,
) -> Vec<DeploymentOutcomeReport> {
    candidates
        .iter()
        .filter_map(|deployment| overdue_outcome(deployment, observed.get(&deployment.id)))
        .take(RECONCILED_PER_CYCLE)
        .collect()
}

fn overdue_outcome(
    deployment: &Deployment,
    instance: Option<&InstanceState>,
) -> Option<DeploymentOutcomeReport> {
    let deployment_id = Uuid::parse_str(&deployment.id.0).ok()?;

    match (deployment.transition?, instance) {
        (Transition::Departing, None) => Some(DeploymentOutcomeReport::observed(
            deployment_id,
            "deleted",
            None,
        )),
        (Transition::Arriving, Some(InstanceState::Serving { version })) => Some(
            DeploymentOutcomeReport::observed(deployment_id, "running", Some(version.clone())),
        ),
        (Transition::Arriving, Some(InstanceState::Failed)) => Some(
            DeploymentOutcomeReport::observed(deployment_id, "failed", None),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::dataplane::DataPlaneId;
    use crate::domain::entities::logs::OrganisationId;

    fn now() -> DateTime<Utc> {
        "2026-10-07T02:00:00Z".parse().expect("a timestamp")
    }

    fn deployment(id: Uuid, transition: Option<Transition>) -> Deployment {
        Deployment {
            id: DeploymentId::new(id.to_string()),
            dataplane_id: DataPlaneId::new("dp-1"),
            organisation_id: OrganisationId::new("org-1"),
            name: "acme-prod".to_string(),
            kind: None,
            namespace: Some("autharie-acme-prod".to_string()),
            log_shipping_enabled: false,
            pooled: false,
            transition,
            changed_at: Some(now() - Duration::minutes(11)),
        }
    }

    fn reconciled(
        deployments: &[Deployment],
        observed: &ObservedInstances,
    ) -> Vec<DeploymentOutcomeReport> {
        let candidates = awaiting_reconciliation(deployments, &HashSet::new(), now());
        overdue_outcomes(&candidates, observed)
    }

    #[test]
    fn a_deployment_left_deleting_with_its_instance_gone_is_reported_deleted() {
        let id = Uuid::new_v4();

        let reports = reconciled(
            &[deployment(id, Some(Transition::Departing))],
            &ObservedInstances::new(),
        );

        assert_eq!(
            reports,
            vec![DeploymentOutcomeReport::observed(id, "deleted", None)]
        );
    }

    #[test]
    fn a_deployment_left_arriving_with_its_instance_serving_is_reported_running() {
        let id = Uuid::new_v4();
        let arriving = deployment(id, Some(Transition::Arriving));
        let observed = ObservedInstances::from([(
            arriving.id.clone(),
            InstanceState::Serving {
                version: "26.0.1".to_string(),
            },
        )]);

        let reports = reconciled(&[arriving], &observed);

        assert_eq!(
            reports,
            vec![DeploymentOutcomeReport::observed(
                id,
                "running",
                Some("26.0.1".to_string())
            )]
        );
    }

    #[test]
    fn a_deployment_left_arriving_with_its_instance_failed_is_reported_failed() {
        let id = Uuid::new_v4();
        let arriving = deployment(id, Some(Transition::Arriving));
        let observed = ObservedInstances::from([(arriving.id.clone(), InstanceState::Failed)]);

        assert_eq!(
            reconciled(&[arriving], &observed),
            vec![DeploymentOutcomeReport::observed(id, "failed", None)]
        );
    }

    #[test]
    fn nothing_is_reported_while_the_cluster_agrees_with_the_control_plane() {
        let departing = deployment(Uuid::new_v4(), Some(Transition::Departing));
        let arriving = deployment(Uuid::new_v4(), Some(Transition::Arriving));
        let absent = deployment(Uuid::new_v4(), Some(Transition::Arriving));
        let observed = ObservedInstances::from([
            (departing.id.clone(), InstanceState::Unsettled),
            (arriving.id.clone(), InstanceState::Unsettled),
        ]);

        assert!(reconciled(&[departing, arriving, absent], &observed).is_empty());
    }

    #[test]
    fn a_settled_deployment_is_not_a_candidate() {
        let settled = deployment(Uuid::new_v4(), None);

        assert!(awaiting_reconciliation(&[settled], &HashSet::new(), now()).is_empty());
    }

    #[test]
    fn a_deployment_that_changed_recently_is_left_to_the_outcome_on_its_way() {
        let mut recent = deployment(Uuid::new_v4(), Some(Transition::Departing));
        recent.changed_at = Some(now() - Duration::minutes(1));
        let mut unknown = deployment(Uuid::new_v4(), Some(Transition::Departing));
        unknown.changed_at = None;

        assert!(awaiting_reconciliation(&[recent, unknown], &HashSet::new(), now()).is_empty());
    }

    #[test]
    fn a_realm_in_a_shared_cell_is_never_a_candidate() {
        let mut realm = deployment(Uuid::new_v4(), Some(Transition::Departing));
        realm.pooled = true;

        assert!(awaiting_reconciliation(&[realm], &HashSet::new(), now()).is_empty());
    }

    #[test]
    fn a_deployment_with_work_claimed_this_cycle_is_not_a_candidate() {
        let departing = deployment(Uuid::new_v4(), Some(Transition::Departing));
        let busy = HashSet::from([departing.id.clone()]);

        assert!(awaiting_reconciliation(&[departing], &busy, now()).is_empty());
    }

    #[test]
    fn one_cycle_reports_a_bounded_batch() {
        let deployments: Vec<Deployment> = (0..RECONCILED_PER_CYCLE + 5)
            .map(|_| deployment(Uuid::new_v4(), Some(Transition::Departing)))
            .collect();

        assert_eq!(
            reconciled(&deployments, &ObservedInstances::new()).len(),
            RECONCILED_PER_CYCLE
        );
    }
}
