use std::collections::BTreeSet;

use autharie_crds::v1alpha::identity_instance::WebhookSpec;

use super::{ManagedRemote, RemoteWebhook, remote_description};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredWebhook<'a> {
    pub spec: &'a WebhookSpec,
    /// Why nothing can be done for it right now (a realm that cannot be
    /// listed, a header value that is not there). A blocked webhook is neither
    /// created nor changed, so a guess never reaches FerrisKey.
    pub blocked: Option<String>,
    /// Its header values differ from the ones last sent, or none were sent.
    pub headers_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change<'a> {
    Create,
    Update {
        ferriskey_id: &'a str,
        send_headers: bool,
    },
    Keep {
        ferriskey_id: &'a str,
    },
    Blocked(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan<'a> {
    /// One per desired webhook, in the same order.
    pub changes: Vec<Change<'a>>,
    pub deletes: Vec<&'a ManagedRemote>,
}

/// What it takes to make FerrisKey's managed webhooks match the spec.
///
/// `remote` holds managed webhooks only; the caller filters, and nothing here
/// can name anything else, which is what keeps a customer's own webhooks safe.
pub fn plan<'a>(desired: &[DesiredWebhook<'a>], remote: &'a [ManagedRemote]) -> Plan<'a> {
    let mut deletes: Vec<&ManagedRemote> = Vec::new();
    let mut changes = Vec::with_capacity(desired.len());
    let mut seen = BTreeSet::new();
    let mut kept: BTreeSet<&str> = BTreeSet::new();

    for webhook in desired {
        let spec = webhook.spec;
        if !seen.insert(spec.id.as_str()) {
            changes.push(Change::Blocked(
                "the webhook is listed twice in the desired set".to_string(),
            ));
            continue;
        }

        let mut here: Vec<&ManagedRemote> = remote
            .iter()
            .filter(|found| found.autharie_id == spec.id && found.realm == spec.realm)
            .collect();
        here.sort_by(|a, b| a.webhook.ferriskey_id.cmp(&b.webhook.ferriskey_id));
        let current = here.first().copied();
        if let Some(current) = current {
            kept.insert(current.webhook.ferriskey_id.as_str());
        }

        if let Some(reason) = &webhook.blocked {
            changes.push(Change::Blocked(reason.clone()));
            continue;
        }

        changes.push(match current {
            None => Change::Create,
            Some(current) if differs(spec, &current.webhook) || webhook.headers_changed => {
                Change::Update {
                    ferriskey_id: current.webhook.ferriskey_id.as_str(),
                    send_headers: webhook.headers_changed,
                }
            }
            Some(current) => Change::Keep {
                ferriskey_id: current.webhook.ferriskey_id.as_str(),
            },
        });
    }

    for found in remote {
        if !kept.contains(found.webhook.ferriskey_id.as_str()) {
            deletes.push(found);
        }
    }

    Plan { changes, deletes }
}

