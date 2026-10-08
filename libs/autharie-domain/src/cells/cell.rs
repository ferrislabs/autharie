use std::{cmp::Reverse, fmt};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    dataplane::value_objects::{DataPlaneId, Region},
    deployments::DeploymentId,
};

use super::{CellError, CellPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct CellId(pub Uuid);

impl fmt::Display for CellId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CellStatus {
    Provisioning,
    Open,
    Full,
    Draining,
    Retired,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    id: CellId,
    region: Region,
    data_plane_id: DataPlaneId,
    instance: DeploymentId,
    status: CellStatus,
    capacity: u16,
    realms: u16,
    created_at: DateTime<Utc>,
}

impl Cell {
    pub fn new(
        id: CellId,
        region: Region,
        data_plane_id: DataPlaneId,
        instance: DeploymentId,
        policy: &CellPolicy,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            region,
            data_plane_id,
            instance,
            status: CellStatus::Provisioning,
            capacity: policy.capacity(),
            realms: 0,
            created_at,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: CellId,
        region: Region,
        data_plane_id: DataPlaneId,
        instance: DeploymentId,
        status: CellStatus,
        capacity: u16,
        realms: u16,
        created_at: DateTime<Utc>,
    ) -> Result<Self, CellError> {
        let inconsistent = |reason: &str| CellError::Inconsistent {
            reason: reason.to_string(),
        };

        if capacity == 0 {
            return Err(inconsistent("its capacity is zero"));
        }
        if realms > capacity {
            return Err(inconsistent("it holds more realms than its capacity"));
        }
        match status {
            CellStatus::Provisioning if realms != 0 => {
                return Err(inconsistent("a cell being provisioned holds no realm"));
            }
            CellStatus::Open if realms == capacity => {
                return Err(inconsistent("an open cell has a free slot"));
            }
            CellStatus::Full if realms != capacity => {
                return Err(inconsistent("a full cell holds exactly its capacity"));
            }
            CellStatus::Retired if realms != 0 => {
                return Err(inconsistent("a retired cell holds no realm"));
            }
            _ => {}
        }

        Ok(Self {
            id,
            region,
            data_plane_id,
            instance,
            status,
            capacity,
            realms,
            created_at,
        })
    }

    pub fn id(&self) -> CellId {
        self.id
    }

    pub fn region(&self) -> &Region {
        &self.region
    }

    pub fn data_plane_id(&self) -> DataPlaneId {
        self.data_plane_id
    }

    pub fn instance(&self) -> DeploymentId {
        self.instance
    }

    pub fn status(&self) -> CellStatus {
        self.status
    }

    pub fn capacity(&self) -> u16 {
        self.capacity
    }

    pub fn realms(&self) -> u16 {
        self.realms
    }

    pub fn free_slots(&self) -> u16 {
        self.capacity.saturating_sub(self.realms)
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    fn takes_tenants(&self) -> bool {
        self.status == CellStatus::Open && self.realms < self.capacity
    }

    pub fn reserve(&mut self) -> Result<(), CellError> {
        match self.status {
            CellStatus::Open => {}
            CellStatus::Full => return Err(CellError::Full),
            CellStatus::Provisioning
            | CellStatus::Draining
            | CellStatus::Retired
            | CellStatus::Failed => return Err(CellError::NotOpen),
        }
        if self.realms >= self.capacity {
            return Err(CellError::Full);
        }

        self.realms += 1;
        if self.realms == self.capacity {
            self.status = CellStatus::Full;
        }

        Ok(())
    }

    pub fn release(&mut self) {
        self.realms = self.realms.saturating_sub(1);
        if self.status == CellStatus::Full && self.realms < self.capacity {
            self.status = CellStatus::Open;
        }
    }

    pub fn open(&mut self) -> Result<(), CellError> {
        match self.status {
            CellStatus::Provisioning => {
                self.status = if self.realms >= self.capacity {
                    CellStatus::Full
                } else {
                    CellStatus::Open
                };
                Ok(())
            }
            _ => Err(CellError::InvalidTransition),
        }
    }

    pub fn drain(&mut self) -> Result<(), CellError> {
        match self.status {
            CellStatus::Open | CellStatus::Full => {
                self.status = CellStatus::Draining;
                Ok(())
            }
            _ => Err(CellError::InvalidTransition),
        }
    }

    pub fn retire(&mut self) -> Result<(), CellError> {
        if self.status != CellStatus::Draining || self.realms != 0 {
            return Err(CellError::InvalidTransition);
        }
        self.status = CellStatus::Retired;
        Ok(())
    }

    pub fn fail(&mut self) {
        if self.status != CellStatus::Retired {
            self.status = CellStatus::Failed;
        }
    }
}

pub fn choose<'a>(cells: &'a [Cell], region: &Region) -> Option<&'a Cell> {
    cells
        .iter()
        .filter(|cell| cell.region == *region && cell.takes_tenants())
        .min_by_key(|cell| (Reverse(cell.realms), cell.created_at))
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn region() -> Region {
        Region::new("eu-west")
    }

    fn cell_in(region: Region, status: CellStatus, capacity: u16, realms: u16, age: i64) -> Cell {
        Cell::restore(
            CellId(Uuid::new_v4()),
            region,
            DataPlaneId(Uuid::new_v4()),
            DeploymentId(Uuid::new_v4()),
            status,
            capacity,
            realms,
            DateTime::<Utc>::UNIX_EPOCH + Duration::days(age),
        )
        .expect("a consistent cell")
    }

    fn cell(status: CellStatus, capacity: u16, realms: u16, age: i64) -> Cell {
        cell_in(region(), status, capacity, realms, age)
    }

    fn ids(cells: &[Cell]) -> Vec<CellId> {
        cells.iter().map(Cell::id).collect()
    }

    #[test]
    fn a_new_cell_is_provisioning_empty_and_sized_by_its_policy() {
        let policy = CellPolicy::new(10, 2).expect("policy");
        let cell = Cell::new(
            CellId(Uuid::new_v4()),
            region(),
            DataPlaneId(Uuid::new_v4()),
            DeploymentId(Uuid::new_v4()),
            &policy,
            DateTime::<Utc>::UNIX_EPOCH,
        );

        assert_eq!(cell.status(), CellStatus::Provisioning);
        assert_eq!(cell.realms(), 0);
        assert_eq!(cell.capacity(), 10);
    }

    /// @spec-fpr-1
    #[test]
    fn choose_takes_an_open_cell_of_the_region() {
        let cells = [cell(CellStatus::Open, 200, 3, 0)];

        assert_eq!(choose(&cells, &region()).map(Cell::id), Some(cells[0].id()));
    }

    /// @spec-fpr-1
    #[test]
    fn choose_is_empty_when_the_region_has_no_cell() {
        let cells = [cell_in(Region::new("us-east"), CellStatus::Open, 200, 3, 0)];

        assert_eq!(choose(&cells, &region()), None);
        assert_eq!(choose(&[], &region()), None);
    }

    /// @spec-fpr-2
    #[test]
    fn choose_takes_the_fullest_cell_that_still_has_room() {
        let cells = [
            cell(CellStatus::Open, 200, 10, 0),
            cell(CellStatus::Open, 200, 150, 1),
            cell(CellStatus::Open, 200, 40, 2),
        ];

        assert_eq!(
            choose(&cells, &region()).map(Cell::id),
            Some(ids(&cells)[1])
        );
    }

    /// @spec-fpr-2
    #[test]
    fn choose_never_takes_a_cell_of_another_region_even_if_fuller() {
        let cells = [
            cell_in(Region::new("us-east"), CellStatus::Open, 200, 199, 0),
            cell(CellStatus::Open, 200, 1, 1),
        ];

        assert_eq!(
            choose(&cells, &region()).map(Cell::id),
            Some(ids(&cells)[1])
        );
    }

    #[test]
    fn choose_breaks_a_tie_on_realms_with_the_oldest_cell() {
        let cells = [
            cell(CellStatus::Open, 200, 5, 9),
            cell(CellStatus::Open, 200, 5, 1),
            cell(CellStatus::Open, 200, 5, 4),
        ];

        assert_eq!(
            choose(&cells, &region()).map(Cell::id),
            Some(ids(&cells)[1])
        );
    }

    /// @spec-fpr-3
    #[test]
    fn reserving_the_last_slot_closes_the_cell_and_choose_moves_on() {
        let mut cells = [
            cell(CellStatus::Open, 200, 199, 0),
            cell(CellStatus::Open, 200, 10, 1),
        ];
        let first = cells[0].id();

        assert_eq!(choose(&cells, &region()).map(Cell::id), Some(first));
        cells[0].reserve().expect("the 200th slot");

        assert_eq!(cells[0].status(), CellStatus::Full);
        assert_eq!(cells[0].realms(), 200);
        assert_eq!(choose(&cells, &region()).map(Cell::id), Some(cells[1].id()));
        assert_eq!(cells[0].reserve(), Err(CellError::Full));
    }

    /// @spec-fpr-9
    #[test]
    fn a_full_cell_that_releases_a_slot_is_open_again() {
        let mut full = cell(CellStatus::Full, 200, 200, 0);

        full.release();

        assert_eq!(full.status(), CellStatus::Open);
        assert_eq!(full.realms(), 199);
    }

    #[test]
    fn release_never_goes_below_zero() {
        let mut empty = cell(CellStatus::Open, 200, 0, 0);

        empty.release();

        assert_eq!(empty.realms(), 0);
        assert_eq!(empty.status(), CellStatus::Open);
    }

    #[test]
    fn release_does_not_reopen_a_draining_cell() {
        let mut draining = cell(CellStatus::Draining, 200, 3, 0);

        draining.release();

        assert_eq!(draining.status(), CellStatus::Draining);
        assert_eq!(draining.realms(), 2);
    }

    /// @spec-fpr-10
    #[test]
    fn a_cell_that_is_not_open_is_never_chosen_and_refuses_a_reservation() {
        let expected = [
            (CellStatus::Provisioning, CellError::NotOpen),
            (CellStatus::Full, CellError::Full),
            (CellStatus::Draining, CellError::NotOpen),
            (CellStatus::Retired, CellError::NotOpen),
            (CellStatus::Failed, CellError::NotOpen),
        ];

        for (status, error) in expected {
            let held = if status == CellStatus::Full { 200 } else { 0 };
            let mut closed = cell(status, 200, held, 0);

            assert_eq!(choose(std::slice::from_ref(&closed), &region()), None);
            assert_eq!(closed.reserve(), Err(error), "{status:?}");
            assert_eq!(closed.realms(), held);
        }
    }

    #[test]
    fn a_cell_goes_from_provisioning_to_open_to_draining_to_retired() {
        let mut cell = cell(CellStatus::Provisioning, 200, 0, 0);

        assert_eq!(cell.retire(), Err(CellError::InvalidTransition));
        cell.open().expect("opens");
        assert_eq!(cell.status(), CellStatus::Open);
        assert_eq!(cell.open(), Err(CellError::InvalidTransition));
        cell.drain().expect("drains");
        assert_eq!(cell.status(), CellStatus::Draining);
        cell.retire().expect("retires");
        assert_eq!(cell.status(), CellStatus::Retired);
    }

    #[test]
    fn a_full_cell_can_drain_but_a_provisioning_one_cannot() {
        let mut full = cell(CellStatus::Full, 200, 200, 0);
        let mut provisioning = cell(CellStatus::Provisioning, 200, 0, 0);

        assert!(full.drain().is_ok());
        assert_eq!(provisioning.drain(), Err(CellError::InvalidTransition));
    }

    #[test]
    fn a_draining_cell_with_realms_cannot_retire() {
        let mut draining = cell(CellStatus::Draining, 200, 1, 0);

        assert_eq!(draining.retire(), Err(CellError::InvalidTransition));
        assert_eq!(draining.status(), CellStatus::Draining);
    }

    #[test]
    fn any_cell_that_is_not_retired_can_fail() {
        for status in [
            CellStatus::Provisioning,
            CellStatus::Open,
            CellStatus::Full,
            CellStatus::Draining,
        ] {
            let held = if status == CellStatus::Full { 200 } else { 0 };
            let mut cell = cell(status, 200, held, 0);
            cell.fail();
            assert_eq!(cell.status(), CellStatus::Failed);
        }
    }

    #[test]
    fn a_retired_cell_stays_retired_when_it_is_told_it_failed() {
        let mut cell = cell(CellStatus::Retired, 200, 0, 0);

        cell.fail();

        assert_eq!(cell.status(), CellStatus::Retired);
    }

    #[test]
    fn reserving_on_a_full_cell_leaves_it_as_it_was() {
        let mut cell = cell(CellStatus::Full, 5, 5, 0);

        assert_eq!(cell.reserve(), Err(CellError::Full));
        assert_eq!(cell.realms(), 5);
        assert_eq!(cell.status(), CellStatus::Full);
    }

    #[test]
    fn a_stored_cell_that_cannot_be_real_is_refused() {
        let restore = |status, capacity, realms| {
            Cell::restore(
                CellId(Uuid::new_v4()),
                region(),
                DataPlaneId(Uuid::new_v4()),
                DeploymentId(Uuid::new_v4()),
                status,
                capacity,
                realms,
                DateTime::<Utc>::UNIX_EPOCH,
            )
        };

        for (status, capacity, realms) in [
            (CellStatus::Open, 0, 0),
            (CellStatus::Open, 5, 6),
            (CellStatus::Open, 5, 5),
            (CellStatus::Full, 5, 4),
            (CellStatus::Retired, 5, 1),
            (CellStatus::Provisioning, 5, 1),
        ] {
            assert!(
                matches!(
                    restore(status, capacity, realms),
                    Err(CellError::Inconsistent { .. })
                ),
                "{status:?} {capacity} {realms}"
            );
        }
        assert!(restore(CellStatus::Draining, 5, 5).is_ok());
    }
}
