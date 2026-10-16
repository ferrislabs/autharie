use autharie_auth::Identity;
use autharie_core::{
    cells::{Cell, CellId, CellStatus, CreateCellCommand},
    dataplane::value_objects::Region,
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath)]
#[typed_path("/platform/cells")]
pub struct CellsRoute;

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/cells/{cell_id}")]
pub struct CellRoute {
    pub cell_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/cells/{cell_id}/drain")]
pub struct DrainCellRoute {
    pub cell_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/cells/{cell_id}/retire")]
pub struct RetireCellRoute {
    pub cell_id: Uuid,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateCellRequest {
    pub region: String,
    #[serde(default)]
    pub capacity: Option<u16>,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct CellView {
    pub id: Uuid,
    pub region: String,
    pub data_plane_id: Uuid,
    pub instance_deployment_id: Uuid,
    pub status: CellStatus,
    pub capacity: u16,
    pub realms: u16,
    pub created_at: DateTime<Utc>,
}

impl From<Cell> for CellView {
    fn from(cell: Cell) -> Self {
        Self {
            id: cell.id().0,
            region: cell.region().as_str().to_string(),
            data_plane_id: cell.data_plane_id().0,
            instance_deployment_id: cell.instance().0,
            status: cell.status(),
            capacity: cell.capacity(),
            realms: cell.realms(),
            created_at: cell.created_at(),
        }
    }
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct CellResponse {
    pub data: CellView,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct CellsResponse {
    pub data: Vec<CellView>,
}

impl CreateCellRequest {
    fn into_command(self) -> Result<CreateCellCommand, ApiError> {
        let region = self.region.trim();
        if region.is_empty() {
            return Err(ApiError::Unprocessable {
                reason: "a cell needs a region".to_string(),
            });
        }

        CreateCellCommand::new(Region::new(region.to_string()), self.capacity)
            .map_err(|error| ApiError::from(autharie_core::CoreError::from(error)))
    }
}

#[utoipa::path(
    post,
    path = "/cells",
    summary = "create a cell",
    tag = "platform",
    description = "Creates a highly available FerrisKey instance owned by the cells organisation, \
                   and a cell on it that opens once the instance serves two replicas on two \
                   database instances.",
    request_body = CreateCellRequest,
    responses(
        (status = 201, description = "The cell, provisioning", body = CellResponse),
                (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the operate_fleet right", body = ApiError),
        (status = 409, description = "No FerrisKey release is available, or no data plane has room", body = ApiError),
        (status = 422, description = "The request names no region, or a capacity outside what a cell holds", body = ApiError),
        (status = 503, description = "Cells are not configured on this installation", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_cell_handler(
    _: CellsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<CreateCellRequest>,
) -> Result<Response<CellResponse>, ApiError> {
    let cell = state
        .service
        .create_cell(identity, request.into_command()?)
        .await?;

    Ok(Response::Created(CellResponse { data: cell.into() }))
}

#[utoipa::path(
    get,
    path = "/cells",
    summary = "list the cells",
    tag = "platform",
    responses(
        (status = 200, description = "Every cell, with its status and how many realms it holds", body = CellsResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the view_estate right", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_cells_handler(
    _: CellsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<CellsResponse>, ApiError> {
    let cells = state.service.list_cells(identity).await?;

    Ok(Response::OK(CellsResponse {
        data: cells.into_iter().map(CellView::from).collect(),
    }))
}

#[utoipa::path(
    get,
    path = "/cells/{cell_id}",
    summary = "get a cell",
    tag = "platform",
    params(CellRoute),
    responses(
        (status = 200, description = "The cell", body = CellResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the view_estate right", body = ApiError),
        (status = 404, description = "No such cell", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_cell_handler(
    CellRoute { cell_id }: CellRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<CellResponse>, ApiError> {
    let cell = state.service.get_cell(identity, CellId(cell_id)).await?;

    Ok(Response::OK(CellResponse { data: cell.into() }))
}

#[utoipa::path(
    post,
    path = "/cells/{cell_id}/drain",
    summary = "drain a cell",
    tag = "platform",
    description = "An open or full cell stops receiving tenants. The tenants it holds stay.",
    params(DrainCellRoute),
    responses(
        (status = 200, description = "The cell, draining", body = CellResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the operate_fleet right", body = ApiError),
        (status = 404, description = "No such cell", body = ApiError),
        (status = 409, description = "The cell is not open or full", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn drain_cell_handler(
    DrainCellRoute { cell_id }: DrainCellRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<CellResponse>, ApiError> {
    let cell = state.service.drain_cell(identity, CellId(cell_id)).await?;

    Ok(Response::OK(CellResponse { data: cell.into() }))
}

#[utoipa::path(
    post,
    path = "/cells/{cell_id}/retire",
    summary = "retire a cell",
    tag = "platform",
    description = "A draining cell that holds no realm is retired.",
    params(RetireCellRoute),
    responses(
        (status = 200, description = "The cell, retired", body = CellResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the operate_fleet right", body = ApiError),
        (status = 404, description = "No such cell", body = ApiError),
        (status = 409, description = "The cell is not draining, or still holds realms", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn retire_cell_handler(
    RetireCellRoute { cell_id }: RetireCellRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<CellResponse>, ApiError> {
    let cell = state.service.retire_cell(identity, CellId(cell_id)).await?;

    Ok(Response::OK(CellResponse { data: cell.into() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_without_a_region_is_refused() {
        let refused = CreateCellRequest {
            region: "  ".to_string(),
            capacity: None,
        }
        .into_command();

        assert!(matches!(refused, Err(ApiError::Unprocessable { .. })));
    }

    #[test]
    fn a_capacity_above_two_hundred_is_unprocessable() {
        let refused = CreateCellRequest {
            region: "fr-par".to_string(),
            capacity: Some(201),
        }
        .into_command();

        assert!(matches!(refused, Err(ApiError::Unprocessable { .. })));
    }

    #[test]
    fn a_request_names_the_region_of_the_cell() {
        let command = CreateCellRequest {
            region: " fr-par ".to_string(),
            capacity: Some(20),
        }
        .into_command()
        .expect("a command");

        assert_eq!(command.region().as_str(), "fr-par");
        assert_eq!(command.policy().capacity(), 20);
    }
}
