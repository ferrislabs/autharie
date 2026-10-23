use std::collections::HashMap;
use std::time::{Duration, Instant};

use autharie_crds::v1alpha::identity_instance::{
    AgentPolicyPhase, AgentsPhase, AgentsStatus, IdentityInstanceStatus,
};
use uuid::Uuid;

use crate::domain::entities::agents_outcome::{
    AgentClientReport, AgentCredentialReport, AgentPolicyOutcome, AgentsOutcomeReport,
};

pub const REPORT_AGAIN_AFTER: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Observation {
    status: Option<AgentsStatus>,
}

impl Observation {
    pub fn of(status: Option<&IdentityInstanceStatus>) -> Self {
        Self {
            status: status.and_then(|status| status.agents.clone()),
        }
    }

    fn is_empty(&self) -> bool {
        self.status.is_none()
    }
}

struct Published {
    observation: Observation,
    at: Instant,
}

#[derive(Default)]
pub struct AgentsTracker {
    published: HashMap<Uuid, Published>,
}

pub fn is_due(previous: Option<(&Observation, Duration)>, observation: &Observation) -> bool {
    match previous {
        None => !observation.is_empty(),
        Some((last, _)) if last != observation => true,
        Some((_, since)) => !observation.is_empty() && since >= REPORT_AGAIN_AFTER,
    }
}

impl AgentsTracker {
    pub fn is_due(&self, deployment_id: Uuid, observation: &Observation) -> bool {
        let previous = self
            .published
            .get(&deployment_id)
            .map(|published| (&published.observation, published.at.elapsed()));

        is_due(previous, observation)
    }

    pub fn record(&mut self, deployment_id: Uuid, observation: Observation) {
        self.published.insert(
            deployment_id,
            Published {
                observation,
                at: Instant::now(),
            },
        );
    }
}

