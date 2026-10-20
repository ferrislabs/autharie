use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CellError {
    #[error("a cell needs a capacity above zero and fewer spare slots than its capacity")]
    CapacityInvalid,

    #[error("the cell is full")]
    Full,

    #[error("the cell does not take new tenants")]
    NotOpen,

    #[error("the stored cell is inconsistent: {reason}")]
    Inconsistent { reason: String },

    #[error("that status change is not allowed for this cell")]
    InvalidTransition,

    #[error("cell not found with id: {id}")]
    UnknownCell { id: Uuid },

    #[error(
        "cells are not configured on this installation: choose the organisation that owns them"
    )]
    NotConfigured,

    #[error(
        "the organisation that owns the cells cannot change while {active} cell(s) are not retired: retire them first"
    )]
    OwnerInUse { active: usize },

    #[error("no FerrisKey release recent enough is available to build a cell: publish one first")]
    NoRelease,

    #[error("this deployment is the instance of cell {cell}: retire the cell first")]
    InstanceInUse { cell: Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlacementError {
    #[error("no cell in region '{region}' has room for a new tenant, try again later")]
    NoOpenCell { region: String },

    #[error("free deployments are not available on this installation: it has no DNS zone for them")]
    NoTenantZone,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RealmError {
    #[error("{reason}")]
    InvalidName { reason: String },

    #[error("the realm already exists in this cell")]
    AlreadyExists,

    #[error("the cell could not be reached")]
    CellUnreachable,

    #[error("the cell refused the realm: {reason}")]
    Refused { reason: String },
}
