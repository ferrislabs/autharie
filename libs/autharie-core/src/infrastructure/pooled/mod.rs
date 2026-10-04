macro_rules! in_tx {
    ($pool:expr, $repository:ty, $method:ident($($argument:expr),*)) => {
        ::autharie_persistence::with_tx(
            $pool,
            ::autharie_postgres::map_sqlx_error,
            async |tx| <$repository>::new(&tx).$method($($argument),*).await,
        )
        .await
    };
}

mod credentials;
mod data_planes;
mod inventory;
mod queue;

pub use credentials::PooledCredentialStore;
pub use data_planes::PooledDataPlanes;
pub use inventory::PooledInventory;
pub use queue::PostgresClusterQueue;
