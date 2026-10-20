use crate::{
    CoreError,
    dataplane::value_objects::{DataPlaneId, DeploymentResources, Region},
    deployments::{environment::Environment, topology::Topology},
    offers::{DatabasePlacement, Offer},
    organisation::OrganisationId,
    user::UserId,
};

use super::{
    DeploymentKind, DeploymentName,
    distribution::{Distribution, DistributionError},
};
use crate::version::Version;

/// Command to create a new deployment
#[derive(Debug, Clone)]
pub struct CreateDeploymentCommand {
    pub organisation_id: OrganisationId,
    pub name: DeploymentName,
    pub kind: DeploymentKind,
    pub version: Version,
    pub created_by: UserId,

    /// Which environment this belongs to. The one thing in the create form
    /// the customer was genuinely answering.
    pub environment: Environment,

    /// Where the caller wants this to run. Carried rather than assumed: a
    /// region silently substituted for another is a deployment in the wrong
    /// jurisdiction. It is also the only infrastructure decision left to the
    /// customer, because it is about where their data lives.
    pub region: Region,

    /// What they bought.
    ///
    /// The size and whether it gets a cluster of its own are read from it. A
    /// command that carried them beside the offer could contradict it, and
    /// there would be no way to tell which half was meant.
    pub offer: Offer,

    /// Present when this deployment exists to hold somebody's data back.
    pub recovery: Option<Recovery>,

    /// Private so the only way to ask for the customer's cloud is
    /// [`CreateDeploymentCommand::with_distribution`], which checks the kind.
    distribution: Distribution,

    topology: Topology,

    sizing: Sizing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sizing {
    Offer,
    Platform(DeploymentResources),
}

/// What a recovery deployment is coming back from.
///
/// The resources are carried rather than read from the offer, and they win.
/// An offer's sizing can be revised, and a recovery sized from today's revision
/// could be smaller than the archive it has to hold -- which is discovered when
/// the disk fills, during the outage the restore was answering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    pub backup: crate::backups::BackupId,
    pub resources: DeploymentResources,
    pub topology: Topology,
}

impl CreateDeploymentCommand {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        organisation_id: OrganisationId,
        name: DeploymentName,
        kind: DeploymentKind,
        version: Version,
        created_by: UserId,
        environment: Environment,
        region: Region,
        offer: Offer,
    ) -> Result<Self, CoreError> {
        if offer.database() == DatabasePlacement::Shared && kind != DeploymentKind::Ferriskey {
            return Err(DistributionError::NotPoolable { kind }.into());
        }

        Ok(Self {
            organisation_id,
            name: name.publishable()?,
            kind,
            version,
            created_by,
            environment,
            region,
            offer,
            recovery: None,
            distribution: Distribution::Shared,
            topology: Topology::SINGLE,
            sizing: Sizing::Offer,
        })
    }

    /// Where it is hosted, refused when this kind cannot be hosted there.
    pub fn with_distribution(
        mut self,
        distribution: Distribution,
    ) -> Result<Self, DistributionError> {
        if self.offer.database() == DatabasePlacement::Shared
            && distribution != Distribution::Shared
        {
            return Err(DistributionError::PooledOfferPlacedElsewhere);
        }
        self.distribution = distribution.checked_for(&self.kind)?;
        Ok(self)
    }

    pub fn distribution(&self) -> &Distribution {
        &self.distribution
    }

    pub fn with_topology(mut self, topology: Topology) -> Self {
        self.topology = topology;
        self
    }

    pub fn topology(&self) -> Topology {
        self.topology
    }

    pub fn sized_by_platform(mut self, resources: DeploymentResources) -> Self {
        self.sizing = Sizing::Platform(resources);
        self
    }

    pub fn sizing(&self) -> Sizing {
        self.sizing
    }

    /// The same deployment, brought back from an archive.
    pub fn recovering(mut self, recovery: Recovery) -> Self {
        self.topology = recovery.topology;
        self.recovery = Some(recovery);
        self
    }
}

/// Command to update an existing deployment
#[derive(Debug, Clone, Default)]
pub struct UpdateDeploymentCommand {
    pub name: Option<DeploymentName>,
    /// The continuous log shipping switch (#294). `None` leaves it as it
    /// stands; there is no console for it yet, so today only the API sets it.
    pub log_shipping_enabled: Option<bool>,
}

impl UpdateDeploymentCommand {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_name(mut self, name: DeploymentName) -> Result<Self, CoreError> {
        self.name = Some(name.publishable()?);
        Ok(self)
    }

