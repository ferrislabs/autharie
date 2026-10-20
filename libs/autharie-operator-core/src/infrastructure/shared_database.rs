use std::collections::BTreeMap;
use std::time::Duration;

use autharie_crds::v1alpha::identity_instance::SharedClusterConfig;
use k8s_openapi::api::core::v1::Secret;
use kube::{Api, Client};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

use crate::domain::OperatorError;
use crate::domain::shared_database::{SharedDatabaseError, SharedDatabaseName};

pub const CLUSTER_ENV: &str = "AUTHARIE_SHARED_DATABASE_CLUSTER";
pub const NAMESPACE_ENV: &str = "AUTHARIE_SHARED_DATABASE_NAMESPACE";

/// The most connections one free deployment holds on the shared cluster.
///
/// FerrisKey's pool is fixed in its code at 5 to 100 connections, so without
/// a cap one tenant under load takes a third of a 300-connection cluster and
/// the others fail to connect. 10 covers its idle 5 with room for a burst; a
/// burst past that fails that tenant's requests, not everyone's.
pub const ROLE_CONNECTION_LIMIT: u32 = 10;

const POSTGRES_PORT: u16 = 5432;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DUPLICATE_OBJECT: &str = "42710";
const DUPLICATE_DATABASE: &str = "42P04";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedCluster {
    pub name: String,
    pub namespace: String,
}

impl SharedCluster {
    pub fn from_env() -> Option<Self> {
        Self::from_values(
            std::env::var(CLUSTER_ENV).ok().as_deref(),
            std::env::var(NAMESPACE_ENV).ok().as_deref(),
        )
    }

    fn from_values(name: Option<&str>, namespace: Option<&str>) -> Option<Self> {
        let clean = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        };

        Some(Self {
            name: clean(name)?,
            namespace: clean(namespace)?,
        })
    }

    pub fn resolve(
        spec: Option<&SharedClusterConfig>,
        data_plane: Option<&Self>,
    ) -> Result<Self, SharedDatabaseError> {
        let name = spec
            .and_then(|shared| shared.cluster_name.clone())
            .or_else(|| data_plane.map(|cluster| cluster.name.clone()));
        let namespace = spec
            .and_then(|shared| shared.cluster_namespace.clone())
            .or_else(|| data_plane.map(|cluster| cluster.namespace.clone()));

        match (name, namespace) {
            (Some(name), Some(namespace)) => Ok(Self { name, namespace }),
            _ => Err(SharedDatabaseError::Refused(format!(
                "the instance names no shared database cluster and the operator was given none ({CLUSTER_ENV}, {NAMESPACE_ENV})"
            ))),
        }
    }

    pub fn host(&self) -> String {
        format!("{}-rw.{}.svc.cluster.local", self.name, self.namespace)
    }

    pub fn superuser_secret(&self) -> String {
        format!("{}-superuser", self.name)
    }
}

pub fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn create_role_sql(name: &SharedDatabaseName, password: &str) -> String {
    format!(
        "CREATE ROLE {} LOGIN PASSWORD {} CONNECTION LIMIT {ROLE_CONNECTION_LIMIT}",
        quote_identifier(name.as_str()),
        quote_literal(password),
    )
}

fn alter_role_sql(name: &SharedDatabaseName, password: &str) -> String {
    format!(
        "ALTER ROLE {} LOGIN PASSWORD {} CONNECTION LIMIT {ROLE_CONNECTION_LIMIT}",
        quote_identifier(name.as_str()),
        quote_literal(password),
    )
}

fn create_database_sql(name: &SharedDatabaseName) -> String {
    format!(
        "CREATE DATABASE {} OWNER {}",
        quote_identifier(name.as_str()),
        quote_identifier(name.as_str()),
    )
}

fn revoke_public_sql(name: &SharedDatabaseName) -> String {
    format!(
        "REVOKE CONNECT ON DATABASE {} FROM PUBLIC",
        quote_identifier(name.as_str())
    )
}

fn grant_connect_sql(name: &SharedDatabaseName) -> String {
    format!(
        "GRANT CONNECT ON DATABASE {} TO {}",
        quote_identifier(name.as_str()),
        quote_identifier(name.as_str()),
    )
}

fn drop_database_sql(name: &SharedDatabaseName) -> String {
    format!(
        "DROP DATABASE IF EXISTS {} WITH (FORCE)",
        quote_identifier(name.as_str())
    )
}

fn drop_role_sql(name: &SharedDatabaseName) -> String {
    format!("DROP ROLE IF EXISTS {}", quote_identifier(name.as_str()))
}

fn classify(error: sqlx::Error) -> SharedDatabaseError {
    match error {
        sqlx::Error::Database(error) => SharedDatabaseError::Refused(error.message().to_string()),
        sqlx::Error::Io(_)
        | sqlx::Error::Tls(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed => SharedDatabaseError::Unreachable,
        other => SharedDatabaseError::Refused(other.to_string()),
    }
}

fn has_code(error: &sqlx::Error, code: &str) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|found| found == code)
}

