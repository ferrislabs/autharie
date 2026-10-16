mod cell;
mod errors;
#[cfg(test)]
pub(crate) mod fixtures;
mod policy;
mod ports;
mod realm_name;
mod realms;
mod service;

pub use cell::{Cell, CellId, CellStatus, HighAvailability, choose};
pub use errors::{CellError, PlacementError, RealmError};
pub use policy::CellPolicy;
pub use ports::CellRepository;
#[cfg(test)]
pub use ports::MockCellRepository;
pub use realm_name::RealmName;
pub use realms::{
    HAND_OFF_FAILED, PooledOutcomes, REALM_CREATE, REALM_DELETE, RealmRequests, Settlement,
};
pub use service::{
    CELL_API_REPLICA, CELL_DATABASE_INSTANCE, CELL_WEBAPP_REPLICA, CellServiceImpl,
    CreateCellCommand, cell_reservation, release_for_cells,
};
