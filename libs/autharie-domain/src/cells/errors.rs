use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CellError {
    #[error("a cell needs a capacity above zero and fewer spare slots than its capacity")]
    CapacityInvalid,

    #[error("the cell is full")]
    Full,

    #[error("the cell does not take new tenants")]
    NotOpen,

    #[error("cell not found with id: {id}")]
    UnknownCell { id: Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlacementError {
    #[error("no cell in region '{region}' has room for a new tenant, try again later")]
    NoOpenCell { region: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RealmError {
    #[error("the realm already exists in this cell")]
    AlreadyExists,

    #[error("the cell could not be reached")]
    CellUnreachable,

    #[error("the cell refused the realm: {reason}")]
    Refused { reason: String },
}