async fn exists(pool: &PgPool, sql: &str, name: &SharedDatabaseName) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(sql)
        .bind(name.as_str())
        .fetch_optional(pool)
        .await?
        .is_some())
}

pub async fn provision(
    pool: &PgPool,
    name: &SharedDatabaseName,
    password: &str,
) -> Result<(), SharedDatabaseError> {
    provision_inner(pool, name, password)
        .await
        .map_err(classify)
}

// ALTER on the existing-role path rather than leaving the role as it is: the
// tenant Secret is the source of truth for the password, so a role that
// drifted -- recreated by hand, restored from a dump -- converges back to it,
// and a cap changed in this file reaches roles created before the change.
//
// CREATE DATABASE has no IF NOT EXISTS and cannot run in a transaction, so
// existence is checked first and a concurrent reconcile winning the race is
// read as success rather than an error.
async fn provision_inner(
    pool: &PgPool,
    name: &SharedDatabaseName,
    password: &str,
) -> Result<(), sqlx::Error> {
    let role_exists = exists(pool, "SELECT 1 FROM pg_roles WHERE rolname = $1", name).await?;
    let converge = alter_role_sql(name, password);
    if role_exists {
        sqlx::query(&converge).execute(pool).await?;
    } else if let Err(error) = sqlx::query(&create_role_sql(name, password))
        .execute(pool)
        .await
    {
        if !has_code(&error, DUPLICATE_OBJECT) {
            return Err(error);
        }
        sqlx::query(&converge).execute(pool).await?;
    }

    let database_exists =
        exists(pool, "SELECT 1 FROM pg_database WHERE datname = $1", name).await?;
    if !database_exists
        && let Err(error) = sqlx::query(&create_database_sql(name)).execute(pool).await
        && !has_code(&error, DUPLICATE_DATABASE)
    {
        return Err(error);
    }

    // Every role may connect to every database by default. Revoking it from
    // PUBLIC is what keeps one tenant's role out of another tenant's data.
    sqlx::query(&revoke_public_sql(name)).execute(pool).await?;
    sqlx::query(&grant_connect_sql(name)).execute(pool).await?;
    Ok(())
}

/// Drops one tenant's database and role, and nothing else on the cluster.
///
/// WITH (FORCE) because the tenant's FerrisKey may still hold connections
/// while its namespace is being torn down; without it the drop fails and the
/// finalizer never completes.
pub async fn drop_database(
    pool: &PgPool,
    name: &SharedDatabaseName,
) -> Result<(), SharedDatabaseError> {
    sqlx::query(&drop_database_sql(name))
        .execute(pool)
        .await
        .map_err(classify)?;
    sqlx::query(&drop_role_sql(name))
        .execute(pool)
        .await
        .map_err(classify)?;
    Ok(())
}

pub fn tenant_secret_data(
    cluster: &SharedCluster,
    name: &SharedDatabaseName,
    password: &str,
) -> BTreeMap<String, String> {
    let host = cluster.host();
    let database = name.as_str();
    let uri = format!("postgres://{database}:{password}@{host}:{POSTGRES_PORT}/{database}");

    BTreeMap::from([
        ("host".to_string(), host),
        ("port".to_string(), POSTGRES_PORT.to_string()),
        ("database".to_string(), database.to_string()),
        ("user".to_string(), database.to_string()),
        ("password".to_string(), password.to_string()),
        ("uri".to_string(), uri.clone()),
        ("database-url".to_string(), uri),
    ])
}

fn secret_text(secret: &Secret, key: &str) -> Option<String> {
    secret
        .data
        .as_ref()
        .and_then(|data| data.get(key))
        .and_then(|value| String::from_utf8(value.0.clone()).ok())
}

