mod cell;
mod errors;
#[cfg(test)]
pub(crate) mod fixtures;
mod policy;
mod ports;
mod realm_name;
mod service;

pub use cell::{Cell, CellId, CellStatus, HighAvailability, choose};
pub use errors::{CellError, PlacementError, RealmError};
pub use policy::CellPolicy;
#[cfg(test)]
pub use ports::MockCellRepository;
pub use ports::{CellCredentialStore, CellRepository, RealmAdmin};
pub use realm_name::RealmName;
pub use service::{
    CELL_API_REPLICA, CELL_DATABASE_INSTANCE, CELL_WEBAPP_REPLICA, CellServiceImpl,
    CreateCellCommand, cell_reservation, release_for_cells,
};
