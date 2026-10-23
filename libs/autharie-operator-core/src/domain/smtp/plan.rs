use autharie_crds::v1alpha::identity_instance::{SmtpPhase, SmtpSpec, SmtpStatus};

use super::SmtpWrite;

/// The two places a realm's password can come from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Passwords<'a> {
    pub stored: Option<&'a str>,
    pub applied: Option<&'a str>,
}

impl<'a> Passwords<'a> {
    /// What Genesis stored wins; what was applied before stands in when it
    /// stored none. An empty value is no value.
    pub fn pick(&self) -> Option<&'a str> {
        self.stored
            .filter(|password| !password.is_empty())
            .or_else(|| self.applied.filter(|password| !password.is_empty()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step<'a> {
    /// Nothing to do: what was reported stands.
    Keep(&'a SmtpStatus),
    Remove,
    Apply(SmtpWrite<'a>),
    Fail(String),
}

/// An entry is worked on when it has not been reported, when it is newer than
/// the report, or when the last attempt hit something temporary. A failure
/// stays reported until the revision moves: the same settings fail the same
/// way.
pub fn decide<'a>(
    entry: &'a SmtpSpec,
    before: Option<&'a SmtpStatus>,
    passwords: &Passwords<'a>,
) -> Step<'a> {
    if let Some(status) = before
        && !due(entry, status)
    {
        return Step::Keep(status);
    }
    if entry.remove {
        return Step::Remove;
    }

    let (
        Some(host),
        Some(port),
        Some(encryption),
        Some(username),
        Some(from_email),
        Some(from_name),
    ) = (
        entry.host.as_deref(),
        entry.port,
        entry.encryption,
        entry.username.as_deref(),
        entry.from_email.as_deref(),
        entry.from_name.as_deref(),
    )
    else {
        return Step::Fail(format!("the settings lack {}", missing(entry).join(", ")));
    };
    let Some(password) = passwords.pick() else {
        return Step::Fail("no password".to_string());
    };
    Step::Apply(SmtpWrite {
        host,
        port,
        encryption,
        username,
        password,
        from_email,
        from_name,
    })
}

fn missing(entry: &SmtpSpec) -> Vec<&'static str> {
    [
        ("host", entry.host.is_none()),
        ("port", entry.port.is_none()),
        ("encryption", entry.encryption.is_none()),
        ("username", entry.username.is_none()),
        ("fromEmail", entry.from_email.is_none()),
        ("fromName", entry.from_name.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, absent)| absent.then_some(name))
    .collect()
}

fn due(entry: &SmtpSpec, status: &SmtpStatus) -> bool {
    status.revision < entry.revision
        || (status.revision == entry.revision && status.phase == SmtpPhase::Pending)
}

#[cfg(test)]
mod tests {
    use autharie_crds::v1alpha::identity_instance::SmtpEncryption;

    use super::*;

    fn entry(revision: u64) -> SmtpSpec {
        SmtpSpec {
            realm: "acme".to_string(),
            revision,
            remove: false,
            host: Some("smtp.example.com".to_string()),
            port: Some(587),
            encryption: Some(SmtpEncryption::Starttls),
            username: Some("mailer".to_string()),
            from_email: Some("no-reply@example.com".to_string()),
            from_name: Some("Acme".to_string()),
        }
    }

    fn tombstone(revision: u64) -> SmtpSpec {
        SmtpSpec {
            realm: "acme".to_string(),
            revision,
            remove: true,
            host: None,
            port: None,
            encryption: None,
            username: None,
            from_email: None,
            from_name: None,
        }
    }

    fn reported(revision: u64, phase: SmtpPhase) -> SmtpStatus {
        SmtpStatus {
            realm: "acme".to_string(),
            revision,
            phase,
            message: None,
        }
    }

    fn stored(password: &str) -> Passwords<'_> {
        Passwords {
            stored: Some(password),
            applied: None,
        }
    }

    #[test]
    fn an_entry_never_reported_is_applied() {
        let spec = entry(1);

        let Step::Apply(write) = decide(&spec, None, &stored("pw")) else {
            panic!("expected an apply");
        };

        assert_eq!(write.host, "smtp.example.com");
        assert_eq!(write.port, 587);
        assert_eq!(write.encryption, SmtpEncryption::Starttls);
        assert_eq!(write.username, "mailer");
        assert_eq!(write.password, "pw");
        assert_eq!(write.from_email, "no-reply@example.com");
        assert_eq!(write.from_name, "Acme");
    }