    pub fn with_log_shipping_enabled(mut self, log_shipping_enabled: bool) -> Self {
        self.log_shipping_enabled = Some(log_shipping_enabled);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.log_shipping_enabled.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn create_deployment_command_sets_fields() {
        let command = CreateDeploymentCommand::new(
            OrganisationId(Uuid::new_v4()),
            DeploymentName("tenant".to_string()),
            DeploymentKind::Keycloak,
            Version::new(1, 0, 0),
            UserId(Uuid::new_v4()),
            Environment::Production,
            Region::new("fr-par"),
            Offer::Standard,
        )
        .expect("a creatable name");

        assert_eq!(command.name.0, "tenant");
        assert_eq!(command.kind, DeploymentKind::Keycloak);
        assert_eq!(command.version.to_string(), "1.0.0");
        assert_eq!(command.environment, Environment::Production);
        assert_eq!(command.offer, Offer::Standard);
    }

    #[test]
    fn a_recovery_decides_the_topology_of_the_command() {
        let command = CreateDeploymentCommand::new(
            OrganisationId(Uuid::new_v4()),
            DeploymentName("tenant".to_string()),
            DeploymentKind::Ferriskey,
            Version::new(1, 0, 0),
            UserId(Uuid::new_v4()),
            Environment::Production,
            Region::new("fr-par"),
            Offer::Standard,
        )
        .expect("a creatable name");
        assert_eq!(command.topology(), Topology::SINGLE);

        let recovering = command.recovering(Recovery {
            backup: crate::backups::BackupId(Uuid::new_v4()),
            resources: DeploymentResources::DEFAULT,
            topology: Topology::HIGH_AVAILABILITY,
        });

        assert_eq!(recovering.topology(), Topology::HIGH_AVAILABILITY);
    }

    /// There is no field for either, and that is the point: a command
    /// carrying a size beside an offer could contradict it, and nothing would
    /// be able to say which half was meant.
    #[test]
    fn the_size_and_the_isolation_come_from_the_offer() {
        assert_eq!(
            Offer::Standard.resources(),
            crate::dataplane::value_objects::DeploymentResources::DEFAULT,
            "the standard offer stopped matching the platform default"
        );
    }

    fn a_command(kind: DeploymentKind) -> CreateDeploymentCommand {
        CreateDeploymentCommand::new(
            OrganisationId(Uuid::new_v4()),
            DeploymentName("tenant".to_string()),
            kind,
            Version::new(1, 0, 0),
            UserId(Uuid::new_v4()),
            Environment::Production,
            Region::new("fr-par"),
            Offer::Standard,
        )
        .expect("a creatable name")
    }

    fn create_named(name: &str) -> Result<CreateDeploymentCommand, CoreError> {
        CreateDeploymentCommand::new(
            OrganisationId(Uuid::new_v4()),
            DeploymentName(name.to_string()),
            DeploymentKind::Ferriskey,
            Version::new(1, 0, 0),
            UserId(Uuid::new_v4()),
            Environment::Production,
            Region::new("fr-par"),
            Offer::Standard,
        )
    }

    #[test]
    fn every_reserved_label_is_refused_at_creation() {
        for label in crate::dns::RESERVED_HOSTNAME_LABELS {
            assert!(
                matches!(
                    create_named(label),
                    Err(CoreError::DeploymentNameReserved { .. })
                ),
                "{label} was accepted"
            );
        }
    }

    #[test]
    fn a_reserved_label_is_refused_however_the_name_is_written() {
        for name in ["WWW", " Api ", "A.P.I", "--www--", "Console!"] {
            let expected_reserved =
                crate::dns::is_refused_name(&crate::deployments::environment::slug(name));
            assert_eq!(
                matches!(
                    create_named(name),
                    Err(CoreError::DeploymentNameReserved { .. })
                ),
                expected_reserved,
                "{name}"
            );
        }
        assert!(create_named("WWW").is_err());
        assert!(create_named(" Api ").is_err());
        assert!(create_named("--www--").is_err());
        assert!(create_named("Console!").is_err());
    }

    #[test]
    fn the_refusal_says_the_name_is_reserved_and_to_pick_another() {
        let Err(error) = create_named("api") else {
            panic!("api must be refused");
        };
        let message = error.to_string();

        assert!(message.contains("reserved"), "{message}");
        assert!(message.contains("pick another"), "{message}");
    }

    #[test]
    fn demo_is_creatable_and_a_name_starting_like_a_platform_label_is_not() {
        assert!(create_named("demo").is_ok());
        assert!(matches!(
            create_named("api-gateway"),
            Err(CoreError::DeploymentNameReserved { .. })
        ));
    }

    #[test]
    fn a_rename_to_a_reserved_label_is_refused() {
        let result = UpdateDeploymentCommand::new().with_name(DeploymentName("Status".to_string()));

        assert!(matches!(
            result,
            Err(CoreError::DeploymentNameReserved { .. })
        ));
    }

    /// @spec-ccp-9
    #[test]
    fn a_keycloak_command_cannot_ask_for_the_customer_cloud() {
        let customer_cloud = crate::deployments::distribution::tests::customer_cloud();

        let result = a_command(DeploymentKind::Keycloak).with_distribution(customer_cloud);

        assert!(matches!(
            result,
            Err(DistributionError::NotAllowedForKind { .. })
        ));
    }

    #[test]
    fn a_ferriskey_command_can_ask_for_the_customer_cloud() {
        let customer_cloud = crate::deployments::distribution::tests::customer_cloud();

        let command = a_command(DeploymentKind::Ferriskey)
            .with_distribution(customer_cloud.clone())
            .expect("allowed");

        assert_eq!(command.distribution(), &customer_cloud);
    }

    fn a_free_command(kind: DeploymentKind) -> Result<CreateDeploymentCommand, CoreError> {
        CreateDeploymentCommand::new(
            OrganisationId(Uuid::new_v4()),
            DeploymentName("tenant".to_string()),
            kind,
            Version::new(1, 0, 0),
            UserId(Uuid::new_v4()),
            Environment::Production,
            Region::new("fr-par"),
            Offer::Free,
        )
    }

    /// @spec-fpr-13
    #[test]
    fn only_a_ferriskey_command_can_take_the_free_offer() {
        let refused = a_free_command(DeploymentKind::Keycloak).expect_err("keycloak is not pooled");

        assert!(
            matches!(
                refused,
                CoreError::Distribution(DistributionError::NotPoolable { .. })
            ),
            "{refused}"
        );
        assert!(a_free_command(DeploymentKind::Ferriskey).is_ok());
    }

    #[test]
    fn a_free_command_cannot_ask_for_another_distribution() {
        let command = a_free_command(DeploymentKind::Ferriskey).expect("ferriskey");

        assert_eq!(
            command
                .clone()
                .with_distribution(crate::deployments::distribution::tests::customer_cloud())
                .err(),
            Some(DistributionError::PooledOfferPlacedElsewhere)
        );
        assert!(command.with_distribution(Distribution::Shared).is_ok());
    }

    #[test]
    fn a_command_is_shared_by_default() {
        assert_eq!(
            a_command(DeploymentKind::Keycloak).distribution(),
            &Distribution::Shared
        );
    }

    #[test]
    fn update_deployment_command_is_empty_when_no_fields_set() {
        let command = UpdateDeploymentCommand::new();

        assert!(command.is_empty());
    }

    #[test]
    fn update_deployment_command_builder_sets_fields() {
        let command = UpdateDeploymentCommand::new()
            .with_name(DeploymentName("new".to_string()))
            .expect("a publishable name")
            .with_log_shipping_enabled(true);

        assert_eq!(command.name.unwrap().0, "new");
        assert_eq!(command.log_shipping_enabled, Some(true));
    }
}

/// What a data plane observed happening to a deployment it was given.
///
/// Parsed at the boundary rather than carried as a string, so an outcome the
/// control plane does not understand is rejected where the caller can be told
/// about it, instead of reaching a match arm that quietly does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentOutcome {
    /// The resources are gone. Reported by the component that removed them.
    Deleted,
    /// The deployment is serving. Reported on the operator's `Running` +
    /// `ready`, which is the only place that knows.
    Running,
    /// The operator gave up on it. An observation, not a timeout.
    Failed,
}

