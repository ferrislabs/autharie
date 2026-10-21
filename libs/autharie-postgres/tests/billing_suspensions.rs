use autharie_domain::{
    CoreError,
    billing::{
        BillingPeriod, BillingSuspension, BillingSuspensionId, Statement, StatementId,
        StatementParts, StatementStatus,
        ports::{BillingSuspensionRepository, StatementRepository},
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::billing::{
    PostgresBillingSuspensionRepository, PostgresStatementRepository,
};
use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

mod support;
use support::pool;

macro_rules! pool_or_skip {
    () => {
        match pool().await {
            Some(pool) => pool,
            None => {
                eprintln!("skipped: DATABASE_URL is not set");
                return;
            }
        }
    };
}

fn map_err(e: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: e.to_string(),
    }
}

fn at(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 12, day, hour, 0, 0).unwrap()
}

async fn scratch<T>(work: impl AsyncFnOnce(SharedTx<'_>) -> Result<T, CoreError>) -> Option<T> {
    let pool = pool().await?;

    Some(
        in_scratch_tx(&pool, map_err, work)
            .await
            .expect("the scratch transaction ran"),
    )
}

async fn a_statement(
    tx: &SharedTx<'_>,
    organisation: OrganisationId,
    month: u32,
) -> Result<StatementId, CoreError> {
    let statement = Statement::from(StatementParts {
        id: StatementId::new(),
        organisation_id: organisation,
        period: BillingPeriod::new(2026, month).unwrap(),
        lines: Vec::new(),
        status: StatementStatus::Held,
        invoice: None,
        created_at: at(1, 0),
        updated_at: at(1, 0),
    });
    PostgresStatementRepository::new(tx)
        .insert(&statement)
        .await?;

    Ok(statement.id())
}

fn suspension(organisation: OrganisationId, statement: StatementId) -> BillingSuspension {
    BillingSuspension {
        id: BillingSuspensionId::new(),
        organisation_id: organisation,
        statement_id: statement,
        stopped: vec![
            DeploymentId(Uuid::new_v4()),
            DeploymentId(Uuid::new_v4()),
            DeploymentId(Uuid::new_v4()),
        ],
        suspended_organisation: true,
        suspended_at: at(20, 6),
        lifted_at: None,
    }
}

async fn lifted_at(
    tx: &SharedTx<'_>,
    id: &BillingSuspensionId,
) -> Result<Option<DateTime<Utc>>, CoreError> {
    let mut guard = tx.lock().await;
    sqlx::query_scalar("SELECT lifted_at FROM billing_suspensions WHERE id = $1")
        .bind(id.0)
        .fetch_one(&mut ***guard)
        .await
        .map_err(map_err)
}

#[tokio::test]
async fn an_active_suspension_is_read_back_with_the_deployments_it_stopped_in_order() {
    let _pool = pool_or_skip!();

    let (found, suspension) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let statement = a_statement(&tx, organisation, 11).await?;
        let suspensions = PostgresBillingSuspensionRepository::new(&tx);
        let suspension = suspension(organisation, statement);
        suspensions.insert(&suspension).await?;

        Ok((suspensions.find_active(&organisation).await?, suspension))
    })
    .await
    .expect("the database is set");

    assert_eq!(found.as_ref(), Some(&suspension));
    assert!(suspension.is_active());
}

#[tokio::test]
async fn a_suspension_that_stopped_nothing_is_read_back() {
    let _pool = pool_or_skip!();

    let (found, suspension) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let statement = a_statement(&tx, organisation, 11).await?;
        let suspensions = PostgresBillingSuspensionRepository::new(&tx);
        let suspension = BillingSuspension {
            stopped: Vec::new(),
            suspended_organisation: false,
            ..suspension(organisation, statement)
        };
        suspensions.insert(&suspension).await?;

        Ok((suspensions.find_active(&organisation).await?, suspension))
    })
    .await
    .expect("the database is set");

    assert_eq!(found, Some(suspension));
}

#[tokio::test]
async fn an_organisation_that_was_never_suspended_has_no_active_suspension() {
    let _pool = pool_or_skip!();

    let found = scratch(async |tx| {
        PostgresBillingSuspensionRepository::new(&tx)
            .find_active(&OrganisationId(Uuid::new_v4()))
            .await
    })
    .await
    .expect("the database is set");

    assert_eq!(found, None);
}

#[tokio::test]
async fn lifting_ends_the_suspension_and_keeps_the_row() {
    let _pool = pool_or_skip!();

    let (active, lifted) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let statement = a_statement(&tx, organisation, 11).await?;
        let suspensions = PostgresBillingSuspensionRepository::new(&tx);
        let suspension = suspension(organisation, statement);
        suspensions.insert(&suspension).await?;

        suspensions.lift(&suspension.id, at(22, 9)).await?;

        Ok((
            suspensions.find_active(&organisation).await?,
            lifted_at(&tx, &suspension.id).await?,
        ))
    })
    .await
    .expect("the database is set");

    assert_eq!(active, None);
    assert_eq!(lifted, Some(at(22, 9)));
}

#[tokio::test]
async fn lifting_twice_keeps_the_first_instant() {
    let _pool = pool_or_skip!();

    let lifted = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let statement = a_statement(&tx, organisation, 11).await?;
        let suspensions = PostgresBillingSuspensionRepository::new(&tx);
        let suspension = suspension(organisation, statement);
        suspensions.insert(&suspension).await?;

        suspensions.lift(&suspension.id, at(22, 9)).await?;
        suspensions.lift(&suspension.id, at(23, 9)).await?;

        lifted_at(&tx, &suspension.id).await
    })
    .await
    .expect("the database is set");

    assert_eq!(lifted, Some(at(22, 9)));
}