fn differs(spec: &WebhookSpec, remote: &RemoteWebhook) -> bool {
    let wanted: BTreeSet<&str> = spec.events.iter().map(String::as_str).collect();
    let present: BTreeSet<&str> = remote.events.iter().map(String::as_str).collect();

    remote.endpoint != spec.endpoint
        || wanted != present
        || remote.description.as_deref() != Some(remote_description(spec).as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID_A: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";
    const ID_B: &str = "7f1c2d3e-0000-4000-8000-00000000000b";

    fn spec(id: &str, realm: &str) -> WebhookSpec {
        WebhookSpec {
            id: id.to_string(),
            realm: realm.to_string(),
            name: "Audit".to_string(),
            description: Some("to the SIEM".to_string()),
            endpoint: "https://example.com/hook".to_string(),
            events: vec!["user.created".to_string(), "user.deleted".to_string()],
            header_names: Vec::new(),
        }
    }

    fn remote(id: &str, realm: &str, ferriskey_id: &str) -> ManagedRemote {
        ManagedRemote {
            realm: realm.to_string(),
            autharie_id: id.to_string(),
            webhook: RemoteWebhook {
                ferriskey_id: ferriskey_id.to_string(),
                name: Some(format!("autharie:{id}")),
                description: Some("Audit — to the SIEM".to_string()),
                endpoint: "https://example.com/hook".to_string(),
                events: vec!["user.deleted".to_string(), "user.created".to_string()],
            },
        }
    }

    fn desired(spec: &WebhookSpec) -> DesiredWebhook<'_> {
        DesiredWebhook {
            spec,
            blocked: None,
            headers_changed: false,
        }
    }

    #[test]
    fn a_webhook_nobody_created_yet_is_created() {
        let a = spec(ID_A, "master");

        let plan = plan(&[desired(&a)], &[]);

        assert_eq!(plan.changes, vec![Change::Create]);
        assert!(plan.deletes.is_empty());
    }

    #[test]
    fn a_webhook_that_matches_is_kept_whatever_the_order_of_its_events() {
        let a = spec(ID_A, "master");
        let found = [remote(ID_A, "master", "f1")];

        let plan = plan(&[desired(&a)], &found);

        assert_eq!(plan.changes, vec![Change::Keep { ferriskey_id: "f1" }]);
        assert!(plan.deletes.is_empty());
    }

    #[test]
    fn each_field_that_differs_triggers_an_update_without_headers() {
        let found = [remote(ID_A, "master", "f1")];
        let mut changed = Vec::new();

        let mut endpoint = spec(ID_A, "master");
        endpoint.endpoint = "https://example.com/other".to_string();
        changed.push(endpoint);
        let mut events = spec(ID_A, "master");
        events.events.push("user.updated".to_string());
        changed.push(events);
        let mut fewer = spec(ID_A, "master");
        fewer.events.pop();
        changed.push(fewer);
        let mut name = spec(ID_A, "master");
        name.name = "Renamed".to_string();
        changed.push(name);
        let mut description = spec(ID_A, "master");
        description.description = None;
        changed.push(description);

        for spec in &changed {
            let plan = plan(&[desired(spec)], &found);
            assert_eq!(
                plan.changes,
                vec![Change::Update {
                    ferriskey_id: "f1",
                    send_headers: false
                }],
                "{spec:?}"
            );
        }
    }

    #[test]
    fn changed_header_values_alone_trigger_an_update_that_sends_them() {
        let a = spec(ID_A, "master");
        let found = [remote(ID_A, "master", "f1")];
        let mut with_headers = desired(&a);
        with_headers.headers_changed = true;

        let plan = plan(&[with_headers], &found);

        assert_eq!(
            plan.changes,
            vec![Change::Update {
                ferriskey_id: "f1",
                send_headers: true
            }]
        );
    }

    #[test]
    fn a_managed_webhook_the_spec_no_longer_names_is_deleted() {
        let a = spec(ID_A, "master");
        let found = [remote(ID_A, "master", "f1"), remote(ID_B, "master", "f2")];

        let plan = plan(&[desired(&a)], &found);

        assert_eq!(plan.deletes, vec![&found[1]]);
    }

    #[test]
    fn an_empty_spec_deletes_every_managed_webhook() {
        let found = [remote(ID_A, "master", "f1"), remote(ID_B, "acme", "f2")];

        let plan = plan(&[], &found);

        assert_eq!(plan.deletes.len(), 2);
        assert!(plan.changes.is_empty());
    }

    #[test]
    fn a_moved_webhook_is_created_in_the_new_realm_and_deleted_from_the_old() {
        let a = spec(ID_A, "acme");
        let found = [remote(ID_A, "master", "f1")];

        let plan = plan(&[desired(&a)], &found);

        assert_eq!(plan.changes, vec![Change::Create]);
        assert_eq!(plan.deletes, vec![&found[0]]);
    }

    #[test]
    fn a_duplicate_in_the_same_realm_keeps_one_and_deletes_the_rest() {
        let a = spec(ID_A, "master");
        let found = [remote(ID_A, "master", "f9"), remote(ID_A, "master", "f1")];

        let plan = plan(&[desired(&a)], &found);

        assert_eq!(plan.changes, vec![Change::Keep { ferriskey_id: "f1" }]);
        assert_eq!(plan.deletes, vec![&found[0]]);
    }

    #[test]
    fn a_blocked_webhook_is_left_as_it_is_in_its_realm() {
        let a = spec(ID_A, "master");
        let found = [remote(ID_A, "master", "f1")];
        let mut blocked = desired(&a);
        blocked.blocked = Some("missing header".to_string());
        blocked.headers_changed = true;

        let plan = plan(&[blocked], &found);

        assert_eq!(plan.changes, vec![Change::Blocked("missing header".into())]);
        assert!(plan.deletes.is_empty());
    }

    #[test]
    fn a_blocked_webhook_with_nothing_in_its_realm_is_not_created() {
        let a = spec(ID_A, "master");
        let mut blocked = desired(&a);
        blocked.blocked = Some("realm unreachable".to_string());

        let plan = plan(&[blocked], &[]);

        assert_eq!(
            plan.changes,
            vec![Change::Blocked("realm unreachable".into())]
        );
    }

    #[test]
    fn an_id_listed_twice_is_acted_on_once() {
        let first = spec(ID_A, "master");
        let second = spec(ID_A, "master");

        let plan = plan(&[desired(&first), desired(&second)], &[]);

        assert_eq!(plan.changes[0], Change::Create);
        assert!(matches!(plan.changes[1], Change::Blocked(_)));
    }

    #[test]
    fn changes_follow_the_order_of_the_desired_set() {
        let a = spec(ID_A, "master");
        let b = spec(ID_B, "master");
        let found = [remote(ID_B, "master", "f2")];

        let plan = plan(&[desired(&a), desired(&b)], &found);

        assert_eq!(
            plan.changes,
            vec![Change::Create, Change::Keep { ferriskey_id: "f2" }]
        );
    }
}
