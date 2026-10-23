use autharie_crds::v1alpha::identity_instance::{
    PortalThemePhase, PortalThemeSpec, PortalThemeStatus,
};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step<'a> {
    Keep(&'a PortalThemeStatus),
    Apply(&'a Value),
}

pub fn decide<'a>(entry: &'a PortalThemeSpec, before: Option<&'a PortalThemeStatus>) -> Step<'a> {
    match before {
        Some(status) if !due(entry, status) => Step::Keep(status),
        _ => Step::Apply(&entry.config),
    }
}

fn due(entry: &PortalThemeSpec, status: &PortalThemeStatus) -> bool {
    status.revision < entry.revision
        || (status.revision == entry.revision && status.phase == PortalThemePhase::Pending)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn entry(revision: u64) -> PortalThemeSpec {
        PortalThemeSpec {
            realm: "acme".to_string(),
            revision,
            config: json!({ "colors": { "primaryButton": "#112233" } }),
        }
    }

    fn reported(revision: u64, phase: PortalThemePhase) -> PortalThemeStatus {
        PortalThemeStatus {
            realm: "acme".to_string(),
            revision,
            phase,
            message: None,
        }
    }

    #[test]
    fn an_entry_never_reported_is_applied_with_its_config_verbatim() {
        let spec = entry(1);

        assert_eq!(decide(&spec, None), Step::Apply(&spec.config));
    }

    #[test]
    fn a_newer_revision_is_applied_whatever_the_last_phase() {
        let spec = entry(4);
        for phase in [
            PortalThemePhase::Applied,
            PortalThemePhase::Failed,
            PortalThemePhase::Pending,
        ] {
            let before = reported(3, phase);
            assert!(
                matches!(decide(&spec, Some(&before)), Step::Apply(_)),
                "{phase:?}"
            );
        }
    }

    #[test]
    fn an_applied_revision_is_left_alone() {
        let spec = entry(4);
        let before = reported(4, PortalThemePhase::Applied);

        assert_eq!(decide(&spec, Some(&before)), Step::Keep(&before));
    }

    #[test]
    fn a_pending_revision_is_tried_again() {
        let spec = entry(4);
        let before = reported(4, PortalThemePhase::Pending);

        assert_eq!(decide(&spec, Some(&before)), Step::Apply(&spec.config));
    }

    #[test]
    fn a_failed_revision_is_not_tried_again_until_the_revision_moves() {
        let spec = entry(4);
        let before = reported(4, PortalThemePhase::Failed);

        assert_eq!(decide(&spec, Some(&before)), Step::Keep(&before));
        assert!(matches!(decide(&entry(5), Some(&before)), Step::Apply(_)));
    }

    #[test]
    fn a_report_ahead_of_the_spec_stands() {
        let spec = entry(2);

        for phase in [
            PortalThemePhase::Applied,
            PortalThemePhase::Failed,
            PortalThemePhase::Pending,
        ] {
            let before = reported(7, phase);
            assert_eq!(decide(&spec, Some(&before)), Step::Keep(&before));
        }
    }
}