async fn stopped_of(
    tx: &SharedTx<'_>,
    id: &BillingSuspensionId,
) -> Result<Vec<DeploymentId>, CoreError> {
    let stopped: Vec<Uuid> = {
        let mut guard = tx.lock().await;
        sqlx::query_scalar("SELECT stopped_deployments FROM billing_suspensions WHERE id = $1")
            .bind(id.0)
            .fetch_one(&mut ***guard)
            .await
            .map_err(map_err)?
    };

    Ok(stopped.into_iter().map(DeploymentId).collect())
}

#[tokio::test]
async fn every_active_suspension_is_listed_and_a_lifted_one_is_not() {
    let _pool = pool_or_skip!();

    let (listed, active, lifted) = scratch(async |tx| {
        let suspensions = PostgresBillingSuspensionRepository::new(&tx);
        let first = OrganisationId(Uuid::new_v4());
        let second = OrganisationId(Uuid::new_v4());
        let active = suspension(first, a_statement(&tx, first, 11).await?);
        let lifted = suspension(second, a_statement(&tx, second, 11).await?);
        suspensions.insert(&active).await?;
        suspensions.insert(&lifted).await?;
        suspensions.lift(&lifted.id, at(22, 9)).await?;

        Ok((suspensions.list_active().await?, active, lifted))
    })
    .await
    .expect("the database is set");

    assert!(listed.contains(&active));
    assert!(!listed.iter().any(|suspension| suspension.id == lifted.id));
}

#[tokio::test]
async fn deployments_stopped_later_are_appended_after_the_first_ones() {
    let _pool = pool_or_skip!();

    let (before, after) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let statement = a_statement(&tx, organisation, 11).await?;
        let suspensions = PostgresBillingSuspensionRepository::new(&tx);
        let suspension = suspension(organisation, statement);
        suspensions.insert(&suspension).await?;

        let later = [DeploymentId(Uuid::new_v4()), DeploymentId(Uuid::new_v4())];
        suspensions.add_stopped(&suspension.id, &later).await?;

        Ok((
            suspension.stopped.clone(),
            (stopped_of(&tx, &suspension.id).await?, later),
        ))
    })
    .await
    .expect("the database is set");

    let (stopped, later) = after;
    assert_eq!(stopped, [before.as_slice(), later.as_slice()].concat());
}

#[tokio::test]
async fn a_deployment_already_recorded_as_stopped_is_not_recorded_twice() {
    let _pool = pool_or_skip!();

    let (first, stopped, new) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let statement = a_statement(&tx, organisation, 11).await?;
        let suspensions = PostgresBillingSuspensionRepository::new(&tx);
        let suspension = suspension(organisation, statement);
        suspensions.insert(&suspension).await?;

        let new = DeploymentId(Uuid::new_v4());
        suspensions
            .add_stopped(&suspension.id, &[suspension.stopped[1], new, new])
            .await?;

        Ok((
            suspension.stopped.clone(),
            stopped_of(&tx, &suspension.id).await?,
            new,
        ))
    })
    .await
    .expect("the database is set");

    assert_eq!(stopped, [first.as_slice(), &[new]].concat());
}

#[tokio::test]
async fn a_lifted_suspension_takes_no_more_stopped_deployments() {
    let _pool = pool_or_skip!();

    let (before, after) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let statement = a_statement(&tx, organisation, 11).await?;
        let suspensions = PostgresBillingSuspensionRepository::new(&tx);
        let suspension = suspension(organisation, statement);
        suspensions.insert(&suspension).await?;
        suspensions.lift(&suspension.id, at(22, 9)).await?;

        suspensions
            .add_stopped(&suspension.id, &[DeploymentId(Uuid::new_v4())])
            .await?;

        Ok((
            suspension.stopped.clone(),
            stopped_of(&tx, &suspension.id).await?,
        ))
    })
    .await
    .expect("the database is set");

    assert_eq!(after, before);
}

#[tokio::test]
async fn an_organisation_has_one_active_suspension_at_a_time() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let november = a_statement(&tx, organisation, 11).await?;
        let october = a_statement(&tx, organisation, 10).await?;
        let suspensions = PostgresBillingSuspensionRepository::new(&tx);
        suspensions
            .insert(&suspension(organisation, november))
            .await?;

        Ok(suspensions.insert(&suspension(organisation, october)).await)
    })
    .await
    .expect("the database is set");

    assert!(
        matches!(outcome, Err(CoreError::DatabaseError { .. })),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn an_organisation_can_be_suspended_again_once_the_first_is_lifted() {
    let _pool = pool_or_skip!();

    let (active, second) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let november = a_statement(&tx, organisation, 11).await?;
        let october = a_statement(&tx, organisation, 10).await?;
        let suspensions = PostgresBillingSuspensionRepository::new(&tx);

        let first = suspension(organisation, november);
        suspensions.insert(&first).await?;
        suspensions.lift(&first.id, at(22, 9)).await?;

        let second = suspension(organisation, october);
        suspensions.insert(&second).await?;

        Ok((suspensions.find_active(&organisation).await?, second))
    })
    .await
    .expect("the database is set");

    assert_eq!(active, Some(second));
}

#[tokio::test]
async fn a_suspension_needs_its_statement() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        Ok(PostgresBillingSuspensionRepository::new(&tx)
            .insert(&suspension(
                OrganisationId(Uuid::new_v4()),
                StatementId::new(),
            ))
            .await)
    })
    .await
    .expect("the database is set");

    assert!(
        matches!(outcome, Err(CoreError::DatabaseError { .. })),
        "{outcome:?}"
    );
}
