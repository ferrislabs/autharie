use autharie_domain::{
    CoreError,
    organisation::{
        OrganisationId,
        commands::CreateOrganisationData,
        ports::OrganisationRepository,
        value_objects::{OrganisationLimits, OrganisationName, OrganisationSlug, Plan},
    },
    user::UserId,
};
use autharie_persistence::in_scratch_tx;
use autharie_postgres::organisation::PostgresOrganisationRepository;
use uuid::Uuid;

mod support;
use support::pool;

fn db_error(e: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: e.to_string(),
    }
}

async fn seed_owner(tx: &autharie_persistence::SharedTx<'_>) -> Result<UserId, CoreError> {
    let owner = UserId(Uuid::new_v4());
    let mut guard = tx.lock().await;
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'mcp-switch', $3)")
        .bind(owner.0)
        .bind(format!("{}@mcp-switch", owner.0))
        .bind(format!("mcp-switch-{}", owner.0))
        .execute(&mut ***guard)
        .await
        .map_err(db_error)?;
    Ok(owner)
}

fn data(owner: UserId) -> CreateOrganisationData {
    let slug = format!("mcp-{}", &Uuid::new_v4().simple().to_string()[..12]);
    CreateOrganisationData {
        name: OrganisationName::new("Switch Corp").unwrap(),
        slug: OrganisationSlug::new(slug).unwrap(),
        owner_id: owner,
        plan: Plan::Free,
        limits: OrganisationLimits::from_plan(&Plan::Free),
    }
}

#[tokio::test]
async fn a_new_organisation_has_the_switch_off() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<bool, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let organisations = PostgresOrganisationRepository::new(&tx);
        let owner = seed_owner(&tx).await?;
        let created = organisations.create(data(owner)).await?;
        let read = organisations
            .find_by_id(&created.id)
            .await?
            .expect("the organisation");
        Ok(read.mcp_enabled)
    })
    .await;

    assert!(!found.expect("the transaction"));
}

#[tokio::test]
async fn switching_it_on_survives_a_read_and_does_not_leak_to_others() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<(bool, bool, bool), CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let organisations = PostgresOrganisationRepository::new(&tx);
        let owner = seed_owner(&tx).await?;
        let mut first = organisations.create(data(owner)).await?;
        let second = organisations.create(data(owner)).await?;
        organisations.insert_member(&first.id, &owner).await?;

        first.set_mcp_enabled(true);
        organisations.update(first.clone()).await?;

        let by_id = organisations.find_by_id(&first.id).await?.unwrap();
        let by_member = organisations.find_by_member(&owner).await?;
        let listed = by_member
            .iter()
            .find(|org| org.id == first.id)
            .map(|org| org.mcp_enabled)
            .unwrap_or(false);
        let other: OrganisationId = second.id;
        let other_read = organisations.find_by_id(&other).await?.unwrap();

        Ok((by_id.mcp_enabled, listed, other_read.mcp_enabled))
    })
    .await;

    let (by_id, listed, other) = found.expect("the transaction");
    assert!(by_id);
    assert!(listed);
    assert!(!other);
}
