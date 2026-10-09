use autharie_domain::{
    CoreError,
    cells::{Cell, CellError, CellId, CellRepository, CellStatus},
    dataplane::value_objects::{DataPlaneId, Region},
    deployments::DeploymentId,
};
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};
use uuid::Uuid;

struct CellRow {
    id: Uuid,
    region: String,
    data_plane_id: Uuid,
    instance_deployment_id: Uuid,
    status: String,
    capacity: i32,
    realms: i64,
    created_at: DateTime<Utc>,
}

impl CellRow {
    fn into_cell(self) -> Result<Cell, CoreError> {
        let id = self.id;
        let stored = |what: &str| {
            CoreError::InternalError(format!("cell {id} has {what} that cannot be read"))
        };
        let capacity = u16::try_from(self.capacity).map_err(|_| stored("a capacity"))?;
        let realms = u16::try_from(self.realms).map_err(|_| stored("a realm count"))?;
        let status = match parse_status(&self.status)? {
            CellStatus::Open if realms >= capacity => CellStatus::Full,
            status => status,
        };

        Cell::restore(
            CellId(self.id),
            Region::new(self.region),
            DataPlaneId(self.data_plane_id),
            DeploymentId(self.instance_deployment_id),
            status,
            capacity,
            realms,
            self.created_at,
        )
        .map_err(CoreError::from)
    }
}

fn parse_status(raw: &str) -> Result<CellStatus, CoreError> {
    match raw {
        "provisioning" => Ok(CellStatus::Provisioning),
        "open" => Ok(CellStatus::Open),
        "draining" => Ok(CellStatus::Draining),
        "retired" => Ok(CellStatus::Retired),
        "failed" => Ok(CellStatus::Failed),
        other => Err(CoreError::InternalError(format!(
            "unknown cell status '{other}'"
        ))),
    }
}

fn status_to_row(status: CellStatus) -> Result<&'static str, CoreError> {
    match status {
        CellStatus::Provisioning => Ok("provisioning"),
        CellStatus::Open => Ok("open"),
        CellStatus::Draining => Ok("draining"),
        CellStatus::Retired => Ok("retired"),
        CellStatus::Failed => Ok("failed"),
        CellStatus::Full => Err(CellError::InvalidTransition.into()),
    }
}

