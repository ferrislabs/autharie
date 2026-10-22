use std::collections::BTreeSet;

use autharie_crds::v1alpha::identity_instance::AgentPolicySpec;
use uuid::Uuid;

use super::{ManagedPolicy, RemotePolicy, remote_description};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change<'a> {
    Create,
    Update { ferriskey_id: &'a str },
    Keep { ferriskey_id: &'a str },
    Blocked(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan<'a> {
    pub changes: Vec<Change<'a>>,
    pub deletes: Vec<&'a ManagedPolicy>,
}

pub fn plan<'a>(desired: &'a [AgentPolicySpec], remote: &'a [ManagedPolicy]) -> Plan<'a> {
    let mut changes = Vec::with_capacity(desired.len());
    let mut seen = BTreeSet::new();
    let mut kept: BTreeSet<&str> = BTreeSet::new();

    for spec in desired {
        if Uuid::parse_str(&spec.id).is_err() {
            changes.push(Change::Blocked(
                "the policy id is not a UUID, so it cannot be tracked in FerrisKey".to_string(),
            ));
            continue;
        }
        if !seen.insert(spec.id.as_str()) {
            changes.push(Change::Blocked(
                "the policy is listed twice in the desired set".to_string(),
            ));
            continue;
        }

        let mut here: Vec<&ManagedPolicy> = remote
            .iter()
            .filter(|found| found.autharie_id == spec.id)
            .collect();
        here.sort_by(|a, b| a.policy.ferriskey_id.cmp(&b.policy.ferriskey_id));
        let current = here.first().copied();
        if let Some(current) = current {
            kept.insert(current.policy.ferriskey_id.as_str());
        }

        changes.push(match current {
            None => Change::Create,
            Some(current) if differs(spec, &current.policy) => Change::Update {
                ferriskey_id: current.policy.ferriskey_id.as_str(),
            },
            Some(current) => Change::Keep {
                ferriskey_id: current.policy.ferriskey_id.as_str(),
            },
        });
    }

    let deletes = remote
        .iter()
        .filter(|found| !kept.contains(found.policy.ferriskey_id.as_str()))
        .collect();

    Plan { changes, deletes }
}

fn differs(spec: &AgentPolicySpec, remote: &RemotePolicy) -> bool {
    remote.source.trim() != spec.cedar.trim()
        || !remote.enabled
        || remote.description.as_deref() != Some(remote_description(spec).as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID_A: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";
    const ID_B: &str = "7f1c2d3e-0000-4000-8000-00000000000b";
    const CEDAR: &str = "permit (principal, action, resource);";

    fn spec(id: &str) -> AgentPolicySpec {
        AgentPolicySpec {
            id: id.to_string(),
            name: "Read only".to_string(),
            cedar: CEDAR.to_string(),
        }
    }

    fn remote(id: &str, ferriskey_id: &str) -> ManagedPolicy {
        ManagedPolicy {
            autharie_id: id.to_string(),
            policy: RemotePolicy {
                ferriskey_id: ferriskey_id.to_string(),
                name: format!("autharie:{id}"),
                description: Some("Read only".to_string()),
                source: CEDAR.to_string(),
                enabled: true,
            },
        }
    }

    #[test]
    fn a_policy_nobody_wrote_yet_is_created() {
        let desired = [spec(ID_A)];

        let plan = plan(&desired, &[]);

        assert_eq!(plan.changes, vec![Change::Create]);
        assert!(plan.deletes.is_empty());
    }

    #[test]
    fn a_policy_that_matches_is_kept() {
        let desired = [spec(ID_A)];
        let found = [remote(ID_A, "f1")];

        let plan = plan(&desired, &found);

        assert_eq!(plan.changes, vec![Change::Keep { ferriskey_id: "f1" }]);
        assert!(plan.deletes.is_empty());
    }

    #[test]
    fn a_changed_source_is_updated_in_place() {
        let mut changed = spec(ID_A);
        changed.cedar = "forbid (principal, action, resource);".to_string();
        let desired = [changed];
        let found = [remote(ID_A, "f1")];

        let plan = plan(&desired, &found);

        assert_eq!(plan.changes, vec![Change::Update { ferriskey_id: "f1" }]);
    }

    #[test]
    fn surrounding_whitespace_is_not_a_change() {
        let mut padded = spec(ID_A);
        padded.cedar = format!("\n{CEDAR}\n");
        let desired = [padded];
        let found = [remote(ID_A, "f1")];

        assert_eq!(
            plan(&desired, &found).changes,
            vec![Change::Keep { ferriskey_id: "f1" }]
        );
    }

    #[test]
    fn a_renamed_policy_is_updated() {
        let mut renamed = spec(ID_A);
        renamed.name = "Support read only".to_string();
        let desired = [renamed];
        let found = [remote(ID_A, "f1")];

        assert_eq!(
            plan(&desired, &found).changes,
            vec![Change::Update { ferriskey_id: "f1" }]
        );
    }

    #[test]
    fn a_policy_somebody_disabled_is_updated_back() {
        let desired = [spec(ID_A)];
        let mut found = [remote(ID_A, "f1")];
        found[0].policy.enabled = false;

        assert_eq!(
            plan(&desired, &found).changes,
            vec![Change::Update { ferriskey_id: "f1" }]
        );
    }

    #[test]
    fn a_managed_policy_no_longer_wanted_is_deleted() {
        let desired = [spec(ID_A)];
        let found = [remote(ID_A, "f1"), remote(ID_B, "f2")];

        let plan = plan(&desired, &found);

        assert_eq!(plan.deletes.len(), 1);
        assert_eq!(plan.deletes[0].policy.ferriskey_id, "f2");
    }

    #[test]
    fn nothing_desired_deletes_every_managed_policy() {
        let found = [remote(ID_A, "f1"), remote(ID_B, "f2")];

        let plan = plan(&[], &found);

        assert!(plan.changes.is_empty());
        assert_eq!(plan.deletes.len(), 2);
    }

    #[test]
    fn a_duplicate_in_ferriskey_keeps_the_first_and_deletes_the_rest() {
        let desired = [spec(ID_A)];
        let found = [remote(ID_A, "f2"), remote(ID_A, "f1")];

        let plan = plan(&desired, &found);

        assert_eq!(plan.changes, vec![Change::Keep { ferriskey_id: "f1" }]);
        assert_eq!(plan.deletes.len(), 1);
        assert_eq!(plan.deletes[0].policy.ferriskey_id, "f2");
    }

    #[test]
    fn a_policy_listed_twice_is_blocked_the_second_time() {
        let desired = [spec(ID_A), spec(ID_A)];

        let plan = plan(&desired, &[]);

        assert_eq!(plan.changes[0], Change::Create);
        assert!(matches!(plan.changes[1], Change::Blocked(_)));
    }

    #[test]
    fn an_id_that_is_not_a_uuid_is_blocked_before_anything_is_written() {
        let desired = [spec("not-a-uuid")];

        let plan = plan(&desired, &[]);

        assert!(matches!(plan.changes[0], Change::Blocked(_)));
    }

    #[test]
    fn changes_follow_the_order_of_the_desired_policies() {
        let desired = [spec(ID_B), spec(ID_A)];
        let found = [remote(ID_A, "f1")];

        let plan = plan(&desired, &found);

        assert_eq!(
            plan.changes,
            vec![Change::Create, Change::Keep { ferriskey_id: "f1" }]
        );
    }
}
