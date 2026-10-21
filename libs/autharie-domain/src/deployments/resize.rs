use chrono::{DateTime, Utc};

use crate::{
    CoreError,
    billing::{BillingError, Hosting},
    dataplane::{
        ports::DataPlaneRepository,
        value_objects::{DataPlaneMode, DeploymentResources},
    },
    deployments::{Deployment, distribution::Distribution},
    offers::{DatabasePlacement, Offer},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resize {
    Unchanged,
    OfferOnly,
    Resources(DeploymentResources),
}

impl Deployment {
    pub fn resize_to(&self, offer: Offer, plane: DataPlaneMode) -> Result<Resize, CoreError> {
        let database = self
            .offer
            .map_or(DatabasePlacement::Managed, Offer::database);
        if offer.database() != database {
            return Err(BillingError::DatabasePlacementChange {
                from: database,
                to: offer.database(),
            }
            .into());
        }

        let mode = self.offer.map_or(plane, Offer::mode);
        if offer.mode() != mode {
            return Err(BillingError::DataPlaneModeChange {
                from: mode,
                to: offer.mode(),
            }
            .into());
        }

        let wanted = offer.resources();
        let target = DeploymentResources {
            storage_gib: wanted.storage_gib.max(self.resources.storage_gib),
            ..wanted
        };

        if target == self.resources {
            return Ok(if self.offer == Some(offer) {
                Resize::Unchanged
            } else {
                Resize::OfferOnly
            });
        }

        if self.distribution != Distribution::Shared {
            return Err(BillingError::ResizeNotAvailable {
                hosting: Hosting::of(&self.distribution),
            }
            .into());
        }

        if self.is_busy() {
            return Err(CoreError::DeploymentBusy {
                deployment: self.id.0,
                status: self.status.to_string(),
                operation: "resized".to_string(),
            });
        }

        Ok(Resize::Resources(target))
    }

    pub fn apply_resize(&mut self, offer: Offer, resize: Resize, at: DateTime<Utc>) {
        match resize {
            Resize::Unchanged => return,
            Resize::OfferOnly => {}
            Resize::Resources(target) => self.resources = target,
        }

        self.offer = Some(offer);
        self.updated_at = at;
    }
}

pub async fn plan_resize<P: DataPlaneRepository>(
    planes: &P,
    deployment: &Deployment,
    offer: Offer,
) -> Result<Resize, CoreError> {
    let missing = || CoreError::DataPlaneNotFound {
        id: deployment.dataplane_id,
    };
    let plane = planes
        .find_by_id(&deployment.dataplane_id)
        .await?
        .ok_or_else(missing)?;
    let resize = deployment.resize_to(offer, plane.allocation.mode())?;

    if let Resize::Resources(target) = resize
        && target.exceeds(deployment.resources)
    {
        let load = planes
            .load_excluding(&deployment.dataplane_id, &deployment.id)
            .await?
            .ok_or_else(missing)?;

        if !load.has_room_for(target) {
            return Err(BillingError::NoRoomToResize { wanted: target }.into());
        }
    }

    Ok(resize)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use uuid::Uuid;

    use super::*;
    use crate::{
        billing::{BillingError, Hosting},
        dataplane::{
            entities::DataPlane,
            ports::MockDataPlaneRepository,
            value_objects::{
                Capacity, DataPlaneAllocation, DataPlaneId, DataPlaneMode, PlaneLoad, Region,
            },
        },
        deployments::{
            DeploymentId, DeploymentName, DeploymentStatus,
            distribution::{Distribution, tests::customer_cloud},
        },
        offers::DatabasePlacement,
        organisation::OrganisationId,
        user::UserId,
        version::Version,
    };

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, 9, 0, 0).unwrap()
    }

    fn sized(cpu_millis: u32, memory_mib: u32, storage_gib: u32) -> DeploymentResources {
        DeploymentResources {
            cpu_millis,
            memory_mib,
            storage_gib,
        }
    }

    fn deployment(offer: Option<Offer>, resources: DeploymentResources) -> Deployment {
        Deployment {
            topology: Default::default(),
            id: DeploymentId(Uuid::from_u128(1)),
            organisation_id: OrganisationId(Uuid::from_u128(2)),
            dataplane_id: DataPlaneId(Uuid::from_u128(3)),
            name: DeploymentName("auth".to_string()),
            kind: crate::deployments::DeploymentKind::Ferriskey,
            version: Version::new(26, 0, 1),
            status: DeploymentStatus::Successful,
            namespace: "production-auth".to_string(),
            environment: crate::deployments::environment::Environment::Development,
            offer,
            restored_from: None,
            resources,
            created_by: UserId(Uuid::from_u128(4)),
            created_at: at(1),
            updated_at: at(1),
            deployed_at: None,
            deleted_at: None,
            auto_upgrade: Default::default(),
            maintenance_window: None,
            network_access: crate::deployments::network::NetworkAccess::Open,
            last_verified_restore_at: None,
            last_restore_drill_seconds: None,
            log_shipping_enabled: false,
            iam_settings: Default::default(),
            distribution: Distribution::Shared,
        }
    }

    fn on(offer: Offer) -> Deployment {
        deployment(Some(offer), offer.resources())
    }

    fn before_the_catalogue() -> Deployment {
        deployment(None, DeploymentResources::DEFAULT)
    }

    fn refusal(result: Result<Resize, CoreError>) -> BillingError {
        match result {
            Err(CoreError::Billing(error)) => error,
            other => panic!("expected a billing refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_bigger_offer_gives_the_deployment_its_size() {
        assert_eq!(
            on(Offer::Standard)
                .resize_to(Offer::Scale, DataPlaneMode::Shared)
                .unwrap(),
            Resize::Resources(sized(2000, 4096, 20))
        );
        assert_eq!(
            on(Offer::Sandbox)
                .resize_to(Offer::Standard, DataPlaneMode::Shared)
                .unwrap(),
            Resize::Resources(sized(500, 1024, 1))
        );
    }

    #[test]
    fn a_smaller_offer_shrinks_cpu_and_memory_and_keeps_the_storage() {
        assert_eq!(
            on(Offer::Scale)
                .resize_to(Offer::Standard, DataPlaneMode::Shared)
                .unwrap(),
            Resize::Resources(sized(500, 1024, 20))
        );
        assert_eq!(
            on(Offer::Scale)
                .resize_to(Offer::Sandbox, DataPlaneMode::Shared)
                .unwrap(),
            Resize::Resources(sized(250, 512, 20))
        );
    }

    #[test]
    fn storage_larger_than_the_offer_gives_is_kept() {
        let grown = deployment(Some(Offer::Standard), sized(500, 1024, 50));

        assert_eq!(
            grown
                .resize_to(Offer::Scale, DataPlaneMode::Shared)
                .unwrap(),
            Resize::Resources(sized(2000, 4096, 50))
        );
    }

    #[test]
    fn the_offer_a_deployment_already_has_changes_nothing() {
        for offer in [Offer::Sandbox, Offer::Standard, Offer::Scale] {
            assert_eq!(
                on(offer).resize_to(offer, DataPlaneMode::Shared).unwrap(),
                Resize::Unchanged
            );
        }
        assert_eq!(
            on(Offer::Scale)
                .resize_to(Offer::Scale, DataPlaneMode::Shared)
                .unwrap(),
            Resize::Unchanged
        );
    }

    #[test]
    fn a_deployment_of_the_size_the_offer_gives_only_records_the_offer() {
        assert_eq!(
            before_the_catalogue()
                .resize_to(Offer::Standard, DataPlaneMode::Shared)
                .unwrap(),
            Resize::OfferOnly
        );
        assert_eq!(
            deployment(Some(Offer::Sandbox), Offer::Standard.resources())
                .resize_to(Offer::Standard, DataPlaneMode::Shared)
                .unwrap(),
            Resize::OfferOnly
        );
    }

    #[test]
    fn a_deployment_without_an_offer_counts_as_managed_and_takes_its_mode_from_its_plane() {
        let before = before_the_catalogue();

        assert_eq!(
            refusal(before.resize_to(Offer::Free, DataPlaneMode::Shared)),
            BillingError::DatabasePlacementChange {
                from: DatabasePlacement::Managed,
                to: DatabasePlacement::Shared,
            }
        );
        assert_eq!(
            refusal(before.resize_to(Offer::Private, DataPlaneMode::Shared)),
            BillingError::DataPlaneModeChange {
                from: DataPlaneMode::Shared,
                to: DataPlaneMode::Dedicated,
            }
        );
        assert_eq!(
            before
                .resize_to(Offer::Scale, DataPlaneMode::Shared)
                .unwrap(),
            Resize::Resources(sized(2000, 4096, 20))
        );
    }

    #[test]
    fn a_deployment_without_an_offer_on_a_dedicated_plane_stays_dedicated_whatever_its_distribution()
     {
        let before = before_the_catalogue();
        assert_eq!(before.distribution, Distribution::Shared);

        assert_eq!(
            before
                .resize_to(Offer::Private, DataPlaneMode::Dedicated)
                .unwrap(),
            Resize::Resources(sized(2000, 4096, 20))
        );
        assert_eq!(
            refusal(before.resize_to(Offer::Standard, DataPlaneMode::Dedicated)),
            BillingError::DataPlaneModeChange {
                from: DataPlaneMode::Dedicated,
                to: DataPlaneMode::Shared,
            }
        );
        assert_eq!(
            refusal(before.resize_to(Offer::Scale, DataPlaneMode::Dedicated)),
            BillingError::DataPlaneModeChange {
                from: DataPlaneMode::Dedicated,
                to: DataPlaneMode::Shared,
            }
        );
    }

    #[test]
    fn a_database_cannot_move_between_the_shared_cluster_and_a_managed_one() {
        assert_eq!(
            refusal(on(Offer::Free).resize_to(Offer::Standard, DataPlaneMode::Shared)),
            BillingError::DatabasePlacementChange {
                from: DatabasePlacement::Shared,
                to: DatabasePlacement::Managed,
            }
        );
        assert_eq!(
            refusal(on(Offer::Free).resize_to(Offer::Sandbox, DataPlaneMode::Shared)),
            BillingError::DatabasePlacementChange {
                from: DatabasePlacement::Shared,
                to: DatabasePlacement::Managed,
            }
        );
        assert_eq!(
            refusal(on(Offer::Scale).resize_to(Offer::Free, DataPlaneMode::Shared)),
            BillingError::DatabasePlacementChange {
                from: DatabasePlacement::Managed,
                to: DatabasePlacement::Shared,
            }
        );
    }

    #[test]
    fn the_database_is_refused_before_the_data_plane() {
        assert!(matches!(
            refusal(on(Offer::Free).resize_to(Offer::Private, DataPlaneMode::Shared)),
            BillingError::DatabasePlacementChange { .. }
        ));
    }

    #[test]
    fn a_shared_deployment_cannot_move_to_a_dedicated_data_plane() {
        assert_eq!(
            refusal(on(Offer::Standard).resize_to(Offer::Private, DataPlaneMode::Shared)),
            BillingError::DataPlaneModeChange {
                from: DataPlaneMode::Shared,
                to: DataPlaneMode::Dedicated,
            }
        );
    }

    #[test]
    fn a_deployment_in_a_customer_cloud_cannot_move_to_a_shared_data_plane() {
        let mut elsewhere = deployment(None, DeploymentResources::DEFAULT);
        elsewhere.distribution = customer_cloud();

        assert_eq!(
            refusal(elsewhere.resize_to(Offer::Standard, DataPlaneMode::Dedicated)),
            BillingError::DataPlaneModeChange {
                from: DataPlaneMode::Dedicated,
                to: DataPlaneMode::Shared,
            }
        );

        let mut private = on(Offer::Private);
        private.distribution = customer_cloud();

        assert_eq!(
            refusal(private.resize_to(Offer::Standard, DataPlaneMode::Dedicated)),
            BillingError::DataPlaneModeChange {
                from: DataPlaneMode::Dedicated,
                to: DataPlaneMode::Shared,
            }
        );
    }

    #[test]
    fn a_deployment_that_is_not_on_a_shared_distribution_is_not_resized() {
        let mut customer = deployment(Some(Offer::Private), sized(1000, 2048, 10));
        customer.distribution = customer_cloud();
        let mut own = deployment(Some(Offer::Private), sized(1000, 2048, 10));
        own.distribution = Distribution::SelfHosted;

        assert_eq!(
            refusal(customer.resize_to(Offer::Private, DataPlaneMode::Dedicated)),
            BillingError::ResizeNotAvailable {
                hosting: Hosting::CustomerCloud
            }
        );
        assert_eq!(
            refusal(own.resize_to(Offer::Private, DataPlaneMode::Dedicated)),
            BillingError::ResizeNotAvailable {
                hosting: Hosting::SelfHosted
            }
        );
    }

    #[test]
    fn a_deployment_that_is_not_shared_still_records_an_offer_of_its_own_size() {
        let mut customer = on(Offer::Private);
        customer.distribution = customer_cloud();

        assert_eq!(
            customer
                .resize_to(Offer::Private, DataPlaneMode::Dedicated)
                .unwrap(),
            Resize::Unchanged
        );

        customer.offer = None;
        customer.resources = Offer::Private.resources();

        assert_eq!(
            customer
                .resize_to(Offer::Private, DataPlaneMode::Dedicated)
                .unwrap(),
            Resize::OfferOnly
        );
    }

    #[test]
    fn a_deployment_being_upgraded_is_not_resized() {
        let mut busy = on(Offer::Standard);
        busy.status = DeploymentStatus::Upgrading;

        let refused = busy
            .resize_to(Offer::Scale, DataPlaneMode::Shared)
            .expect_err("busy");

        assert!(
            matches!(
                &refused,
                CoreError::DeploymentBusy { deployment, status, operation }
                    if *deployment == busy.id.0 && status == "upgrading" && operation == "resized"
            ),
            "{refused}"
        );
    }

    #[test]
    fn a_deployment_being_upgraded_still_records_an_offer_of_its_own_size() {
        let mut busy = before_the_catalogue();
        busy.status = DeploymentStatus::Upgrading;

        assert_eq!(
            busy.resize_to(Offer::Standard, DataPlaneMode::Shared)
                .unwrap(),
            Resize::OfferOnly
        );
    }

    #[test]
    fn a_stopped_deployment_is_resized_and_stays_stopped() {
        let mut stopped = on(Offer::Standard);
        stopped.status = DeploymentStatus::Stopped;

        assert_eq!(
            stopped
                .resize_to(Offer::Scale, DataPlaneMode::Shared)
                .unwrap(),
            Resize::Resources(sized(2000, 4096, 20))
        );
    }

    #[test]
    fn applying_a_resize_records_the_offer_the_size_and_the_instant() {
        let mut grown = on(Offer::Standard);
        let size = sized(2000, 4096, 20);

        grown.apply_resize(Offer::Scale, Resize::Resources(size), at(9));

        assert_eq!(grown.offer, Some(Offer::Scale));
        assert_eq!(grown.resources, size);
        assert_eq!(grown.updated_at, at(9));
    }

    #[test]
    fn applying_an_offer_only_leaves_the_size_alone() {
        let mut legacy = before_the_catalogue();

        legacy.apply_resize(Offer::Standard, Resize::OfferOnly, at(9));

        assert_eq!(legacy.offer, Some(Offer::Standard));
        assert_eq!(legacy.resources, DeploymentResources::DEFAULT);
        assert_eq!(legacy.updated_at, at(9));
    }

    #[test]
    fn applying_nothing_changes_nothing() {
        let mut same = on(Offer::Standard);
        let before = same.clone();

        same.apply_resize(Offer::Standard, Resize::Unchanged, at(9));

        assert_eq!(same, before);
    }

    fn plane(used: DeploymentResources) -> PlaneLoad {
        PlaneLoad {
            capacity: Capacity::new(4_000, 8_192, 100).unwrap(),
            used,
        }
    }

    fn planes_in(deployment: &Deployment, mode: DataPlaneMode) -> MockDataPlaneRepository {
        let allocation = match mode {
            DataPlaneMode::Shared => DataPlaneAllocation::Shared,
            DataPlaneMode::Dedicated => DataPlaneAllocation::Dedicated {
                organisation_id: deployment.organisation_id,
            },
        };
        let mut found = DataPlane::new(
            allocation,
            Region::new("eu-west-1"),
            Capacity::new(4_000, 8_192, 100).unwrap(),
        );
        found.id = deployment.dataplane_id;
        let plane_id = deployment.dataplane_id;
        let mut planes = MockDataPlaneRepository::new();
        planes
            .expect_find_by_id()
            .withf(move |id| *id == plane_id)
            .returning(move |_| {
                let found = found.clone();
                Box::pin(async move { Ok(Some(found)) })
            });
        planes
    }

    fn holding(
        mut planes: MockDataPlaneRepository,
        deployment: &Deployment,
        load: Option<PlaneLoad>,
    ) -> MockDataPlaneRepository {
        let plane_id = deployment.dataplane_id;
        let id = deployment.id;
        planes
            .expect_load_excluding()
            .withf(move |plane, excluding| *plane == plane_id && *excluding == id)
            .times(1)
            .returning(move |_, _| Box::pin(async move { Ok(load) }));
        planes
    }

    fn planes_holding(deployment: &Deployment, load: Option<PlaneLoad>) -> MockDataPlaneRepository {
        holding(
            planes_in(deployment, DataPlaneMode::Shared),
            deployment,
            load,
        )
    }

    #[tokio::test]
    async fn a_resize_that_grows_is_confirmed_when_the_plane_has_room_without_the_deployment() {
        let grown = on(Offer::Standard);
        let planes = planes_holding(&grown, Some(plane(sized(2_000, 4_096, 80))));

        let planned = plan_resize(&planes, &grown, Offer::Scale).await.unwrap();

        assert_eq!(planned, Resize::Resources(sized(2000, 4096, 20)));
    }

    #[tokio::test]
    async fn a_resize_that_grows_is_refused_when_the_plane_has_no_room_in_one_dimension() {
        let grown = on(Offer::Standard);

        for used in [sized(2_001, 0, 0), sized(0, 4_097, 0), sized(0, 0, 81)] {
            let planes = planes_holding(&grown, Some(plane(used)));

            let refused = plan_resize(&planes, &grown, Offer::Scale)
                .await
                .expect_err("no room");

            assert!(
                matches!(
                    refused,
                    CoreError::Billing(BillingError::NoRoomToResize { wanted })
                        if wanted == sized(2000, 4096, 20)
                ),
                "{used:?}: {refused}"
            );
        }
    }

    #[tokio::test]
    async fn a_resize_that_grows_must_fit_whole_even_in_a_dimension_that_shrinks() {
        let mixed = deployment(Some(Offer::Standard), sized(500, 8_000, 1));

        let crowded = planes_holding(&mixed, Some(plane(sized(0, 6_000, 0))));
        let refused = plan_resize(&crowded, &mixed, Offer::Scale)
            .await
            .expect_err("memory shrinks but the plane has no room for the new size of it");
        assert!(
            matches!(
                refused,
                CoreError::Billing(BillingError::NoRoomToResize { wanted })
                    if wanted == sized(2000, 4096, 20)
            ),
            "{refused}"
        );

        let exact = planes_holding(&mixed, Some(plane(sized(0, 4_096, 0))));
        assert_eq!(
            plan_resize(&exact, &mixed, Offer::Scale).await.unwrap(),
            Resize::Resources(sized(2000, 4096, 20))
        );
    }

    #[tokio::test]
    async fn a_resize_that_shrinks_asks_the_plane_for_its_mode_and_never_for_room() {
        let shrunk = on(Offer::Scale);
        let planes = planes_in(&shrunk, DataPlaneMode::Shared);

        let planned = plan_resize(&planes, &shrunk, Offer::Standard)
            .await
            .unwrap();

        assert_eq!(planned, Resize::Resources(sized(500, 1024, 20)));
    }

    #[tokio::test]
    async fn nothing_that_leaves_the_size_alone_asks_the_plane_for_room() {
        let standard = on(Offer::Standard);
        let legacy = before_the_catalogue();

        assert_eq!(
            plan_resize(
                &planes_in(&standard, DataPlaneMode::Shared),
                &standard,
                Offer::Standard
            )
            .await
            .unwrap(),
            Resize::Unchanged
        );
        assert_eq!(
            plan_resize(
                &planes_in(&legacy, DataPlaneMode::Shared),
                &legacy,
                Offer::Standard
            )
            .await
            .unwrap(),
            Resize::OfferOnly
        );
    }

    #[tokio::test]
    async fn a_refusal_comes_before_any_question_about_room() {
        let free = on(Offer::Free);
        let planes = planes_in(&free, DataPlaneMode::Shared);

        let refused = plan_resize(&planes, &free, Offer::Standard)
            .await
            .expect_err("a database move");

        assert!(matches!(
            refused,
            CoreError::Billing(BillingError::DatabasePlacementChange { .. })
        ));
    }

    #[tokio::test]
    async fn a_deployment_without_an_offer_follows_the_mode_of_the_plane_it_runs_on() {
        let legacy = before_the_catalogue();
        assert_eq!(legacy.distribution, Distribution::Shared);

        let allowed = holding(
            planes_in(&legacy, DataPlaneMode::Dedicated),
            &legacy,
            Some(plane(sized(0, 0, 0))),
        );
        assert_eq!(
            plan_resize(&allowed, &legacy, Offer::Private)
                .await
                .unwrap(),
            Resize::Resources(sized(2000, 4096, 20))
        );

        let refused = plan_resize(
            &planes_in(&legacy, DataPlaneMode::Dedicated),
            &legacy,
            Offer::Standard,
        )
        .await
        .expect_err("it stays on a dedicated plane");
        assert!(
            matches!(
                refused,
                CoreError::Billing(BillingError::DataPlaneModeChange {
                    from: DataPlaneMode::Dedicated,
                    to: DataPlaneMode::Shared,
                })
            ),
            "{refused}"
        );
    }

    #[tokio::test]
    async fn a_deployment_whose_plane_is_gone_cannot_be_resized() {
        let orphan = on(Offer::Standard);
        let mut planes = MockDataPlaneRepository::new();
        planes
            .expect_find_by_id()
            .returning(|_| Box::pin(async { Ok(None) }));

        let refused = plan_resize(&planes, &orphan, Offer::Scale)
            .await
            .expect_err("no such plane");

        assert!(
            matches!(
                refused,
                CoreError::DataPlaneNotFound { id } if id == orphan.dataplane_id
            ),
            "{refused}"
        );
    }

    #[tokio::test]
    async fn a_plane_that_disappears_before_its_load_is_read_cannot_be_grown_into() {
        let grown = on(Offer::Standard);
        let planes = planes_holding(&grown, None);

        let refused = plan_resize(&planes, &grown, Offer::Scale)
            .await
            .expect_err("no such plane");

        assert!(
            matches!(
                refused,
                CoreError::DataPlaneNotFound { id } if id == grown.dataplane_id
            ),
            "{refused}"
        );
    }
}