fn database_error(action: &'static str) -> impl FnOnce(sqlx::Error) -> CoreError {
    move |error| CoreError::DatabaseError {
        message: format!("Failed to {action}: {error}"),
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
pub struct PostgresCellRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresCellRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl CellRepository for PostgresCellRepository<'_> {
    async fn create(&self, cell: &Cell) -> Result<(), CoreError> {
        if cell.status() != CellStatus::Provisioning {
            return Err(CellError::InvalidTransition.into());
        }

        let mut tx = self.tx.lock().await;
        sqlx::query!(
            r#"
            INSERT INTO cells (
                id, region, data_plane_id, instance_deployment_id, status, capacity, created_at
            )
            VALUES ($1, $2, $3, $4, 'provisioning', $5, $6)
            "#,
            cell.id().0,
            cell.region().as_str(),
            cell.data_plane_id().0,
            cell.instance().0,
            i32::from(cell.capacity()),
            cell.created_at(),
        )
        .execute(&mut ***tx)
        .await
        .map_err(database_error("create a cell"))?;

        Ok(())
    }

    async fn find(&self, id: CellId) -> Result<Option<Cell>, CoreError> {
        let mut tx = self.tx.lock().await;
        let row = sqlx::query_as!(
            CellRow,
            r#"
            SELECT c.id,
                   c.region,
                   c.data_plane_id,
                   c.instance_deployment_id,
                   c.status,
                   c.capacity,
                   (SELECT count(*) FROM deployments d
                    WHERE d.cell_id = c.id AND d.cell_slot_held) AS "realms!",
                   c.created_at
            FROM cells c
            WHERE c.id = $1
            "#,
            id.0
        )
        .fetch_optional(&mut ***tx)
        .await
        .map_err(database_error("find a cell"))?;

        row.map(CellRow::into_cell).transpose()
    }

    async fn list(&self) -> Result<Vec<Cell>, CoreError> {
        let mut tx = self.tx.lock().await;
        let rows = sqlx::query_as!(
            CellRow,
            r#"
            SELECT c.id,
                   c.region,
                   c.data_plane_id,
                   c.instance_deployment_id,
                   c.status,
                   c.capacity,
                   (SELECT count(*) FROM deployments d
                    WHERE d.cell_id = c.id AND d.cell_slot_held) AS "realms!",
                   c.created_at
            FROM cells c
            ORDER BY c.created_at, c.id
            "#
        )
        .fetch_all(&mut ***tx)
        .await
        .map_err(database_error("list cells"))?;

        rows.into_iter().map(CellRow::into_cell).collect()
    }

    async fn set_status(&self, id: CellId, status: CellStatus) -> Result<(), CoreError> {
        let status = status_to_row(status)?;

        let mut tx = self.tx.lock().await;
        let affected = sqlx::query!(
            r#"
            UPDATE cells
            SET status = $2
            WHERE id = $1
            "#,
            id.0,
            status
        )
        .execute(&mut ***tx)
        .await
        .map_err(database_error("set the status of a cell"))?
        .rows_affected();

        if affected == 0 {
            return Err(CellError::UnknownCell { id: id.0 }.into());
        }

        Ok(())
    }

    async fn place(&self, region: &Region) -> Result<Option<Cell>, CoreError> {
        let mut tx = self.tx.lock().await;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(format!("cells:{}", region.as_str()))
            .execute(&mut ***tx)
            .await
            .map_err(database_error("serialise the placements of a region"))?;

        let candidates = sqlx::query_scalar!(
            r#"
            SELECT c.id AS "id!"
            FROM (
                SELECT cells.id,
                       cells.capacity,
                       cells.created_at,
                       (SELECT count(*) FROM deployments d
                        WHERE d.cell_id = cells.id AND d.cell_slot_held) AS realms
                FROM cells
                WHERE cells.region = $1 AND cells.status = 'open'
            ) c
            WHERE c.realms < c.capacity
            ORDER BY c.realms DESC, c.created_at, c.id
            "#,
            region.as_str()
        )
        .fetch_all(&mut ***tx)
        .await
        .map_err(database_error("list the cells that can take a tenant"))?;

        for id in candidates {
            let locked = sqlx::query_scalar!(
                r#"
                SELECT id
                FROM cells
                WHERE id = $1 AND status = 'open'
                FOR UPDATE
                "#,
                id
            )
            .fetch_optional(&mut ***tx)
            .await
            .map_err(database_error("lock a cell"))?;

            if locked.is_none() {
                continue;
            }

            let row = sqlx::query_as!(
                CellRow,
                r#"
                SELECT c.id,
                       c.region,
                       c.data_plane_id,
                       c.instance_deployment_id,
                       c.status,
                       c.capacity,
                       (SELECT count(*) FROM deployments d
                        WHERE d.cell_id = c.id AND d.cell_slot_held) AS "realms!",
                       c.created_at
                FROM cells c
                WHERE c.id = $1
                "#,
                id
            )
            .fetch_one(&mut ***tx)
            .await
            .map_err(database_error("recount a locked cell"))?;

            if row.realms < i64::from(row.capacity) {
                return row.into_cell().map(Some);
            }
        }

        Ok(None)
    }

    async fn release(&self, deployment: DeploymentId) -> Result<bool, CoreError> {
        let mut tx = self.tx.lock().await;
        let affected = sqlx::query!(
            r#"
            UPDATE deployments
            SET cell_slot_held = false
            WHERE id = $1 AND cell_slot_held
            "#,
            deployment.0
        )
        .execute(&mut ***tx)
        .await
        .map_err(database_error("release a cell slot"))?
        .rows_affected();

        Ok(affected > 0)
    }
}