pub fn build_report(deployment_id: Uuid, observation: &Observation) -> Option<AgentsOutcomeReport> {
    let status = observation.status.as_ref()?;

    let policies = status
        .policies
        .iter()
        .map(|policy| AgentPolicyOutcome {
            id: policy.id.clone(),
            phase: match policy.phase {
                AgentPolicyPhase::Applied => "Applied",
                AgentPolicyPhase::Invalid => "Invalid",
                AgentPolicyPhase::Failed => "Failed",
            }
            .to_string(),
            message: policy.message.clone(),
        })
        .collect();

    let clients = status
        .clients
        .iter()
        .map(|client| AgentClientReport {
            client_id: client.client_id.clone(),
            name: client.name.clone(),
            source: client.source.clone(),
            kind: client.kind.clone(),
            enabled: client.enabled,
            created_at: client.created_at.clone(),
        })
        .collect();
    let credentials = status
        .credentials
        .iter()
        .map(|credential| AgentCredentialReport {
            request_id: credential.request_id.clone(),
            client_id: credential.client_id.clone(),
            sealed: credential.sealed.clone(),
        })
        .collect();

    Some(
        AgentsOutcomeReport::new(
            deployment_id,
            status.observed_revision,
            match status.phase {
                AgentsPhase::Applying => "Applying",
                AgentsPhase::Ready => "Ready",
                AgentsPhase::Failed => "Failed",
            },
            status.gateway_client_ready,
            status.mcp_url.clone(),
            status.message.clone(),
            policies,
        )
        .with_registry(clients, status.clients_truncated, credentials),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use autharie_crds::v1alpha::identity_instance::{
        AgentClientStatus, AgentCredentialStatus, AgentPolicyStatus,
    };

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);

    fn status(phase: AgentsPhase, ready: bool) -> AgentsStatus {
        AgentsStatus {
            phase,
            gateway_client_ready: ready,
            observed_revision: 3,
            mcp_url: Some("https://auth.example.com/mcp".to_string()),
            message: None,
            policies: vec![
                AgentPolicyStatus {
                    id: "p1".to_string(),
                    phase: AgentPolicyPhase::Applied,
                    message: None,
                },
                AgentPolicyStatus {
                    id: "p2".to_string(),
                    phase: AgentPolicyPhase::Invalid,
                    message: Some("unexpected token".to_string()),
                },
            ],
            ..AgentsStatus::default()
        }
    }

    fn observed(status: AgentsStatus) -> Observation {
        Observation::of(Some(&IdentityInstanceStatus {
            agents: Some(status),
            ..Default::default()
        }))
    }

    #[test]
    fn nothing_is_reported_before_the_operator_has_written_something() {
        let empty = Observation::of(None);

        assert!(!is_due(None, &empty));
        assert!(build_report(DEPLOYMENT, &empty).is_none());
        assert!(!is_due(
            None,
            &Observation::of(Some(&IdentityInstanceStatus::default()))
        ));
    }

    #[test]
    fn a_first_observation_is_due() {
        assert!(is_due(
            None,
            &observed(status(AgentsPhase::Applying, false))
        ));
    }

    #[test]
    fn a_change_is_due_at_once() {
        let before = observed(status(AgentsPhase::Applying, false));
        let after = observed(status(AgentsPhase::Ready, true));

        assert!(is_due(Some((&before, Duration::ZERO)), &after));
    }

    #[test]
    fn an_unchanged_observation_waits_to_be_reported_again() {
        let same = observed(status(AgentsPhase::Ready, true));

        assert!(!is_due(
            Some((&same, REPORT_AGAIN_AFTER - Duration::from_secs(1))),
            &same
        ));
        assert!(is_due(Some((&same, REPORT_AGAIN_AFTER)), &same));
    }

    #[test]
    fn the_tracker_remembers_what_it_published() {
        let mut tracker = AgentsTracker::default();
        let observation = observed(status(AgentsPhase::Ready, true));

        assert!(tracker.is_due(DEPLOYMENT, &observation));
        tracker.record(DEPLOYMENT, observation.clone());

        assert!(!tracker.is_due(DEPLOYMENT, &observation));
        assert!(tracker.is_due(DEPLOYMENT, &observed(status(AgentsPhase::Failed, true))));
        assert!(tracker.is_due(Uuid::from_u128(2), &observation));
    }

    #[test]
    fn the_report_follows_the_status_and_its_observed_revision() {
        let report = build_report(DEPLOYMENT, &observed(status(AgentsPhase::Ready, true)))
            .expect("a report");

        assert_eq!(report.deployment_id, DEPLOYMENT);
        assert_eq!(report.revision, 3);
        assert_eq!(report.phase, "Ready");
        assert!(report.gateway_client_ready);
        assert_eq!(
            report.mcp_url.as_deref(),
            Some("https://auth.example.com/mcp")
        );
        assert_eq!(report.policies[0].phase, "Applied");
        assert_eq!(report.policies[1].phase, "Invalid");
        assert_eq!(
            report.policies[1].message.as_deref(),
            Some("unexpected token")
        );
    }

    #[test]
    fn the_registry_and_the_sealed_credentials_ride_with_the_report() {
        let mut with_registry = status(AgentsPhase::Ready, true);
        with_registry.clients = vec![AgentClientStatus {
            client_id: "dcr".to_string(),
            name: Some("DCR".to_string()),
            source: "dynamic".to_string(),
            kind: "delegated".to_string(),
            enabled: true,
            created_at: None,
        }];
        with_registry.clients_truncated = true;
        with_registry.credentials = vec![AgentCredentialStatus {
            request_id: "r1".to_string(),
            client_id: "agent-x-abc123".to_string(),
            sealed: "c2VhbGVk".to_string(),
        }];

        let report = build_report(DEPLOYMENT, &observed(with_registry)).expect("report");

        assert_eq!(report.clients[0].client_id, "dcr");
        assert_eq!(report.clients[0].name.as_deref(), Some("DCR"));
        assert!(report.clients_truncated);
        assert_eq!(report.credentials[0].sealed, "c2VhbGVk");
    }

    #[test]
    fn a_new_client_is_a_change_that_is_due_at_once() {
        let before = observed(status(AgentsPhase::Ready, true));
        let mut grown = status(AgentsPhase::Ready, true);
        grown.clients = vec![AgentClientStatus {
            client_id: "dcr".to_string(),
            name: None,
            source: "dynamic".to_string(),
            kind: "delegated".to_string(),
            enabled: true,
            created_at: None,
        }];

        assert!(is_due(Some((&before, Duration::ZERO)), &observed(grown)));
    }

    #[test]
    fn every_phase_has_its_name() {
        for (phase, name) in [
            (AgentsPhase::Applying, "Applying"),
            (AgentsPhase::Ready, "Ready"),
            (AgentsPhase::Failed, "Failed"),
        ] {
            let report = build_report(DEPLOYMENT, &observed(status(phase, false))).expect("report");

            assert_eq!(report.phase, name);
        }
    }
}
