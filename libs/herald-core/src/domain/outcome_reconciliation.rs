use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use super::entities::deployment::{Deployment, DeploymentId, Transition};
use super::entities::outcome::DeploymentOutcomeReport;

pub const QUIET_BEFORE_RECONCILING: Duration = Duration::minutes(5);
pub const RECONCILED_PER_CYCLE: usize = 16;
pub const RECONCILE_EVERY_CYCLES: u64 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceState {
    Serving { version: String },
    Failed,
    Unsettled,
}

pub type ObservedInstances = HashMap<DeploymentId, InstanceState>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overdue {
    pub deployment_id: DeploymentId,
    pub report: DeploymentOutcomeReport,
    pub absence_to_confirm_in: Option<String>,
}

pub fn is_reconciling_cycle(cycle: u64) -> bool {
    cycle.is_multiple_of(RECONCILE_EVERY_CYCLES)
}

pub fn awaiting_reconciliation<'a>(
    deployments: &'a [Deployment],
    busy: &HashSet<DeploymentId>,
    now: DateTime<Utc>,
) -> Vec<&'a Deployment> {
    let quiet_since = now - QUIET_BEFORE_RECONCILING;

    deployments
        .iter()
        .filter(|deployment| deployment.transition.is_some())
        .filter(|deployment| deployment.changed_at.is_some_and(|at| at <= quiet_since))
        .filter(|deployment| !busy.contains(&deployment.id))
        .collect()
}

pub fn overdue_outcomes(candidates: &[&Deployment], observed: &ObservedInstances) -> Vec<Overdue> {
    candidates
        .iter()
        .filter_map(|deployment| overdue_outcome(deployment, observed.get(&deployment.id)))
        .collect()
}

fn overdue_outcome(deployment: &Deployment, instance: Option<&InstanceState>) -> Option<Overdue> {
    let id = Uuid::parse_str(&deployment.id.0).ok()?;
    let overdue = |report, absence_to_confirm_in| Overdue {
        deployment_id: deployment.id.clone(),
        report,
        absence_to_confirm_in,
    };

    match (deployment.transition?, instance) {
        (Transition::Departing, None) => Some(overdue(
            DeploymentOutcomeReport::observed(id, "deleted", None),
            Some(deployment.namespace.clone()?),
        )),
        (Transition::Arriving, Some(InstanceState::Serving { version })) => Some(overdue(
            DeploymentOutcomeReport::observed(id, "running", Some(version.clone())),
            None,
        )),
        (Transition::Arriving, Some(InstanceState::Failed)) => Some(overdue(
            DeploymentOutcomeReport::observed(id, "failed", None),
            None,
        )),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct ReconciliationTurns {
    last_attempt: HashMap<DeploymentId, DateTime<Utc>>,
}

impl ReconciliationTurns {
    pub fn next(
        &mut self,
        candidates: &[&Deployment],
        mut overdue: Vec<Overdue>,
        now: DateTime<Utc>,
    ) -> Vec<Overdue> {
        let still_candidates: HashSet<&DeploymentId> =
            candidates.iter().map(|deployment| &deployment.id).collect();
        self.last_attempt
            .retain(|id, _| still_candidates.contains(id));

        overdue.sort_by_key(|item| self.last_attempt.get(&item.deployment_id).copied());
        overdue.truncate(RECONCILED_PER_CYCLE);
        for item in &overdue {
            self.last_attempt.insert(item.deployment_id.clone(), now);
        }
        overdue
    }

    #[cfg(test)]
    fn remembered(&self) -> usize {
        self.last_attempt.len()
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
            .into_iter()
            .map(|overdue| overdue.report)
            .collect()
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
    fn a_deployment_with_work_claimed_this_cycle_is_not_a_candidate() {
        let departing = deployment(Uuid::new_v4(), Some(Transition::Departing));
        let busy = HashSet::from([departing.id.clone()]);

        assert!(awaiting_reconciliation(&[departing], &busy, now()).is_empty());
    }

    #[test]
    fn a_deletion_is_only_overdue_where_absence_can_be_confirmed() {
        let departing = deployment(Uuid::new_v4(), Some(Transition::Departing));
        let mut nowhere = deployment(Uuid::new_v4(), Some(Transition::Departing));
        nowhere.namespace = None;
        let candidates = [&departing, &nowhere];

        let overdue = overdue_outcomes(&candidates, &ObservedInstances::new());

        assert_eq!(overdue.len(), 1);
        assert_eq!(overdue[0].deployment_id, departing.id);
        assert_eq!(
            overdue[0].absence_to_confirm_in.as_deref(),
            Some("autharie-acme-prod")
        );
    }

    #[test]
    fn only_one_cycle_in_eight_reconciles() {
        let reconciling: Vec<u64> = (0..20)
            .filter(|cycle| is_reconciling_cycle(*cycle))
            .collect();

        assert_eq!(reconciling, [0, 8, 16]);
    }

    fn twenty_overdue() -> (Vec<Deployment>, ObservedInstances) {
        let deployments = (0..20)
            .map(|_| deployment(Uuid::new_v4(), Some(Transition::Departing)))
            .collect();
        (deployments, ObservedInstances::new())
    }

    #[test]
    fn one_cycle_takes_a_bounded_batch() {
        let (deployments, observed) = twenty_overdue();
        let candidates = awaiting_reconciliation(&deployments, &HashSet::new(), now());
        let mut turns = ReconciliationTurns::default();

        let taken = turns.next(&candidates, overdue_outcomes(&candidates, &observed), now());

        assert_eq!(taken.len(), RECONCILED_PER_CYCLE);
    }

    #[test]
    fn deployments_left_out_go_first_on_the_next_turn() {
        let (deployments, observed) = twenty_overdue();
        let candidates = awaiting_reconciliation(&deployments, &HashSet::new(), now());
        let mut turns = ReconciliationTurns::default();

        let first: HashSet<DeploymentId> = turns
            .next(&candidates, overdue_outcomes(&candidates, &observed), now())
            .into_iter()
            .map(|item| item.deployment_id)
            .collect();
        let second: Vec<DeploymentId> = turns
            .next(
                &candidates,
                overdue_outcomes(&candidates, &observed),
                now() + Duration::minutes(2),
            )
            .into_iter()
            .map(|item| item.deployment_id)
            .collect();

        let left_out: HashSet<DeploymentId> = deployments
            .iter()
            .map(|deployment| deployment.id.clone())
            .filter(|id| !first.contains(id))
            .collect();
        assert_eq!(left_out.len(), 4);
        assert_eq!(
            second[..4].iter().cloned().collect::<HashSet<_>>(),
            left_out
        );
    }

    #[test]
    fn a_deployment_that_left_the_candidates_is_forgotten() {
        let (deployments, observed) = twenty_overdue();
        let candidates = awaiting_reconciliation(&deployments, &HashSet::new(), now());
        let mut turns = ReconciliationTurns::default();
        turns.next(&candidates, overdue_outcomes(&candidates, &observed), now());

        let remaining = &candidates[..3];
        turns.next(remaining, overdue_outcomes(remaining, &observed), now());

        assert_eq!(turns.remembered(), 3);
    }
}