pub async fn connect(client: &Client, cluster: &SharedCluster) -> Result<PgPool, OperatorError> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &cluster.namespace);
    let secret_name = cluster.superuser_secret();
    let secret = secrets
        .get_opt(&secret_name)
        .await
        .map_err(|error| OperatorError::Kube {
            message: error.to_string(),
        })?
        .ok_or_else(|| {
            SharedDatabaseError::Refused(format!(
                "secret `{secret_name}` is not in namespace `{}`",
                cluster.namespace
            ))
        })?;

    let (Some(username), Some(password)) = (
        secret_text(&secret, "username"),
        secret_text(&secret, "password"),
    ) else {
        return Err(SharedDatabaseError::Refused(format!(
            "secret `{secret_name}` has no `username` and `password`"
        ))
        .into());
    };

    let options = PgConnectOptions::new()
        .host(&cluster.host())
        .port(POSTGRES_PORT)
        .username(&username)
        .password(&password)
        .database("postgres");

    Ok(PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(CONNECT_TIMEOUT)
        .connect_lazy_with(options))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn name() -> SharedDatabaseName {
        SharedDatabaseName::for_deployment(
            Uuid::parse_str("b6a1c2d3-e4f5-4a6b-8c9d-0e1f2a3b4c5d").unwrap(),
        )
    }

    #[test]
    fn identifiers_are_double_quoted_and_inner_quotes_doubled() {
        assert_eq!(quote_identifier("free_ab"), "\"free_ab\"");
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn literals_are_single_quoted_and_inner_quotes_doubled() {
        assert_eq!(quote_literal("abc"), "'abc'");
        assert_eq!(
            quote_literal("a'; DROP ROLE x; --"),
            "'a''; DROP ROLE x; --'"
        );
    }

    #[test]
    fn the_role_is_created_with_a_connection_limit() {
        assert_eq!(
            create_role_sql(&name(), "pw"),
            "CREATE ROLE \"free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d\" LOGIN PASSWORD 'pw' CONNECTION LIMIT 10"
        );
        assert_eq!(
            alter_role_sql(&name(), "pw"),
            "ALTER ROLE \"free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d\" LOGIN PASSWORD 'pw' CONNECTION LIMIT 10"
        );
    }

    #[test]
    fn the_database_is_owned_by_the_role_and_closed_to_public() {
        assert_eq!(
            create_database_sql(&name()),
            "CREATE DATABASE \"free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d\" OWNER \"free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d\""
        );
        assert_eq!(
            revoke_public_sql(&name()),
            "REVOKE CONNECT ON DATABASE \"free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d\" FROM PUBLIC"
        );
        assert_eq!(
            grant_connect_sql(&name()),
            "GRANT CONNECT ON DATABASE \"free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d\" TO \"free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d\""
        );
    }

    #[test]
    fn dropping_forces_the_database_and_then_the_role() {
        assert_eq!(
            drop_database_sql(&name()),
            "DROP DATABASE IF EXISTS \"free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d\" WITH (FORCE)"
        );
        assert_eq!(
            drop_role_sql(&name()),
            "DROP ROLE IF EXISTS \"free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d\""
        );
    }

    #[test]
    fn the_cluster_from_env_needs_both_values() {
        assert_eq!(SharedCluster::from_values(Some("db"), None), None);
        assert_eq!(SharedCluster::from_values(Some(" "), Some("ns")), None);
        assert_eq!(
            SharedCluster::from_values(Some("db"), Some("ns")),
            Some(SharedCluster {
                name: "db".to_string(),
                namespace: "ns".to_string()
            })
        );
    }

    #[test]
    fn the_spec_overrides_the_operator_and_either_fills_the_other() {
        let data_plane = SharedCluster {
            name: "free-db".to_string(),
            namespace: "autharie-system".to_string(),
        };

        assert_eq!(
            SharedCluster::resolve(None, Some(&data_plane)),
            Ok(data_plane.clone())
        );

        let spec = SharedClusterConfig {
            cluster_name: Some("other".to_string()),
            cluster_namespace: None,
        };
        assert_eq!(
            SharedCluster::resolve(Some(&spec), Some(&data_plane)),
            Ok(SharedCluster {
                name: "other".to_string(),
                namespace: "autharie-system".to_string()
            })
        );
    }

    #[test]
    fn no_coordinates_anywhere_is_refused() {
        assert!(matches!(
            SharedCluster::resolve(None, None),
            Err(SharedDatabaseError::Refused(_))
        ));
    }

    #[test]
    fn the_cluster_is_reached_through_its_rw_service() {
        let cluster = SharedCluster {
            name: "free-db".to_string(),
            namespace: "autharie-system".to_string(),
        };

        assert_eq!(
            cluster.host(),
            "free-db-rw.autharie-system.svc.cluster.local"
        );
        assert_eq!(cluster.superuser_secret(), "free-db-superuser");
    }

    #[test]
    fn the_tenant_secret_carries_every_key_ferriskey_reads() {
        let cluster = SharedCluster {
            name: "free-db".to_string(),
            namespace: "autharie-system".to_string(),
        };
        let data = tenant_secret_data(&cluster, &name(), "pw");

        for key in [
            "host",
            "port",
            "database",
            "user",
            "password",
            "uri",
            "database-url",
        ] {
            assert!(data.contains_key(key), "missing {key}");
        }
        assert_eq!(data["user"], "free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d");
        assert_eq!(data["database"], data["user"]);
        assert_eq!(
            data["database-url"],
            "postgres://free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d:pw@free-db-rw.autharie-system.svc.cluster.local:5432/free_b6a1c2d3e4f54a6b8c9d0e1f2a3b4c5d"
        );
    }
}