#[derive(Debug, Clone)]
pub struct ReportDeploymentOutcomeCommand {
    /// The version the data plane observed running, when it said so.
    ///
    /// Optional because the field is new: a Genesis built before it sends
    /// nothing, and a report that cannot be read is a report thrown away. It
    /// is what tells an upgrade that landed from one that has not started.
    pub observed_version: Option<Version>,
    pub dataplane_id: DataPlaneId,
    pub deployment_id: super::DeploymentId,
    pub outcome: DeploymentOutcome,
    pub reason: Option<String>,
}

impl ReportDeploymentOutcomeCommand {
    pub fn parse(
        dataplane_id: DataPlaneId,
        deployment_id: super::DeploymentId,
        outcome: &str,
        observed_version: Option<&str>,
    ) -> Result<Self, String> {
        // A version that will not parse is dropped rather than refused. The
        // outcome itself is still worth recording, and answering with an error
        // would have the data plane retry a report that can never succeed.
        let observed_version = observed_version.and_then(|raw| Version::parse(raw).ok());

        let outcome = match outcome {
            "deleted" => DeploymentOutcome::Deleted,
            "running" => DeploymentOutcome::Running,
            "failed" => DeploymentOutcome::Failed,
            other => {
                return Err(format!(
                    "unknown deployment outcome '{other}', expected one of: deleted, running, failed"
                ));
            }
        };

        Ok(Self {
            dataplane_id,
            deployment_id,
            outcome,
            observed_version,
            reason: None,
        })
    }

    pub fn with_reason(mut self, reason: Option<&str>) -> Self {
        self.reason = reason
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .map(|reason| reason.chars().take(MAX_REASON_CHARS).collect());
        self
    }
}

const MAX_REASON_CHARS: usize = 500;