    #[test]
    fn a_newer_revision_is_applied_whatever_the_last_phase() {
        let spec = entry(4);
        for phase in [SmtpPhase::Applied, SmtpPhase::Failed, SmtpPhase::Pending] {
            let before = reported(3, phase);
            assert!(
                matches!(decide(&spec, Some(&before), &stored("pw")), Step::Apply(_)),
                "{phase:?}"
            );
        }
    }

    #[test]
    fn an_applied_revision_is_left_alone() {
        let spec = entry(4);
        let before = reported(4, SmtpPhase::Applied);

        assert_eq!(
            decide(&spec, Some(&before), &stored("pw")),
            Step::Keep(&before)
        );
    }

    #[test]
    fn a_pending_revision_is_tried_again() {
        let spec = entry(4);
        let before = reported(4, SmtpPhase::Pending);

        assert!(matches!(
            decide(&spec, Some(&before), &stored("pw")),
            Step::Apply(_)
        ));
    }

    #[test]
    fn a_failed_revision_is_not_tried_again_until_the_revision_moves() {
        let spec = entry(4);
        let before = reported(4, SmtpPhase::Failed);

        assert_eq!(
            decide(&spec, Some(&before), &stored("pw")),
            Step::Keep(&before)
        );
        assert!(matches!(
            decide(&entry(5), Some(&before), &stored("pw")),
            Step::Apply(_)
        ));
    }

    #[test]
    fn a_report_ahead_of_the_spec_stands() {
        let spec = entry(2);
        let before = reported(7, SmtpPhase::Pending);

        assert_eq!(
            decide(&spec, Some(&before), &stored("pw")),
            Step::Keep(&before)
        );
    }

    #[test]
    fn a_removal_deletes_and_needs_no_password() {
        let spec = tombstone(6);

        assert_eq!(decide(&spec, None, &Passwords::default()), Step::Remove);
        let before = reported(5, SmtpPhase::Applied);
        assert_eq!(
            decide(&spec, Some(&before), &Passwords::default()),
            Step::Remove
        );
    }

    #[test]
    fn a_removal_already_applied_is_not_repeated_and_a_pending_one_is() {
        let spec = tombstone(6);
        let done = reported(6, SmtpPhase::Applied);
        let waiting = reported(6, SmtpPhase::Pending);

        assert_eq!(
            decide(&spec, Some(&done), &Passwords::default()),
            Step::Keep(&done)
        );
        assert_eq!(
            decide(&spec, Some(&waiting), &Passwords::default()),
            Step::Remove
        );
    }

    #[test]
    fn the_stored_password_wins_over_the_applied_one() {
        let spec = entry(2);
        let passwords = Passwords {
            stored: Some("new"),
            applied: Some("old"),
        };

        let Step::Apply(write) = decide(&spec, None, &passwords) else {
            panic!("expected an apply");
        };

        assert_eq!(write.password, "new");
    }

    #[test]
    fn without_a_stored_password_the_applied_one_is_kept() {
        let spec = entry(2);
        let passwords = Passwords {
            stored: None,
            applied: Some("old"),
        };

        let Step::Apply(write) = decide(&spec, None, &passwords) else {
            panic!("expected an apply");
        };

        assert_eq!(write.password, "old");
    }

    #[test]
    fn an_empty_stored_password_counts_as_absent() {
        let spec = entry(2);
        let passwords = Passwords {
            stored: Some(""),
            applied: Some("old"),
        };

        let Step::Apply(write) = decide(&spec, None, &passwords) else {
            panic!("expected an apply");
        };

        assert_eq!(write.password, "old");
        assert_eq!(
            decide(
                &spec,
                None,
                &Passwords {
                    stored: Some(""),
                    applied: Some("")
                }
            ),
            Step::Fail("no password".to_string())
        );
    }

    #[test]
    fn no_password_anywhere_fails_a_save() {
        assert_eq!(
            decide(&entry(1), None, &Passwords::default()),
            Step::Fail("no password".to_string())
        );
    }

    #[test]
    fn a_save_missing_settings_names_them() {
        let mut spec = entry(1);
        spec.host = None;
        spec.from_name = None;

        assert_eq!(
            decide(&spec, None, &stored("pw")),
            Step::Fail("the settings lack host, fromName".to_string())
        );
    }

    #[test]
    fn missing_settings_are_reported_before_a_missing_password() {
        let mut spec = entry(1);
        spec.port = None;

        assert_eq!(
            decide(&spec, None, &Passwords::default()),
            Step::Fail("the settings lack port".to_string())
        );
    }
}
