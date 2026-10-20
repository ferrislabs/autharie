use std::future::Future;

use autharie_auth::Identity;
use chrono::{DateTime, Utc};

use crate::{
    CoreError,
    organisation::{OrganisationId, ports::OrganisationRepository},
    platform::{PlatformRight, ports::PlatformPolicy},
};

use super::{CellError, CellRepository, CellStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellsSettings {
    pub organisation_id: Option<OrganisationId>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
}

#[cfg_attr(test, mockall::automock)]
pub trait CellsSettingsRepository: Send + Sync {
    fn get(&self) -> impl Future<Output = Result<CellsSettings, CoreError>> + Send;

    fn set(
        &self,
        organisation: Option<OrganisationId>,
        updated_by: &str,
        updated_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<CellsSettings, CoreError>> + Send;
}

pub struct CellsSettingsService<S, C, O, P>
where
    S: CellsSettingsRepository,
    C: CellRepository,
    O: OrganisationRepository,
    P: PlatformPolicy,
{
    settings: S,
    cells: C,
    organisations: O,
    policy: P,
}

impl<S, C, O, P> CellsSettingsService<S, C, O, P>
where
    S: CellsSettingsRepository,
    C: CellRepository,
    O: OrganisationRepository,
    P: PlatformPolicy,
{
    pub fn new(settings: S, cells: C, organisations: O, policy: P) -> Self {
        Self {
            settings,
            cells,
            organisations,
            policy,
        }
    }

    pub async fn get(&self, identity: Identity) -> Result<CellsSettings, CoreError> {
        self.policy
            .require(identity, PlatformRight::ViewEstate)
            .await?;

        self.settings.get().await
    }

    pub async fn set_organisation(
        &self,
        identity: Identity,
        organisation: Option<OrganisationId>,
    ) -> Result<CellsSettings, CoreError> {
        self.policy
            .require(identity.clone(), PlatformRight::OperateFleet)
            .await?;

        let current = self.settings.get().await?;
        if current.organisation_id == organisation {
            return Ok(current);
        }

        if let Some(id) = organisation
            && self.organisations.find_by_id(&id).await?.is_none()
        {
            return Err(CoreError::OrganisationNotFound { id: id.0 });
        }

        let active = self
            .cells
            .list()
            .await?
            .iter()
            .filter(|cell| cell.status() != CellStatus::Retired)
            .count();
        if active > 0 {
            return Err(CellError::OwnerInUse { active }.into());
        }

        self.settings
            .set(organisation, identity.id(), Utc::now())
            .await
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use uuid::Uuid;

    use super::*;
    use crate::{
        cells::{Cell, CellId, MockCellRepository},
        dataplane::value_objects::{DataPlaneId, Region},
        deployments::DeploymentId,
        organisation::{
            Organisation,
            ports::MockOrganisationRepository,
            value_objects::{OrganisationName, OrganisationSlug, Plan},
        },
        platform::fixtures::Granting,
        user::UserId,
    };

    fn identity() -> Identity {
        Identity::Client(autharie_auth::Client {
            id: "operator".to_string(),
            client_id: "operator".to_string(),
            roles: vec![],
            scopes: vec![],
        })
    }

    fn cell(status: CellStatus) -> Cell {
        Cell::restore(
            CellId(Uuid::new_v4()),
            Region::new("fr-par"),
            DataPlaneId(Uuid::new_v4()),
            DeploymentId(Uuid::new_v4()),
            status,
            200,
            0,
            DateTime::<Utc>::UNIX_EPOCH,
        )
        .expect("a consistent cell")
    }

    fn settings(organisation: Option<OrganisationId>) -> CellsSettings {
        CellsSettings {
            organisation_id: organisation,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
            updated_by: None,
        }
    }

    fn stored(organisation: Option<OrganisationId>) -> MockCellsSettingsRepository {
        let mut repository = MockCellsSettingsRepository::new();
        repository
            .expect_get()
            .returning(move || Box::pin(std::future::ready(Ok(settings(organisation)))));

        repository
    }

    fn holding(cells: Vec<Cell>) -> MockCellRepository {
        let mut repository = MockCellRepository::new();
        repository
            .expect_list()
            .returning(move || Box::pin(std::future::ready(Ok(cells.clone()))));

        repository
    }

    fn organisations(known: Option<OrganisationId>) -> MockOrganisationRepository {
        let mut repository = MockOrganisationRepository::new();
        repository.expect_find_by_id().returning(move |_| {
            let found = known.map(|id| {
                let mut organisation = Organisation::new(
                    OrganisationName::new("Cells").expect("a name"),
                    OrganisationSlug::new("cells").expect("a slug"),
                    UserId(Uuid::new_v4()),
                    Plan::Free,
                );
                organisation.id = id;
                organisation
            });
            Box::pin(std::future::ready(Ok(found)))
        });

        repository
    }

    fn expecting_write(
        mut repository: MockCellsSettingsRepository,
        organisation: Option<OrganisationId>,
    ) -> MockCellsSettingsRepository {
        repository
            .expect_set()
            .withf(move |asked, by, _| *asked == organisation && by == "operator")
            .times(1)
            .returning(move |asked, _, _| Box::pin(std::future::ready(Ok(settings(asked)))));

        repository
    }

    #[tokio::test]
    async fn an_unconfigured_installation_takes_an_existing_organisation() {
        let target = OrganisationId(Uuid::new_v4());

        let saved = CellsSettingsService::new(
            expecting_write(stored(None), Some(target)),
            holding(vec![]),
            organisations(Some(target)),
            Granting::everything(),
        )
        .set_organisation(identity(), Some(target))
        .await
        .expect("saved");

        assert_eq!(saved.organisation_id, Some(target));
    }

    #[tokio::test]
    async fn retired_cells_do_not_hold_the_owner() {
        let target = OrganisationId(Uuid::new_v4());

        CellsSettingsService::new(
            expecting_write(stored(Some(OrganisationId(Uuid::new_v4()))), Some(target)),
            holding(vec![cell(CellStatus::Retired), cell(CellStatus::Retired)]),
            organisations(Some(target)),
            Granting::everything(),
        )
        .set_organisation(identity(), Some(target))
        .await
        .expect("saved");
    }

    #[tokio::test]
    async fn a_cell_that_is_not_retired_keeps_the_owner_in_place() {
        for status in [
            CellStatus::Provisioning,
            CellStatus::Open,
            CellStatus::Draining,
            CellStatus::Failed,
        ] {
            let target = OrganisationId(Uuid::new_v4());

            let refused = CellsSettingsService::new(
                stored(Some(OrganisationId(Uuid::new_v4()))),
                holding(vec![cell(CellStatus::Retired), cell(status)]),
                organisations(Some(target)),
                Granting::everything(),
            )
            .set_organisation(identity(), Some(target))
            .await
            .expect_err("a change with a live cell");

            assert!(
                matches!(
                    refused,
                    CoreError::Cell(CellError::OwnerInUse { active: 1 })
                ),
                "{status:?}: {refused}"
            );
        }
    }

    #[tokio::test]
    async fn clearing_the_owner_follows_the_same_rule() {
        let refused = CellsSettingsService::new(
            stored(Some(OrganisationId(Uuid::new_v4()))),
            holding(vec![cell(CellStatus::Open)]),
            organisations(None),
            Granting::everything(),
        )
        .set_organisation(identity(), None)
        .await
        .expect_err("a clear with a live cell");

        assert!(matches!(
            refused,
            CoreError::Cell(CellError::OwnerInUse { .. })
        ));

        CellsSettingsService::new(
            expecting_write(stored(Some(OrganisationId(Uuid::new_v4()))), None),
            holding(vec![]),
            organisations(None),
            Granting::everything(),
        )
        .set_organisation(identity(), None)
        .await
        .expect("cleared");
    }

    #[tokio::test]
    async fn setting_the_same_owner_writes_nothing_even_with_live_cells() {
        let current = OrganisationId(Uuid::new_v4());

        let saved = CellsSettingsService::new(
            stored(Some(current)),
            holding(vec![cell(CellStatus::Open)]),
            organisations(Some(current)),
            Granting::everything(),
        )
        .set_organisation(identity(), Some(current))
        .await
        .expect("a no-op");

        assert_eq!(saved.organisation_id, Some(current));
    }

    #[tokio::test]
    async fn an_unknown_organisation_is_not_found() {
        let target = OrganisationId(Uuid::new_v4());

        let refused = CellsSettingsService::new(
            stored(None),
            holding(vec![]),
            organisations(None),
            Granting::everything(),
        )
        .set_organisation(identity(), Some(target))
        .await
        .expect_err("no such organisation");

        assert!(matches!(
            refused,
            CoreError::OrganisationNotFound { id } if id == target.0
        ));
    }

    #[tokio::test]
    async fn changing_the_owner_needs_operate_fleet() {
        let refused = CellsSettingsService::new(
            stored(None),
            holding(vec![]),
            organisations(None),
            Granting::only(PlatformRight::ViewEstate),
        )
        .set_organisation(identity(), None)
        .await
        .expect_err("a reader changed the owner");

        assert!(matches!(refused, CoreError::MissingPlatformRight { .. }));
    }

    #[tokio::test]
    async fn reading_the_owner_needs_view_estate() {
        let service = |granting| {
            CellsSettingsService::new(stored(None), holding(vec![]), organisations(None), granting)
        };

        assert!(service(Granting::nothing()).get(identity()).await.is_err());
        assert!(
            service(Granting::only(PlatformRight::ViewEstate))
                .get(identity())
                .await
                .is_ok()
        );
    }
}
